use cloudstream_core::{models::*, App};
use serde_json::json;
use std::path::PathBuf;

fn encode_tracker(t: &str) -> String {
    url::form_urlencoded::byte_serialize(t.as_bytes()).collect()
}

#[tokio::test]
async fn verify_torrentio_loads_custom_trackers_into_magnet_links() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");

    let app = App::open(data_dir.clone(), resources).unwrap();

    // 1. Configure custom trackers with mixed case and trailing slashes
    let custom_tracker_1 = "udp://tracker.custom-test-alpha.org:1337/announce";
    let custom_tracker_2 = "udp://tracker.custom-test-beta.org:6969/announce";
    let trackers = vec![
        custom_tracker_1.to_string(),
        format!("{custom_tracker_1}/"), // duplicate with trailing slash
        "   ".to_string(),
        custom_tracker_2.to_string(),
        // Also include one default tracker with uppercase to verify preset deduplication
        "UDP://TRACKER.OPENTRACKR.ORG:1337/ANNOUNCE/".to_string(),
    ];
    app.save_custom_trackers(&trackers).unwrap();

    // Verify stored trackers at core level
    let saved = app.get_custom_trackers().unwrap();
    assert_eq!(saved.len(), 3, "Expected 3 unique canonical trackers");
    assert_eq!(saved[0], custom_tracker_1);
    assert_eq!(saved[1], custom_tracker_2);

    // 2. Install Torrentio extension
    let plugin = app.install_torrentio().await.unwrap();
    assert_eq!(plugin.entry.name, "Torrentio");

    let providers = app.providers().await.unwrap();
    let provider = providers
        .iter()
        .find(|p| p.name == "Torrentio")
        .expect("Torrentio provider should be registered");

    // 3. Resolve links for Big Buck Bunny (tt1254207)
    let links_val = app
        .call(
            "loadLinks",
            json!({
                "providerId": provider.id,
                "data": "https://torrentio.strem.fun/movie/tt1254207"
            }),
        )
        .await
        .unwrap();

    let resolved: ResolvedLinks = serde_json::from_value(links_val).unwrap();
    assert!(
        !resolved.streams.is_empty(),
        "Torrentio must resolve at least one stream"
    );

    let magnet_stream = resolved
        .streams
        .iter()
        .find(|s| s.url.starts_with("magnet:?"))
        .expect("Expected at least one magnet stream");

    let magnet_url = &magnet_stream.url;

    // Verify magnet link starts with standard bittorrent infohash
    assert!(magnet_url.contains("xt=urn:btih:"));

    // 4. Verify custom trackers actually got loaded into the magnet URI
    assert!(
        magnet_url.contains(&encode_tracker(custom_tracker_1)),
        "Magnet URI must contain encoded custom tracker 1. Found: {magnet_url}"
    );
    assert!(
        magnet_url.contains(&encode_tracker(custom_tracker_2)),
        "Magnet URI must contain encoded custom tracker 2. Found: {magnet_url}"
    );

    // 5. Verify preset deduplication: opentrackr must only appear once in &tr=
    let opentrackr_encoded = encode_tracker("udp://tracker.opentrackr.org:1337/announce");
    let opentrackr_matches = magnet_url.matches(&opentrackr_encoded).count();
    assert_eq!(
        opentrackr_matches, 1,
        "Preset tracker 'opentrackr.org' must appear exactly once in magnet link (no duplicate)"
    );

    // 6. Verify zero duplicate trackers across the entire magnet link
    let tr_parts: Vec<&str> = magnet_url.split("&tr=").skip(1).map(|t| t.split('&').next().unwrap()).collect();
    let unique_tr: std::collections::HashSet<&str> = tr_parts.iter().cloned().collect();
    assert_eq!(
        tr_parts.len(),
        unique_tr.len(),
        "Magnet URI must have zero duplicate trackers across all &tr= parameters"
    );

    // Clean up worker process
    app.worker.stop().await;
}
