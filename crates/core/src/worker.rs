use crate::models::*;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, oneshot, Mutex, Semaphore},
};

#[derive(Clone)]
pub struct WorkerConfig {
    pub java: PathBuf,
    pub jar: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WorkerRpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for WorkerRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for WorkerRpcError {}

type ResponseSender = oneshot::Sender<Result<Value, WorkerRpcError>>;
pub type ProgressSender = mpsc::UnboundedSender<Value>;

struct InFlight {
    sender: ResponseSender,
    progress: Option<ProgressSender>,
}

pub struct WorkerSession {
    child: Mutex<Option<Child>>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<u64, InFlight>>>,
    sequence: Arc<AtomicU64>,
    dead: Arc<AtomicBool>,
}

pub struct Worker {
    config: WorkerConfig,
    session: Mutex<Option<Arc<WorkerSession>>>,
    concurrency_limit: Arc<Semaphore>,
    sequence: Arc<AtomicU64>,
}

impl Worker {
    pub fn new(config: WorkerConfig) -> Self {
        Self {
            config,
            session: Mutex::new(None),
            concurrency_limit: Arc::new(Semaphore::new(32)),
            sequence: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn next_id(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst)
    }

    async fn start(&self) -> Result<WorkerSession> {
        if !self.config.jar.is_file() {
            bail!("Extension worker is missing. Run npm run worker:build")
        }
        let mut child = Command::new(&self.config.java)
            .args(["-Xmx512m", "-Djava.awt.headless=true", "-jar"])
            .arg(&self.config.jar)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Could not start Java. Build the bundled runtime or install Java 21+")?;

        let stdin = child.stdin.take().context("Worker stdin is missing")?;
        let stdout = child.stdout.take().context("Worker stdout is missing")?;

        let pending: Arc<Mutex<HashMap<u64, InFlight>>> = Arc::new(Mutex::new(HashMap::new()));
        let dead = Arc::new(AtomicBool::new(false));

        let pending_clone = Arc::clone(&pending);
        let dead_clone = Arc::clone(&dead);

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let val: Value = match serde_json::from_str(trimmed) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("Worker emitted malformed line ({e}): {trimmed}");
                                continue;
                            }
                        };

                        // Check for progress notifications
                        if val.get("method").and_then(Value::as_str) == Some("progress") {
                            if let Some(params) = val.get("params") {
                                if let Some(req_id) = params.get("id").and_then(Value::as_u64) {
                                    let map = pending_clone.lock().await;
                                    if let Some(entry) = map.get(&req_id) {
                                        if let Some(prog) = &entry.progress {
                                            let _ = prog.send(params.clone());
                                        }
                                    }
                                }
                            }
                            continue;
                        }

