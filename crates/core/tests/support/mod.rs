//! Shared fixtures for the player tests: silent WAV audio, a tiny HTTP server that records
//! requests, and a headless mpv launcher (no window, no sound).
#![allow(dead_code)]

use cloudstream_core::{models::PlayerState, player::Player};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Mono 8 kHz 16-bit PCM silence of the given length.
pub fn wav(seconds: f64) -> Vec<u8> {
    let pcm_bytes = (8_000.0 * 2.0 * seconds) as u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8_000u32.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&pcm_bytes.to_le_bytes());
    wav.resize(wav.len() + pcm_bytes as usize, 0);
    wav
}

pub struct Fixture {
    pub address: std::net::SocketAddr,
    pub requests: Arc<tokio::sync::Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    pub async fn requested(&self, path: &str) -> bool {
        self.requests
            .lock()
            .await
            .iter()
            .any(|r| r.lines().next().is_some_and(|l| l.split(' ').nth(1) == Some(path)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serve `routes` (path to body) with `Range` support; every other path is a 404.
pub async fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let recorded = requests.clone();
    let routes = Arc::new(routes);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let routes = routes.clone();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16_384];
                let Ok(count) = socket.read(&mut buffer).await else { return };
                let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
                recorded.lock().await.push(request.clone());
                let path = request
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("/")
                    .to_string();
                let Some((_, body)) = routes.iter().find(|(p, _)| *p == path) else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                };
                let range_start = request.lines().find_map(|l| {
                    let lower = l.to_ascii_lowercase();
                    let value = lower.strip_prefix("range: bytes=")?;
                    value.split('-').next()?.trim().parse::<usize>().ok()
                });
                let (status, extra, slice) = match range_start {
                    Some(start) if start < body.len() => (
                        "206 Partial Content",
                        format!("Content-Range: bytes {}-{}/{}\r\n", start, body.len() - 1, body.len()),
                        &body[start..],
                    ),
                    _ => ("200 OK", String::new(), &body[..]),
                };
                let mime = if path.ends_with(".y4m") { "video/x-yuv4mpeg" } else if path.ends_with(".vtt") { "text/vtt" } else { "audio/wav" };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    slice.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(slice).await;
            });
        }
    });
    Fixture { address, requests, task }
}

pub fn mpv_installed() -> bool {
    std::process::Command::new("mpv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A real mpv with no window and no audio output, so the tests run without a display.
pub fn headless_player(directory: &Path) -> Player {
    Player::new(directory.to_owned()).with_launch_args(vec![
        "--vo=null".into(),
        "--ao=null".into(),
        "--force-window=no".into(),
    ])
}

/// Poll the player state until `condition` holds; panic with the last state on timeout.
pub async fn wait_for_state(
    player: &Player,
    seconds: u64,
    what: &str,
    condition: impl Fn(&PlayerState) -> bool,
) -> PlayerState {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let state = player.state().await;
        if condition(&state) {
            return state;
        }
        if Instant::now() > deadline {
            panic!("timed out waiting for {what}; last state: {state:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
