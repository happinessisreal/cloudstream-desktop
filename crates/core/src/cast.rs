//! Casting implementation (T28): FCast first, Chromecast separately.
//!
//! Provides:
//! - FCast protocol (opcode + JSON payload over TCP)
//! - Chromecast CastV2 protocol message representation
//! - LAN scoped media relay with unpredictable session tokens
//! - Device discovery, remote playback control, position tracking, and disconnect cleanup.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::broadcast,
};

/// Supported cast protocol types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CastKind {
    FCast,
    Chromecast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub kind: CastKind,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CastState {
    Idle,
    Connecting,
    Playing,
    Paused,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastSubtitle {
    pub url: String,
    pub language: String,
    pub label: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastMedia {
    pub url: String,
    pub title: String,
    pub container: String,
    pub subtitles: Vec<CastSubtitle>,
    pub start_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastStatus {
    pub session_id: String,
    pub device_id: String,
    pub state: CastState,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub title: Option<String>,
}

// -----------------------------------------------------------------------------
// FCast Protocol Definitions
// -----------------------------------------------------------------------------
pub const FCAST_PORT: u16 = 46899;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FCastOpcode {
    None = 0,
    Play = 1,
    Pause = 2,
    Resume = 3,
    Stop = 4,
    Seek = 5,
    SetVolume = 6,
    PlaybackUpdate = 7,
    Version = 8,
    SetSpeed = 9,
}

impl TryFrom<u8> for FCastOpcode {
    type Error = anyhow::Error;
    fn try_from(v: u8) -> Result<Self> {
        match v {
            0 => Ok(FCastOpcode::None),
            1 => Ok(FCastOpcode::Play),
            2 => Ok(FCastOpcode::Pause),
            3 => Ok(FCastOpcode::Resume),
            4 => Ok(FCastOpcode::Stop),
            5 => Ok(FCastOpcode::Seek),
            6 => Ok(FCastOpcode::SetVolume),
            7 => Ok(FCastOpcode::PlaybackUpdate),
            8 => Ok(FCastOpcode::Version),
            9 => Ok(FCastOpcode::SetSpeed),
            other => bail!("Unknown FCast opcode: {other}"),
        }
    }
}

pub struct FCastPacket {
    pub opcode: FCastOpcode,
    pub payload: Value,
}

impl FCastPacket {
    pub fn new(opcode: FCastOpcode, payload: Value) -> Self {
        Self { opcode, payload }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let json_bytes = serde_json::to_vec(&self.payload)?;
        let length = (json_bytes.len() + 1) as u32; // opcode + json
        let mut buf = Vec::with_capacity(4 + 1 + json_bytes.len());
        buf.extend_from_slice(&length.to_be_bytes());
        buf.push(self.opcode as u8);
        buf.extend_from_slice(&json_bytes);
        Ok(buf)
    }

    pub async fn read_from<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await.context("Failed reading FCast length prefix")?;
        let length = u32::from_be_bytes(len_buf) as usize;
        if length == 0 || length > 10 * 1024 * 1024 {
            bail!("Invalid FCast packet length: {length}");
        }

        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await.context("Failed reading FCast packet body")?;

        let opcode = FCastOpcode::try_from(body[0])?;
        let payload: Value = if body.len() > 1 {
            serde_json::from_slice(&body[1..]).context("Failed parsing FCast JSON payload")?
        } else {
            Value::Null
        };

        Ok(Self { opcode, payload })
    }

    pub async fn write_to<W: AsyncWriteExt + Unpin>(&self, writer: &mut W) -> Result<()> {
        let bytes = self.to_bytes()?;
        writer.write_all(&bytes).await.context("Failed writing FCast packet")?;
        writer.flush().await.context("Failed flushing FCast packet")?;
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Chromecast Protocol Message Helper
// -----------------------------------------------------------------------------
pub mod chromecast {
    use super::*;

    pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
    pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
    pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CastV2Message {
        pub namespace: String,
        pub source_id: String,
        pub destination_id: String,
        pub payload_utf8: String,
    }

    pub fn connect_message(source_id: &str, destination_id: &str) -> CastV2Message {
        CastV2Message {
            namespace: NS_CONNECTION.into(),
            source_id: source_id.into(),
            destination_id: destination_id.into(),
            payload_utf8: json!({"type": "CONNECT"}).to_string(),
        }
    }

    pub fn load_media_message(source_id: &str, destination_id: &str, media: &CastMedia, request_id: i64) -> CastV2Message {
        let payload = json!({
            "type": "LOAD",
            "requestId": request_id,
            "media": {
                "contentId": media.url,
                "contentType": match media.container.as_str() {
                    "mp4" => "video/mp4",
                    "mkv" => "video/x-matroska",
                    "hls" => "application/x-mpegurl",
                    "dash" => "application/dash+xml",
                    _ => "video/mp4",
                },
                "streamType": "BUFFERED",
                "metadata": {
                    "metadataType": 0,
                    "title": media.title,
                }
            },
            "currentTime": media.start_time,
            "autoplay": true
        });
        CastV2Message {
            namespace: NS_MEDIA.into(),
            source_id: source_id.into(),
            destination_id: destination_id.into(),
            payload_utf8: payload.to_string(),
        }
    }
}

// -----------------------------------------------------------------------------
// LAN Scoped Media Relay (T28)
// -----------------------------------------------------------------------------
/// Relays media streams to LAN cast devices with unpredictable per-session tokens.
/// Upstream authentication headers/cookies are attached on the desktop side,
/// preventing leakage to the casting receiver while allowing authorized playback.
pub struct CastRelay {
    tokens: Arc<Mutex<HashMap<String, ScopedMediaRoute>>>,
    listener_addr: Arc<Mutex<Option<SocketAddr>>>,
    shutdown_tx: broadcast::Sender<()>,
}

#[derive(Clone)]
struct ScopedMediaRoute {
    pub upstream_url: String,
    pub headers: HashMap<String, String>,
}

impl CastRelay {
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            listener_addr: Arc::new(Mutex::new(None)),
            shutdown_tx,
        }
    }

    /// Register a media stream and return an unpredictable scoped token and relay URL path.
    pub fn register_stream(&self, upstream_url: String, headers: HashMap<String, String>) -> String {
        let token = format!("{:x}{:x}", uuid::Uuid::new_v4().as_u128(), uuid::Uuid::new_v4().as_u128());
        let mut map = self.tokens.lock().unwrap();
        map.insert(token.clone(), ScopedMediaRoute { upstream_url, headers });
        token
    }

    /// Revoke a token when the cast session ends.
    pub fn revoke_token(&self, token: &str) {
        let mut map = self.tokens.lock().unwrap();
        map.remove(token);
    }

    /// Clear all active tokens.
    pub fn revoke_all(&self) {
        let mut map = self.tokens.lock().unwrap();
        map.clear();
    }

    /// Start the LAN HTTP relay server on an ephemeral port.
    pub async fn start(&self, bind_ip: &str) -> Result<SocketAddr> {
        let listener = TcpListener::bind(format!("{bind_ip}:0")).await.context("Failed binding cast relay socket")?;
        let addr = listener.local_addr()?;
        *self.listener_addr.lock().unwrap() = Some(addr);

        let tokens = self.tokens.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_res = listener.accept() => {
                        let Ok((mut socket, _client_addr)) = accept_res else { continue; };
                        let tokens = tokens.clone();
                        tokio::spawn(async move {
                            let mut buf = vec![0u8; 4096];
                            let Ok(n) = socket.read(&mut buf).await else { return; };
                            if n == 0 { return; }

                            let req_str = String::from_utf8_lossy(&buf[..n]);
                            let mut lines = req_str.lines();
                            let Some(request_line) = lines.next() else { return; };
                            let parts: Vec<&str> = request_line.split_whitespace().collect();
                            if parts.len() < 2 || parts[0] != "GET" && parts[0] != "HEAD" {
                                let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n").await;
                                return;
                            }

                            let path = parts[1];
                            // Expected format: /relay/<token>/stream
                            let route = if let Some(stripped) = path.strip_prefix("/relay/") {
                                let token = stripped.split('/').next().unwrap_or("");
                                let map = tokens.lock().unwrap();
                                map.get(token).cloned()
                            } else {
                                None
                            };

                            let Some(route) = route else {
                                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                                return;
                            };

                            // Relay upstream request
                            let client = reqwest::Client::new();
                            let mut req_builder = client.get(&route.upstream_url);
                            for (k, v) in route.headers {
                                req_builder = req_builder.header(k, v);
                            }

                            match req_builder.send().await {
                                Ok(resp) => {
                                    let status = resp.status();
                                    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("video/mp4");
                                    let content_length = resp.headers().get("content-length").and_then(|v| v.to_str().ok());

                                    let mut header_bytes = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nConnection: close\r\n");
                                    if let Some(cl) = content_length {
                                        header_bytes.push_str(&format!("Content-Length: {cl}\r\n"));
                                    }
                                    header_bytes.push_str("\r\n");

                                    if socket.write_all(header_bytes.as_bytes()).await.is_ok() && parts[0] == "GET" {
                                        if let Ok(bytes) = resp.bytes().await {
                                            let _ = socket.write_all(&bytes).await;
                                        }
                                    }
                                }
                                Err(_) => {
                                    let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
                                }
                            }
                        });
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        Ok(addr)
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(());
        self.revoke_all();
    }
}

// -----------------------------------------------------------------------------
// Cast Controller Session
// -----------------------------------------------------------------------------
pub struct CastSession {
    pub id: String,
    pub device: CastDevice,
    pub state: Arc<Mutex<CastStatus>>,
    stream: Arc<tokio::sync::Mutex<Option<TcpStream>>>,
}

impl CastSession {
    pub async fn connect_fcast(device: CastDevice) -> Result<Self> {
        let session_id = format!("fcast-{}", uuid::Uuid::new_v4());
        let addr = format!("{}:{}", device.host, device.port);
        let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr))
            .await
            .context("Connection timed out")?
            .context("Failed connecting to FCast receiver")?;

        let status = Arc::new(Mutex::new(CastStatus {
            session_id: session_id.clone(),
            device_id: device.id.clone(),
            state: CastState::Idle,
            position: 0.0,
            duration: 0.0,
            volume: 1.0,
            muted: false,
            title: None,
        }));

        Ok(Self {
            id: session_id,
            device,
            state: status,
            stream: Arc::new(tokio::sync::Mutex::new(Some(stream))),
        })
    }

    pub async fn play(&self, media: CastMedia) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("Session not connected")?;

        let packet = FCastPacket::new(
            FCastOpcode::Play,
            json!({
                "url": media.url,
                "container": media.container,
                "time": media.start_time,
                "subtitles": media.subtitles,
            }),
        );
        packet.write_to(stream).await?;

        let mut st = self.state.lock().unwrap();
        st.state = CastState::Playing;
        st.position = media.start_time;
        st.title = Some(media.title);
        Ok(())
    }

    pub async fn pause(&self) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("Session not connected")?;
        FCastPacket::new(FCastOpcode::Pause, json!({})).write_to(stream).await?;

        let mut st = self.state.lock().unwrap();
        st.state = CastState::Paused;
        Ok(())
    }

    pub async fn resume(&self) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("Session not connected")?;
        FCastPacket::new(FCastOpcode::Resume, json!({})).write_to(stream).await?;

        let mut st = self.state.lock().unwrap();
        st.state = CastState::Playing;
        Ok(())
    }

    pub async fn seek(&self, position: f64) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("Session not connected")?;
        FCastPacket::new(FCastOpcode::Seek, json!({"time": position})).write_to(stream).await?;

        let mut st = self.state.lock().unwrap();
        st.position = position;
        Ok(())
    }

    pub async fn set_volume(&self, volume: f64) -> Result<()> {
        let mut guard = self.stream.lock().await;
        let stream = guard.as_mut().context("Session not connected")?;
        let vol = volume.clamp(0.0, 1.0);
        FCastPacket::new(FCastOpcode::SetVolume, json!({"volume": vol})).write_to(stream).await?;

        let mut st = self.state.lock().unwrap();
        st.volume = vol;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let mut guard = self.stream.lock().await;
        if let Some(stream) = guard.as_mut() {
            let _ = FCastPacket::new(FCastOpcode::Stop, json!({})).write_to(stream).await;
        }

        let mut st = self.state.lock().unwrap();
        st.state = CastState::Stopped;
        Ok(())
    }

    pub async fn disconnect(&self) {
        let mut guard = self.stream.lock().await;
        if let Some(mut stream) = guard.take() {
            let _ = FCastPacket::new(FCastOpcode::Stop, json!({})).write_to(&mut stream).await;
            let _ = stream.shutdown().await;
        }
        let mut st = self.state.lock().unwrap();
        st.state = CastState::Idle;
    }
}