                        // Response with id
                        if let Some(req_id) = val.get("id").and_then(Value::as_u64) {
                            let in_flight = {
                                let mut map = pending_clone.lock().await;
                                map.remove(&req_id)
                            };
                            if let Some(entry) = in_flight {
                                if let Some(err) = val.get("error") {
                                    let code =
                                        err.get("code").and_then(Value::as_i64).unwrap_or(-32000);
                                    let msg = err
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Extension failed");
                                    let _ = entry.sender.send(Err(WorkerRpcError {
                                        code,
                                        message: msg.to_string(),
                                    }));
                                } else {
                                    let res = val.get("result").cloned().unwrap_or(Value::Null);
                                    let _ = entry.sender.send(Ok(res));
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("Worker stdout read error: {e}");
                        break;
                    }
                }
            }
            dead_clone.store(true, Ordering::SeqCst);
            let mut map = pending_clone.lock().await;
            for (_id, entry) in map.drain() {
                let _ = entry.sender.send(Err(WorkerRpcError {
                    code: -32000,
                    message: "Worker disconnected".to_string(),
                }));
            }
        });

        let session = WorkerSession {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            pending,
            sequence: Arc::clone(&self.sequence),
            dead,
        };

        let health = session.call("health", json!({})).await?;
        let protocol_version = health.get("protocolVersion").and_then(Value::as_u64);
        if !matches!(protocol_version, Some(1 | 2)) {
            bail!("Extension worker protocol is incompatible (expected version 1 or 2, got {protocol_version:?})")
        }

        Ok(session)
    }

    async fn get_or_restart_session(
        &self,
        installed: &[InstalledPlugin],
    ) -> Result<Arc<WorkerSession>> {
        let mut guard = self.session.lock().await;
        let needs_restart = match guard.as_ref() {
            Some(s) => s.is_dead(),
            None => true,
        };
        if needs_restart {
            *guard = None;
            let new_session = Arc::new(self.start().await?);
            for plugin in installed {
                if let Err(error) = new_session
                    .call(
                        "loadPlugin",
                        json!({"pluginId": plugin.id, "path": plugin.path}),
                    )
                    .await
                {
                    eprintln!(
                        "Extension {} could not be restored: {error}",
                        plugin.entry.name
                    );
                }
            }
            *guard = Some(Arc::clone(&new_session));
            Ok(new_session)
        } else {
            Ok(Arc::clone(guard.as_ref().unwrap()))
        }
    }

    pub async fn request_with_id(
        &self,
        installed: &[InstalledPlugin],
        id: u64,
        method: &str,
        params: Value,
        progress: Option<ProgressSender>,
    ) -> Result<Value> {
        let _permit = self
            .concurrency_limit
            .acquire()
            .await
            .context("Worker concurrency limit exceeded")?;

        let session = self.get_or_restart_session(installed).await?;
        let response = session
            .call_internal(id, method, params, progress)
            .await;

        if response.as_ref().err().is_some_and(|e| {
            e.to_string().contains("Worker disconnected")
                || e.to_string().contains("Worker action timed out")
        }) {
            let mut guard = self.session.lock().await;
            if let Some(current) = guard.as_ref() {
                if Arc::ptr_eq(current, &session) {
                    *guard = None;
                }
            }
        }
        response
    }

    pub async fn request(
        &self,
        installed: &[InstalledPlugin],
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let id = self.next_id();
        self.request_with_id(installed, id, method, params, None)
            .await
    }

    pub async fn request_with_progress(
        &self,
        installed: &[InstalledPlugin],
        method: &str,
        params: Value,
        progress: Option<ProgressSender>,
    ) -> Result<Value> {
        let id = self.next_id();
        self.request_with_id(installed, id, method, params, progress)
            .await
    }

    pub async fn cancel(&self, id: u64) {
        let guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            session.cancel(id).await;
        }
    }

    pub async fn create_media_session(
        &self,
        installed: &[InstalledPlugin],
        resource_handle: &str,
    ) -> Result<MediaSessionInfo> {
        let val = self
            .request(
                installed,
                "createMediaSession",
                json!({ "resourceHandle": resource_handle }),
            )
            .await?;
        let info: MediaSessionInfo = serde_json::from_value(val)?;
        Ok(info)
    }

    pub async fn close_media_session(
        &self,
        installed: &[InstalledPlugin],
        session_token: &str,
    ) -> Result<bool> {
        let val = self
            .request(
                installed,
                "closeMediaSession",
                json!({ "sessionToken": session_token }),
            )
            .await?;
        Ok(val.get("closed").and_then(|v| v.as_bool()).unwrap_or(true))
    }

    pub async fn get_webview_status(
        &self,
        installed: &[InstalledPlugin],
    ) -> Result<WebViewReport> {
        let val = self.request(installed, "getWebViewStatus", json!({})).await?;
        let report: WebViewReport = serde_json::from_value(val)?;
        Ok(report)
    }

    pub async fn stop(&self) {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.take() {
            session.stop().await;
        }
    }
}

impl WorkerSession {
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    pub fn next_id(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_with_progress(method, params, None).await
    }

    pub async fn call_with_progress(
        &self,
        method: &str,
        params: Value,
        progress: Option<ProgressSender>,
    ) -> Result<Value> {
        let id = self.next_id();
        self.call_internal(id, method, params, progress).await
    }

    pub async fn call_internal(
        &self,
        id: u64,
        method: &str,
        params: Value,
        progress: Option<ProgressSender>,
    ) -> Result<Value> {
        if self.is_dead() {
            bail!("Worker disconnected");
        }

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(
                id,
                InFlight {
                    sender: tx,
                    progress,
                },
            );
        }

        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = stdin.write_all(&payload).await {
                self.pending.lock().await.remove(&id);
                bail!("Worker disconnected: {e}");
            }
            if let Err(e) = stdin.write_all(b"\n").await {
                self.pending.lock().await.remove(&id);
                bail!("Worker disconnected: {e}");
            }
            if let Err(e) = stdin.flush().await {
                self.pending.lock().await.remove(&id);
                bail!("Worker disconnected: {e}");
            }
        }

        match tokio::time::timeout(std::time::Duration::from_secs(65), rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(rpc_err))) => {
                bail!("{rpc_err}")
            }
            Ok(Err(_oneshot_dropped)) => {
                bail!("Worker disconnected")
            }
            Err(_timeout) => {
                self.cancel(id).await;
                bail!("Worker action timed out")
            }
        }
    }

    pub async fn cancel(&self, id: u64) {
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
        }

        let payload = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": "cancel",
            "params": { "id": id }
        }));

        if let Ok(line) = payload {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.write_all(&line).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
    }

    pub async fn stop(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let mut child = self.child.lock().await;
        if let Some(mut c) = child.take() {
            let _ = c.kill().await;
        }
    }
}
