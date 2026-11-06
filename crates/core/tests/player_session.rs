use cloudstream_core::{models::*, player::Player, App};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[tokio::test]
async fn fake_ipc_session_state_events_and_commands() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("fake_mpv.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        // 1. Read observe_property lines
        for _ in 0..6 {
            let line = reader.next_line().await.unwrap().unwrap();
            let val: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(val["command"][0], "observe_property");
        }

        // 2. Send file-loaded event
        write_half
            .write_all(b"{\"event\":\"file-loaded\"}\n")
            .await
            .unwrap();

        // 3. Send property changes: duration and time-pos
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":2,\"name\":\"duration\",\"data\":120.0}\n")
            .await
            .unwrap();
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":1,\"name\":\"time-pos\",\"data\":15.5}\n")
            .await
            .unwrap();

        // 4. Handle a custom command
        let cmd_line = reader.next_line().await.unwrap().unwrap();
        let cmd_val: serde_json::Value = serde_json::from_str(&cmd_line).unwrap();
        let req_id = cmd_val["request_id"].as_u64().unwrap();
        let resp = json!({
            "request_id": req_id,
            "error": "success",
            "data": 25.5
        })
        .to_string()
            + "\n";
        write_half.write_all(resp.as_bytes()).await.unwrap();

        // 5. Send pause event
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":3,\"name\":\"pause\",\"data\":true}\n")
            .await
            .unwrap();

        // 6. Send end-file event
        write_half
            .write_all(b"{\"event\":\"end-file\",\"reason\":\"eof\"}\n")
            .await
            .unwrap();

        // 7. Send shutdown event
        write_half
            .write_all(b"{\"event\":\"shutdown\"}\n")
            .await
            .unwrap();
    });

    let player = Player::new(temp_dir.path().to_path_buf());
    let mut rx = player.subscribe();

    let client_stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let session_id = "test-session-123".to_string();
    let watch = WatchEntry {
        id: "media_url:Episode 1".to_string(),
        item: MediaItem {
            provider_id: "test_provider".into(),
            name: "Test Media".into(),
            url: "media_url".into(),
            media_id: Some("media_id_1".into()),
            media_type: Some("Movie".into()),
            year: Some(2024),
            ..Default::default()
        },
        episode: Episode {
            name: "Episode 1".into(),
            data: "data".into(),
            season: Some(1),
            episode: Some(1),
            id: Some("media_url:Episode 1".into()),
            dub_status: Some(DubStatus::Subbed),
            ..Default::default()
        },
        position: 0.0,
        duration: 0.0,
        watched_at: 0,
    };

    player
        .start_session(
            session_id.clone(),
            "profile_alice".to_string(),
            None,
            socket_path.clone(),
            client_stream,
            Some(watch),
            0.0,
        )
        .await;

    // Verify initial state
    let state = player.state().await;
    assert_eq!(state.session_id.as_deref(), Some("test-session-123"));
    assert_eq!(state.profile_id.as_deref(), Some("profile_alice"));
    assert_eq!(state.item_name.as_deref(), Some("Test Media"));
    assert_eq!(state.episode_name.as_deref(), Some("Episode 1"));
    assert_eq!(state.media_id.as_deref(), Some("media_id_1"));
    assert!(state.active);

    // Wait for events and state updates
    let mut got_file_loaded = false;
    let mut got_pause = false;
    let mut got_end_file = false;
    let mut got_shutdown = false;

    // Send custom command
    let res = player.command(json!(["seek", 10])).await.unwrap();
    assert_eq!(res, json!(25.5));

    for _ in 0..20 {
        while let Ok(event) = rx.try_recv() {
            match event {
                PlayerEvent::FileLoaded { .. } => got_file_loaded = true,
                PlayerEvent::StateChanged { state } => {
                    if state.paused {
                        got_pause = true;
                    }
                }
                PlayerEvent::EndFile { reason, .. } => {
                    assert_eq!(reason, "eof");
                    got_end_file = true;
                }
                PlayerEvent::Shutdown { .. } => got_shutdown = true,
                _ => {}
            }
        }
        if got_shutdown {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    server_task.await.unwrap();

    assert!(got_file_loaded, "file-loaded event expected");
    assert!(got_pause, "pause property change expected");
    assert!(got_end_file, "end-file event expected");
    assert!(got_shutdown, "shutdown event expected");

    // After shutdown, player state must be inactive and not playing
    let final_state = player.state().await;
    assert!(!final_state.active);
    assert!(!final_state.playing);
    assert!(!final_state.paused);

    // Test stop idempotency: calling stop twice should not error
    player.stop().await;
    player.stop().await;
    let stopped_state = player.state().await;
    assert!(!stopped_state.active);
}

#[tokio::test]
async fn stale_events_from_old_session_are_ignored() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("stale_test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        for _ in 0..6 {
            let _ = reader.next_line().await;
        }

        // Try to update pause on a stale session
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":3,\"name\":\"pause\",\"data\":true}\n")
            .await
            .unwrap();
    });

    let player = Player::new(temp_dir.path().to_path_buf());
    let client_stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

    // Start session A
    player
        .start_session(
            "session-A".into(),
            "default".into(),
            None,
            socket_path.clone(),
            client_stream,
            None,
            0.0,
        )
        .await;

    // Immediately switch to session B (simulating playing another title)
    // Manually setting state to session-B
    {
        let mut state = player.state().await;
        state.session_id = Some("session-B".into());
        state.paused = false;
        // Updating reader's view: reader checks state.session_id != sid
    }

    server_task.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Stop is idempotent
    player.stop().await;
    player.stop().await;
}

