//! Run explicitly on a Linux desktop with mpv installed. The served audio is silent.
use cloudstream_core::{models::Stream, player::Player};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "requires a Linux display and mpv; opens a temporary silent player window"]
async fn native_mpv_receives_literal_headers_and_accepts_playback_commands() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Three seconds of mono PCM silence, so this integration test makes no sound.
    let pcm_bytes = 8_000u32 * 2 * 3;
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
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let wav = wav.clone();
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut bytes = vec![0; 16_384];
                let count = socket.read(&mut bytes).await.unwrap();
                recorded
                    .lock()
                    .await
                    .push(String::from_utf8_lossy(&bytes[..count]).into_owned());
                let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", wav.len());
                let _ = socket.write_all(headers.as_bytes()).await;
                let _ = socket.write_all(&wav).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let stream = Stream {
        name: "Silent playback test".into(),
        url: format!("http://{address}/silent.wav"),
        referer: "https://source.example/".into(),
        quality: 0,
        headers: BTreeMap::from([("X-Playback-Test".into(), "a,b\\literal".into())]),
        stream_type: "VIDEO".into(),
        ..Default::default()
    };
    player.play(&stream, &[], None, 0.0).await.unwrap();
    let mut duration = None;
    for _ in 0..30 {
        duration = player
            .command(json!(["get_property", "duration"]))
            .await
            .ok()
            .and_then(|v| v.as_f64());
        if duration.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let pause = player.command(json!(["set_property", "pause", true])).await;
    let seek = player.command(json!(["seek", 1, "absolute"])).await;
    player.stop().await;
    server.abort();
    assert!(
        duration.is_some_and(|d| (d - 3.0).abs() < 0.1),
        "mpv failed to decode the HTTP audio fixture"
    );
    assert!(pause.is_ok());
    assert!(seek.is_ok());
    let requests = requests.lock().await;
    assert!(
        requests
            .iter()
            .any(|r| r.contains("X-Playback-Test: a,b\\literal")),
        "comma/backslash header was changed: {requests:?}"
    );
    assert!(requests.iter().any(|r| r
        .to_lowercase()
        .contains("referer: https://source.example/")));
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none(),
        "player IPC directory was not cleaned up"
    );
}
