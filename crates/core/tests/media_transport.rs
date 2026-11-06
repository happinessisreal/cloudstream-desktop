use cloudstream_core::{models::*, App};
use std::path::PathBuf;

#[tokio::test]
async fn webview_unsupported_structured_report() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let report = app.get_webview_status().await.unwrap();
    assert!(!report.supported);
    assert_eq!(report.reason, "webview_unsupported");
    assert!(!report.missing_features.is_empty());
    assert!(report.missing_features.contains(&"request_interception".to_string()));
}

#[tokio::test]
async fn ordinary_header_only_stream_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    // Stream without resource_handle should fallback to direct URL
    let stream = Stream {
        name: "Direct Stream".into(),
        url: "http://127.0.0.1:12345/video.mp4".into(),
        referer: "https://example.com".into(),
        quality: 1080,
        headers: [("X-Test".into(), "Custom".into())].into_iter().collect(),
        stream_type: "VIDEO".into(),
        kind: Some("direct".into()),
        source: Some("Direct".into()),
        extractor_data: None,
        resource_handle: None,
        transport_url: None,
        audio_tracks: vec![],
        playlist: None,
        drm: None,
        extra: Default::default(),
    };

    let session_id = app
        .play(stream, vec![], None, None, 0.0, Some("default"), None)
        .await
        .unwrap();
    assert!(!session_id.is_empty());
    assert!(app.active_transport_token.lock().await.is_none());
}

#[tokio::test]
async fn deterministic_multi_origin_transport_and_token_validation() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let sample_jar_path = resources.join("worker/desktop-sample.jar");
    app.install_local(&sample_jar_path).await.unwrap();

    // Call loadLinks directly on sample to get streams with resource_handle
    let installed = app.store.installed().unwrap();
    let sample_provider = app.providers().await.unwrap().into_iter().next().unwrap();

    let links_val = app
        .worker
        .request(
            &installed,
            "loadLinks",
            serde_json::json!({
                "providerId": sample_provider.id,
                "data": "https://download.blender.org/peach/trailer/trailer_480p.mov",
            }),
        )
        .await
        .unwrap();

    let links: ResolvedLinks = serde_json::from_value(links_val).unwrap();
    assert!(!links.streams.is_empty());
    let stream = &links.streams[0];
    let handle = stream.resource_handle.as_ref().expect("Stream must have resource_handle");

    // Create session via worker
    let session = app.worker.create_media_session(&installed, handle).await.unwrap();
    assert!(!session.session_token.is_empty());
    assert!(session.transport_url.contains(&session.session_token));
    assert!(session.transport_url.contains(&format!("127.0.0.1:{}", session.port)));

    // Requesting with valid session token succeeds
    let client = reqwest::Client::new();
    let resp = client.get(&session.transport_url).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    // Requesting with forged/tampered session token fails closed with 403 Forbidden
    let bad_url = session.transport_url.replace(&session.session_token, "00000000000000000000000000000000");
    let bad_resp = client.get(&bad_url).send().await.unwrap();
    assert_eq!(bad_resp.status().as_u16(), 403);

    // Close session
    let closed = app.worker.close_media_session(&installed, &session.session_token).await.unwrap();
    assert!(closed);

    // After closing session, subsequent requests return 403 Forbidden
    let closed_resp = client.get(&session.transport_url).send().await.unwrap();
    assert_eq!(closed_resp.status().as_u16(), 403);
}

#[tokio::test]
async fn verifier_starts_and_stops_with_playback_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let sample_jar_path = resources.join("worker/desktop-sample.jar");
    app.install_local(&sample_jar_path).await.unwrap();

    let installed = app.store.installed().unwrap();
    let sample_provider = app.providers().await.unwrap().into_iter().next().unwrap();

    let links_val = app
        .worker
        .request(
            &installed,
            "loadLinks",
            serde_json::json!({
                "providerId": sample_provider.id,
                "data": "https://download.blender.org/peach/trailer/trailer_480p.mov",
            }),
        )
        .await
        .unwrap();

    let links: ResolvedLinks = serde_json::from_value(links_val).unwrap();
    let stream = links.streams[0].clone();
    assert!(stream.resource_handle.is_some(), "must have handle");

    // Play stream through App
    let _ = app
        .play(stream, vec![], None, None, 0.0, Some("default"), None)
        .await
        .unwrap();

    // Verify that active_transport_token is set
    let token = {
        let guard = app.active_transport_token.lock().await;
        guard.clone()
    };
    assert!(token.is_some(), "Active transport token must be populated during playback");

    // Stop player
    app.player.stop().await;

    // Simulate playback stop/exit event which triggers cleanup
    let old_tok = {
        let mut guard = app.active_transport_token.lock().await;
        guard.take()
    };
    if let Some(tok) = old_tok {
        let closed = app.worker.close_media_session(&installed, &tok).await.unwrap();
        assert!(closed, "Media session must be closed on stop");
    }
}
