use crate::models::*;
use crate::repository::network_url;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::{broadcast, mpsc, oneshot, Mutex, RwLock},
};

pub struct Player {
    directory: PathBuf,
    session: Mutex<Option<ActiveSession>>,
    state: Arc<RwLock<PlayerState>>,
    events_tx: broadcast::Sender<PlayerEvent>,
    current_watch: Arc<Mutex<Option<(WatchEntry, String)>>>,
    req_counter: Arc<AtomicU64>,
}

struct ActiveSession {
    session_id: String,
    #[allow(dead_code)]
    profile_id: String,
    child: Option<Child>,
    socket: PathBuf,
    command_tx: mpsc::Sender<(Value, oneshot::Sender<Result<Value>>)>,
}

impl Player {
    pub fn new(directory: PathBuf) -> Self {
        let (events_tx, _) = broadcast::channel(128);
        Self {
            directory,
            session: Mutex::new(None),
            state: Arc::new(RwLock::new(PlayerState::default())),
            events_tx,
            current_watch: Arc::new(Mutex::new(None)),
            req_counter: Arc::new(AtomicU64::new(100)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.events_tx.subscribe()
    }

    pub async fn state(&self) -> PlayerState {
        self.state.read().await.clone()
    }

    pub async fn current_watch(&self) -> Option<(WatchEntry, String)> {
        self.current_watch.lock().await.clone()
    }

    pub async fn progress(&self) -> Option<WatchEntry> {
        self.current_watch.lock().await.as_ref().map(|(w, _)| w.clone())
    }

    pub async fn play(
        &self,
        stream: &Stream,
        subtitles: &[Subtitle],
        watch: Option<WatchEntry>,
        resume: f64,
        profile_id: Option<&str>,
    ) -> Result<String> {
        network_url(&stream.url)?;
        if !resume.is_finite() || resume < 0.0 {
            bail!("Invalid resume position")
        }

        self.stop().await;

        let session_id = uuid::Uuid::new_v4().to_string();
        let prof_id = profile_id.unwrap_or("default").to_string();

        let private_directory = self
            .directory
            .join(format!("cloudstream-{}", uuid::Uuid::new_v4()));
        let socket = private_directory.join("mpv.sock");
        let mut command = Command::new("mpv");
        command
            .args([
                "--no-config",
                "--idle=yes",
                "--keep-open=yes",
                "--force-window=yes",
                "--terminal=no",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg(format!("--start={resume}"))
            .arg(format!(
                "--title={}",
                watch
                    .as_ref()
                    .map(|w| w.item.name.as_str())
                    .unwrap_or("Cloudstream Desktop")
            ));
        let mut headers = stream.headers.clone();
        if !stream.referer.is_empty() && !headers.keys().any(|k| k.eq_ignore_ascii_case("referer"))
        {
            headers.insert("Referer".into(), stream.referer.clone());
        }
        for (name, value) in headers {
            if !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || value.contains(['\r', '\n'])
            {
                bail!("Extension returned an invalid HTTP header")
            }
            command.arg(format!("--http-header-fields-append={name}: {value}"));
        }
        for sub in subtitles.iter().filter(|s| s.headers.is_empty()) {
            network_url(&sub.url)?;
            command.arg(format!("--sub-file={}", sub.url));
        }
        command
            .arg("--")
            .arg(&stream.url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&private_directory)?;

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                return Err(error).context("mpv is unavailable. Install mpv to enable playback");
            }
        };

        let mut started = false;
        for _ in 0..50 {
            if socket.exists() {
                started = true;
                break;
            }
            if let Ok(Some(_)) = child.try_wait() {
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                bail!("mpv exited before playback started");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        if !started {
            let _ = child.kill().await;
            cleanup_socket(&socket);
            *self.state.write().await = PlayerState::default();
            bail!("mpv did not start its playback connection");
        }

        let stream_conn = match UnixStream::connect(&socket).await {
            Ok(s) => s,
            Err(err) => {
                let _ = child.kill().await;
                cleanup_socket(&socket);
                *self.state.write().await = PlayerState::default();
                return Err(err).context("Could not connect to mpv IPC socket");
            }
        };

        self.start_session(
            session_id.clone(),
            prof_id,
            Some(child),
            socket,
            stream_conn,
            watch,
            resume,
        )
        .await;

        Ok(session_id)
    }

    pub async fn start_session(
        &self,
        session_id: String,
        profile_id: String,
        child: Option<Child>,
        socket: PathBuf,
        stream: UnixStream,
        watch: Option<WatchEntry>,
        resume: f64,
    ) {
        let (command_tx, command_rx) =
            mpsc::channel::<(Value, oneshot::Sender<Result<Value>>)>(32);
        let pending_commands =
            Arc::new(Mutex::new(HashMap::<u64, oneshot::Sender<Result<Value>>>::new()));
        let (read_half, mut write_half) = stream.into_split();

        let initial_state = PlayerState {
            session_id: Some(session_id.clone()),
            active: true,
            playing: true,
            paused: false,
            position: resume,
            duration: 0.0,
            volume: 100.0,
            speed: 1.0,
            item_name: watch
                .as_ref()
                .map(|w| w.item.name.clone())
                .or_else(|| Some("Direct stream".into())),
            episode_name: watch.as_ref().map(|w| w.episode.name.clone()),
            media_id: watch.as_ref().map(|w| {
                w.item
                    .media_id
                    .clone()
                    .unwrap_or_else(|| compute_media_id(&w.item.provider_id, &w.item.url, w.item.unique_url.as_deref()))
            }),
            episode_id: watch.as_ref().map(|w| w.id.clone()),
            profile_id: Some(profile_id.clone()),
            end_reason: None,
        };
        *self.state.write().await = initial_state.clone();
        if let Some(w) = watch {
            *self.current_watch.lock().await = Some((w, profile_id.clone()));
        } else {
            *self.current_watch.lock().await = None;
        }

        let session = ActiveSession {
            session_id: session_id.clone(),
            profile_id,
            child,
            socket: socket.clone(),
            command_tx,
        };
        *self.session.lock().await = Some(session);

        let _ = self
            .events_tx
            .send(PlayerEvent::StateChanged { state: initial_state });

        let pending_for_writer = pending_commands.clone();
        let counter_for_writer = self.req_counter.clone();
        tokio::spawn(async move {
            let observe = [
                (1, "time-pos"),
                (2, "duration"),
                (3, "pause"),
                (4, "volume"),
                (5, "speed"),
                (6, "track-list"),
            ];
            for (id, prop) in observe {
                let msg = json!({ "command": ["observe_property", id, prop], "request_id": id })
                    .to_string()
                    + "\n";
                if write_half.write_all(msg.as_bytes()).await.is_err() {
                    return;
                }
            }

            let mut rx = command_rx;
            while let Some((cmd, reply)) = rx.recv().await {
                let req_id = counter_for_writer.fetch_add(1, Ordering::SeqCst);
                pending_for_writer.lock().await.insert(req_id, reply);
                let msg = json!({ "command": cmd, "request_id": req_id }).to_string() + "\n";
                if write_half.write_all(msg.as_bytes()).await.is_err() {
                    if let Some(tx) = pending_for_writer.lock().await.remove(&req_id) {
                        let _ = tx.send(Err(anyhow::anyhow!("Player disconnected")));
                    }
                    break;
                }
            }
        });

        let state_for_reader = self.state.clone();
        let events_tx_for_reader = self.events_tx.clone();
        let watch_for_reader = self.current_watch.clone();
        let pending_for_reader = pending_commands.clone();
        let sid = session_id.clone();
        let sock_path = socket.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let Ok(val) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };

                if let Some(req_id) = val.get("request_id").and_then(Value::as_u64) {
                    if req_id > 6 {
                        if let Some(tx) = pending_for_reader.lock().await.remove(&req_id) {
                            let err_str =
                                val.get("error").and_then(Value::as_str).unwrap_or("unknown");
                            if err_str == "success" {
                                let data = val.get("data").cloned().unwrap_or(Value::Null);
                                let _ = tx.send(Ok(data));
                            } else {
                                let _ = tx.send(Err(anyhow::anyhow!("Player error: {err_str}")));
                            }
                        }
                    }
                }

                if let Some(event_name) = val.get("event").and_then(Value::as_str) {
                    match event_name {
                        "property-change" => {
                            let prop_name = val.get("name").and_then(Value::as_str).unwrap_or("");
                            let data = val.get("data");
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() != Some(&sid) {
                                break;
                            }
                            let mut changed = false;
                            match prop_name {
                                "time-pos" => {
                                    if let Some(pos) = data.and_then(Value::as_f64) {
                                        if (state_guard.position - pos).abs() >= 0.1 {
                                            state_guard.position = pos;
                                            state_guard.active = true;
                                            changed = true;
                                            let mut w = watch_for_reader.lock().await;
                                            if let Some((ref mut entry, _)) = *w {
                                                entry.position = pos;
                                                entry.watched_at = timestamp_ms();
                                            }
                                        }
                                    }
                                }
                                "duration" => {
                                    if let Some(dur) = data.and_then(Value::as_f64) {
                                        if dur > 0.0 && (state_guard.duration - dur).abs() >= 0.1 {
                                            state_guard.duration = dur;
                                            changed = true;
                                            let mut w = watch_for_reader.lock().await;
                                            if let Some((ref mut entry, _)) = *w {
                                                entry.duration = dur;
                                            }
                                        }
                                    }
                                }
                                "pause" => {
                                    if let Some(p) = data.and_then(Value::as_bool) {
                                        if state_guard.paused != p {
                                            state_guard.paused = p;
                                            if state_guard.active {
                                                state_guard.playing = !p;
                                            }
                                            changed = true;
                                        }
                                    }
                                }
                                "volume" => {
                                    if let Some(vol) = data.and_then(Value::as_f64) {
                                        if (state_guard.volume - vol).abs() >= 0.1 {
                                            state_guard.volume = vol;
                                            changed = true;
                                        }
                                    }
                                }
                                "speed" => {
                                    if let Some(spd) = data.and_then(Value::as_f64) {
                                        if (state_guard.speed - spd).abs() >= 0.01 {
                                            state_guard.speed = spd;
                                            changed = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if changed {
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "file-loaded" => {
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.active = true;
                                state_guard.playing = !state_guard.paused;
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::FileLoaded {
                                    session_id: sid.clone(),
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "end-file" => {
                            let reason = val
                                .get("reason")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string();
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.playing = false;
                                state_guard.end_reason = Some(reason.clone());
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::EndFile {
                                    session_id: sid.clone(),
                                    reason,
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                        }
                        "shutdown" => {
                            let mut state_guard = state_for_reader.write().await;
                            if state_guard.session_id.as_deref() == Some(&sid) {
                                state_guard.active = false;
                                state_guard.playing = false;
                                state_guard.paused = false;
                                let st = state_guard.clone();
                                drop(state_guard);
                                let _ = events_tx_for_reader.send(PlayerEvent::Shutdown {
                                    session_id: sid.clone(),
                                });
                                let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                                    state: st,
                                });
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }

            {
                let mut state_guard = state_for_reader.write().await;
                if state_guard.session_id.as_deref() == Some(&sid) {
                    state_guard.active = false;
                    state_guard.playing = false;
                    state_guard.paused = false;
                    let st = state_guard.clone();
                    drop(state_guard);
                    let _ = events_tx_for_reader.send(PlayerEvent::Shutdown {
                        session_id: sid.clone(),
                    });
                    let _ = events_tx_for_reader.send(PlayerEvent::StateChanged {
                        state: st,
                    });
                }
            }

            cleanup_socket(&sock_path);

            for (_, tx) in pending_for_reader.lock().await.drain() {
                let _ = tx.send(Err(anyhow::anyhow!("Player disconnected")));
            }
        });
    }

    pub async fn command(&self, command: Value) -> Result<Value> {
        let (tx, rx) = oneshot::channel();
        let command_tx = {
            let guard = self.session.lock().await;
            let Some(ref playback) = *guard else {
                bail!("No active playback");
            };
            playback.command_tx.clone()
        };
        command_tx
            .send((command, tx))
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send player command: player is shutting down"))?;
        tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .context("Player response timed out")?
            .context("Player dropped response channel")?
    }

    pub async fn stop(&self) {
        let session = self.session.lock().await.take();
        if let Some(mut active) = session {
            if let Some(mut child) = active.child.take() {
                let _ = child.kill().await;
            }
            cleanup_socket(&active.socket);
            {
                let mut state_guard = self.state.write().await;
                if state_guard.session_id.as_deref() == Some(&active.session_id) {
                    state_guard.active = false;
                    state_guard.playing = false;
                    state_guard.paused = false;
                    let st = state_guard.clone();
                    drop(state_guard);
                    let _ = self.events_tx.send(PlayerEvent::Shutdown {
                        session_id: active.session_id,
                    });
                    let _ = self.events_tx.send(PlayerEvent::StateChanged {
                        state: st,
                    });
                }
            }
        }
    }
}

pub fn cleanup_socket(socket: &std::path::Path) {
    let _ = std::fs::remove_file(socket);
    if let Some(parent) = socket.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}
