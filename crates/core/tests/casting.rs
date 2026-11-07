use cloudstream_core::cast::{
    CastDevice, CastKind, CastMedia, CastRelay, CastSession, FCastOpcode, FCastPacket,
};
use serde_json::json;
use std::collections::HashMap;
use tokio::net::TcpListener;

#[tokio::test]
async fn fcast_packet_round_trip() {
    let payload = json!({
        "url": "https://example.test/video.mp4",
        "container": "mp4",
        "time": 42.5
    });
    let packet = FCastPacket::new(FCastOpcode::Play, payload.clone());
    let bytes = packet.to_bytes().unwrap();

    let mut cursor = std::io::Cursor::new(bytes);
    let decoded = FCastPacket::read_from(&mut cursor).await.unwrap();
    assert_eq!(decoded.opcode, FCastOpcode::Play);
    assert_eq!(decoded.payload["url"], "https://example.test/video.mp4");
    assert_eq!(decoded.payload["time"], 42.5);
}

#[tokio::test]
async fn cast_relay_scoped_tokens_and_revocation() {
    let relay = CastRelay::new();
    let addr = relay.start("127.0.0.1").await.unwrap();

    // 1. Start a fake upstream server
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = upstream_listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\n\r\nHELLO")
                .await;
        }
    });

    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), "Bearer secret-token".to_string());
    let token = relay.register_stream(format!("http://{upstream_addr}/video.mp4"), headers);

    // 2. Fetch through relay with valid token
    let relay_url = format!("http://{addr}/relay/{token}/stream");
    let resp = reqwest::get(&relay_url).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "HELLO");

    // 3. Revoke token and verify access is refused
    relay.revoke_token(&token);
    let resp2 = reqwest::get(&relay_url).await.unwrap();
    assert_eq!(resp2.status(), 404);

    relay.stop();
}

#[tokio::test]
async fn fcast_session_commands_over_tcp() {
    let receiver = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = receiver.local_addr().unwrap();

    let received_opcodes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let opcodes_clone = received_opcodes.clone();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = receiver.accept().await {
            while let Ok(packet) = FCastPacket::read_from(&mut socket).await {
                opcodes_clone.lock().unwrap().push(packet.opcode);
                if packet.opcode == FCastOpcode::Stop {
                    break;
                }
            }
        }
    });

    let device = CastDevice {
        id: "test-device".to_string(),
        name: "Living Room TV".to_string(),
        kind: CastKind::FCast,
        host: "127.0.0.1".to_string(),
        port: addr.port(),
    };

    let session = CastSession::connect_fcast(device).await.unwrap();
    let media = CastMedia {
        url: "http://example.test/stream.mp4".to_string(),
        title: "Test Movie".to_string(),
        container: "mp4".to_string(),
        subtitles: vec![],
        start_time: 0.0,
    };

    session.play(media).await.unwrap();
    session.pause().await.unwrap();
    session.resume().await.unwrap();
    session.seek(120.0).await.unwrap();
    session.set_volume(0.8).await.unwrap();
    session.stop().await.unwrap();
    session.disconnect().await;

    // Small delay to allow receiver task to record all packets
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let opcodes = received_opcodes.lock().unwrap().clone();
    assert_eq!(
        opcodes,
        vec![
            FCastOpcode::Play,
            FCastOpcode::Pause,
            FCastOpcode::Resume,
            FCastOpcode::Seek,
            FCastOpcode::SetVolume,
            FCastOpcode::Stop,
        ]
    );
}