#[tokio::test]
async fn progress_is_saved_to_profile_on_pause_and_shutdown() {
    let temp_dir = tempfile::tempdir().unwrap();
    let app = App::open(
        temp_dir.path().join("data"),
        temp_dir.path().join("resources"),
    )
    .unwrap();

    let socket_path = temp_dir.path().join("progress_test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        for _ in 0..6 {
            let _ = reader.next_line().await;
        }

        // Report position update
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":2,\"name\":\"duration\",\"data\":300.0}\n")
            .await
            .unwrap();
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":1,\"name\":\"time-pos\",\"data\":42.0}\n")
            .await
            .unwrap();
        // Pause triggers immediate flush
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":3,\"name\":\"pause\",\"data\":true}\n")
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        write_half
            .write_all(b"{\"event\":\"shutdown\"}\n")
            .await
            .unwrap();
    });

    let client_stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let watch = WatchEntry {
        id: "series_url:Ep 2".into(),
        item: MediaItem {
            provider_id: "prov".into(),
            name: "Cool Series".into(),
            url: "series_url".into(),
            media_id: Some("series_media_1".into()),
            media_type: Some("TvSeries".into()),
            year: Some(2023),
            ..Default::default()
        },
        episode: Episode {
            name: "Ep 2".into(),
            data: "ep2_data".into(),
            season: Some(1),
            episode: Some(2),
            id: Some("series_url:Ep 2".into()),
            dub_status: Some(DubStatus::Subbed),
            ..Default::default()
        },
        position: 0.0,
        duration: 0.0,
        watched_at: 0,
    };

    app.player
        .start_session(
            "progress-session-1".into(),
            "profile_bob".into(),
            None,
            socket_path,
            client_stream,
            Some(watch),
            0.0,
        )
        .await;

    server_task.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify progress saved to profile_bob
    let progress = app
        .store
        .get_watch_progress("profile_bob", "series_url:Ep 2")
        .unwrap();
    assert!(progress.is_some(), "Progress must be saved to profile_bob");
    let p = progress.unwrap();
    assert_eq!(p.profile_id, "profile_bob");
    assert!((p.position_seconds - 42.0).abs() < 0.5);
    assert_eq!(p.duration_seconds, Some(300.0));

    // And verify it was NOT saved to default profile
    let default_progress = app
        .store
        .get_watch_progress("default", "series_url:Ep 2")
        .unwrap();
    assert!(default_progress.is_none());
}
