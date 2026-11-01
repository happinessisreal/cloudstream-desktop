use crate::models::*;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

#[derive(Clone)]
pub struct WorkerConfig {
    pub java: PathBuf,
    pub jar: PathBuf,
}
pub struct Worker {
    config: WorkerConfig,
    process: Mutex<Option<Process>>,
}
struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
}

impl Worker {
    pub fn new(config: WorkerConfig) -> Self {
        Self {
            config,
            process: Mutex::new(None),
        }
    }
    async fn start(&self) -> Result<Process> {
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
        let input = child.stdin.take().context("Worker stdin is missing")?;
        let output = BufReader::new(child.stdout.take().context("Worker stdout is missing")?);
        let mut process = Process {
            child,
            input,
            output,
            sequence: 0,
        };
        let health = process.call("health", json!({})).await?;
        let protocol_version = health.get("protocolVersion").and_then(Value::as_u64);
        if !matches!(protocol_version, Some(1 | 2)) {
            bail!("Extension worker protocol is incompatible (expected version 1 or 2, got {protocol_version:?})")
        }
        Ok(process)
    }
    pub async fn request(
        &self,
        installed: &[InstalledPlugin],
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let mut guard = self.process.lock().await;
        let exited = match guard.as_mut() {
            Some(process) => process.child.try_wait()?.is_some(),
            None => true,
        };
        if exited {
            *guard = None;
            let mut process = self.start().await?;
            for plugin in installed {
                // A bad extension must not prevent healthy extensions from being restored.
                if let Err(error) = process
                    .call(
                        "loadPlugin",
                        json!({"pluginId":plugin.id,"path":plugin.path}),
                    )
                    .await
                {
                    eprintln!(
                        "Extension {} could not be restored: {error}",
                        plugin.entry.name
                    );
                }
            }
            *guard = Some(process);
        }
        let response = guard
            .as_mut()
            .ok_or_else(|| anyhow!("Worker failed to start"))?
            .call(method, params)
            .await;
        if response.as_ref().err().is_some_and(|e| {
            e.downcast_ref::<std::io::Error>().is_some()
                || e.to_string().contains("Worker timed out")
                || e.to_string().contains("Worker disconnected")
        }) {
            *guard = None;
        }
        response
    }
    pub async fn stop(&self) {
        *self.process.lock().await = None;
    }
}
impl Process {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        let id = self.sequence;
        let line =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let operation = async {
            self.input.write_all(&line).await?;
            self.input.write_all(b"\n").await?;
            self.input.flush().await?;
            let mut response = String::new();
            if self.output.read_line(&mut response).await? == 0 {
                bail!("Worker disconnected. Retry to restart it")
            }
            if response.len() > 16 * 1024 * 1024 {
                bail!("Worker response exceeded the limit")
            }
            let response: Value =
                serde_json::from_str(&response).context("Worker returned invalid JSON")?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                bail!("Worker response identity mismatch")
            }
            if let Some(error) = response.get("error") {
                bail!(
                    "{}",
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Extension failed")
                )
            }
            Ok(response.get("result").cloned().unwrap_or(Value::Null))
        };
        tokio::time::timeout(std::time::Duration::from_secs(125), operation)
            .await
            .map_err(|_| anyhow!("Worker timed out. Retry to restart it"))?
    }
}
