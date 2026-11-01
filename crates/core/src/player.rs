use crate::models::*;
use crate::repository::network_url;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::os::unix::fs::DirBuilderExt;
use std::{path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::Mutex,
};

pub struct Player {
    directory: PathBuf,
    process: Mutex<Option<Playback>>,
}
struct Playback {
    child: Child,
    socket: PathBuf,
    watch: Option<WatchEntry>,
}
impl Player {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            process: Mutex::new(None),
        }
    }
    pub async fn play(
        &self,
        stream: &Stream,
        subtitles: &[Subtitle],
        watch: Option<WatchEntry>,
        resume: f64,
    ) -> Result<()> {
        network_url(&stream.url)?;
        if !resume.is_finite() || resume < 0.0 {
            bail!("Invalid resume position")
        }
        // A short private directory avoids Unix socket path limits when app data
        // lives under a long project path, and restricts access to the player IPC.
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
            // -append accepts one literal field, including commas and backslashes.
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
        let mut guard = self.process.lock().await;
        if let Some(mut old) = guard.take() {
            let _ = old.child.kill().await;
            cleanup_socket(&old.socket);
        }
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&private_directory)?;
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                cleanup_socket(&socket);
                return Err(error).context("mpv is unavailable. Install mpv to enable playback");
            }
        };
        *guard = Some(Playback {
            child,
            socket: socket.clone(),
            watch,
        });
        for _ in 0..50 {
            if socket.exists() {
                return Ok(());
            }
            if guard.as_mut().unwrap().child.try_wait()?.is_some() {
                cleanup_socket(&socket);
                *guard = None;
                bail!("mpv exited before playback started")
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        cleanup_socket(&socket);
        *guard = None;
        bail!("mpv did not start its playback connection")
    }
    pub async fn command(&self, command: Value) -> Result<Value> {
        let guard = self.process.lock().await;
        let Some(playback) = guard.as_ref() else {
            bail!("No active playback")
        };
        socket_command(&playback.socket, command).await
    }
    pub async fn progress(&self) -> Result<Option<WatchEntry>> {
        let mut guard = self.process.lock().await;
        let Some(playback) = guard.as_mut() else {
            return Ok(None);
        };
        let position = socket_command(&playback.socket, json!(["get_property", "time-pos"]))
            .await
            .ok()
            .and_then(|v| v.as_f64());
        let duration = socket_command(&playback.socket, json!(["get_property", "duration"]))
            .await
            .ok()
            .and_then(|v| v.as_f64());
        if let Some(watch) = playback.watch.as_mut() {
            if let Some(position) = position {
                watch.position = position;
            }
            if let Some(duration) = duration {
                watch.duration = duration;
            }
            watch.watched_at = timestamp_ms();
        }
        let result = playback.watch.clone();
        if playback.child.try_wait()?.is_some() {
            cleanup_socket(&playback.socket);
            *guard = None;
        }
        Ok(result)
    }
    pub async fn stop(&self) {
        if let Some(mut playback) = self.process.lock().await.take() {
            let _ = playback.child.kill().await;
            cleanup_socket(&playback.socket);
        }
    }
}
fn cleanup_socket(socket: &std::path::Path) {
    let _ = std::fs::remove_file(socket);
    if let Some(parent) = socket.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}
async fn socket_command(socket: &PathBuf, command: Value) -> Result<Value> {
    let operation = async {
        let mut stream = UnixStream::connect(socket).await?;
        stream
            .write_all(format!("{}\n", json!({"command":command,"request_id":1})).as_bytes())
            .await?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
                bail!("Player disconnected")
            }
            let response: Value = serde_json::from_str(&line)?;
            if response.get("request_id") == Some(&json!(1)) {
                if response.get("error").and_then(Value::as_str) != Some("success") {
                    bail!("Player could not complete the command")
                }
                return Ok(response.get("data").cloned().unwrap_or(Value::Null));
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), operation)
        .await
        .context("Player response timed out")?
}
