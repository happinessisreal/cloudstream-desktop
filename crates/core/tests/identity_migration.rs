use cloudstream_core::{
    models::*,
    repository::plugin_id,
    store::Store,
};
use std::collections::BTreeMap;

#[test]
fn unicode_and_colon_containing_provider_names_and_urls() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("unicode.db")).unwrap();

    let provider = "custom:provider:name:日本語:v2";
    let url = "https://example.com/anime/日本語/title?id=42:foo&param=bar";
    let unique_url = Some("anime:unique:123:日本語");

    let media_id = compute_media_id(provider, url, unique_url);
    assert!(!media_id.is_empty());
    assert!(media_id.starts_with(provider));

    let ep = Episode {
        name: "第1話 · 始まり".to_string(),
        data: "{\"streamUrl\":\"https://cdn.example/video?auth=secret:token:123\",\"meta\":{\"jp\":\"テスト\"}}".to_string(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: Some(DubStatus::Subbed),
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep_id = compute_episode_id(&media_id, &ep, None, None);
    assert!(ep_id.contains("subbed"));
    assert!(ep_id.contains("s1e1"));

    // Ensure we can store and retrieve with colons and Unicode
    let media = MediaItem {
        provider_id: provider.to_string(),
        name: "アニメ タイトル".to_string(),
        url: url.to_string(),
        poster_url: None,
        poster_headers: BTreeMap::new(),
        media_type: Some("Anime".to_string()),
        year: Some(2024),
        unique_url: unique_url.map(|s| s.to_string()),
        media_id: Some(media_id.clone()),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let watch = WatchEntry {
        id: ep_id.clone(),
        item: media.clone(),
        episode: ep.clone(),
        position: 154.5,
        duration: 1420.0,
        watched_at: timestamp_ms(),
    };

    store.save_watch_entry(&watch, "default").unwrap();

    let retrieved = store.get_watch_progress("default", &ep_id).unwrap();
    assert!(retrieved.is_some());
    let prog = retrieved.unwrap();
    assert_eq!(prog.position_seconds, 154.5);
    assert_eq!(prog.duration_seconds, Some(1420.0));
    assert_eq!(prog.media_id, media_id);
    assert_eq!(prog.episode_id, ep_id);
}

#[test]
fn changed_episode_token_retains_progress_and_updates_payload() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("tokens.db")).unwrap();

    let media_id = compute_media_id("sample:provider", "https://example.com/show/1", None);

    let ep_token_v1 = Episode {
        name: "Episode 1".to_string(),
        data: "https://stream.example/video.mp4?token=expiring_token_abc&expires=1000".to_string(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: Some(DubStatus::None),
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep_id_v1 = compute_episode_id(&media_id, &ep_token_v1, None, None);

    let item = MediaItem {
        provider_id: "sample:provider".to_string(),
        name: "Sample Show".to_string(),
        url: "https://example.com/show/1".to_string(),
        poster_url: None,
        poster_headers: BTreeMap::new(),
        media_type: Some("TvSeries".to_string()),
        year: Some(2023),
        unique_url: None,
        media_id: Some(media_id.clone()),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let watch_v1 = WatchEntry {
        id: ep_id_v1.clone(),
        item: item.clone(),
        episode: ep_token_v1.clone(),
        position: 450.0,
        duration: 1800.0,
        watched_at: timestamp_ms(),
    };

    // Save initial watch progress
    store.save_watch_entry(&watch_v1, "default").unwrap();

    // Verify initial payload stored
    let initial_payload = store.get_episode_payload(&ep_id_v1).unwrap();
    assert_eq!(initial_payload.as_deref(), Some(ep_token_v1.data.as_str()));

    // Now provider returns a refreshed token for the exact same episode
    let ep_token_v2 = Episode {
        name: "Episode 1".to_string(),
        data: "https://stream.example/video.mp4?token=refreshed_token_xyz&expires=9999".to_string(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: Some(DubStatus::None),
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep_id_v2 = compute_episode_id(&media_id, &ep_token_v2, None, None);

    // Episode ID MUST remain identical despite token change
    assert_eq!(
        ep_id_v1, ep_id_v2,
        "Episode identity must not depend on token payload"
    );

    // Updating payload with fresh token
    store
        .save_episode_payload(&ep_id_v2, &media_id, &ep_token_v2.data)
        .unwrap();

    // Verify watch progress was retained without being overwritten
    let progress = store
        .get_watch_progress("default", &ep_id_v2)
        .unwrap()
        .expect("Watch progress must exist");
    assert_eq!(
        progress.position_seconds, 450.0,
        "Position progress must be retained"
    );

    // Verify payload was updated to the fresh token
    let fresh_payload = store.get_episode_payload(&ep_id_v2).unwrap();
    assert_eq!(fresh_payload.as_deref(), Some(ep_token_v2.data.as_str()));

    // Retained data-hash alias resolves correctly
    let legacy_alias_v1 = plugin_id("sample:provider", &ep_token_v1.data);
    let resolved = store.resolve_alias(&legacy_alias_v1, "episode").unwrap();
    assert_eq!(resolved.as_deref(), Some(ep_id_v1.as_str()));
}

#[test]
fn same_numbered_episode_in_two_dub_groups_remains_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dub_groups.db")).unwrap();

    let media_id = compute_media_id("provider:anime", "https://example.com/series/hero", None);

    let ep_dubbed = Episode {
        name: "Origins".to_string(),
        data: "payload_dubbed".to_string(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: Some(DubStatus::Dubbed),
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep_subbed = Episode {
        name: "Origins".to_string(),
        data: "payload_subbed".to_string(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: Some(DubStatus::Subbed),
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let id_dubbed = compute_episode_id(&media_id, &ep_dubbed, None, None);
    let id_subbed = compute_episode_id(&media_id, &ep_subbed, None, None);

    assert_ne!(
        id_dubbed, id_subbed,
        "Dubbed and Subbed episode IDs must be distinct"
    );

    let item = MediaItem {
        provider_id: "provider:anime".to_string(),
        name: "Anime Hero".to_string(),
        url: "https://example.com/series/hero".to_string(),
        poster_url: None,
        poster_headers: BTreeMap::new(),
        media_type: Some("Anime".to_string()),
        year: Some(2022),
        unique_url: None,
        media_id: Some(media_id.clone()),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    // Watch dubbed to 600 seconds
    let watch_dubbed = WatchEntry {
        id: id_dubbed.clone(),
        item: item.clone(),
        episode: ep_dubbed,
        position: 600.0,
        duration: 1200.0,
        watched_at: timestamp_ms(),
    };
    store.save_watch_entry(&watch_dubbed, "default").unwrap();

    // Watch subbed to 150 seconds
    let watch_subbed = WatchEntry {
        id: id_subbed.clone(),
        item,
        episode: ep_subbed,
        position: 150.0,
        duration: 1200.0,
        watched_at: timestamp_ms(),
    };
    store.save_watch_entry(&watch_subbed, "default").unwrap();

    // Verify both exist independently and neither overwrote the other
    let prog_dubbed = store
        .get_watch_progress("default", &id_dubbed)
        .unwrap()
        .unwrap();
    let prog_subbed = store
        .get_watch_progress("default", &id_subbed)
        .unwrap()
        .unwrap();

    assert_eq!(prog_dubbed.position_seconds, 600.0);
    assert_eq!(prog_subbed.position_seconds, 150.0);
}

#[test]
fn movie_synthetic_episode_stays_stable() {
    let media_id = compute_media_id("provider:movies", "https://example.com/movie/test", None);

    let ep1 = Episode {
        name: "Test Movie".to_string(),
        data: "https://stream.example/film.m3u8?token=first".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: None,
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep2 = Episode {
        name: "Test Movie".to_string(),
        data: "https://stream.example/film.m3u8?token=second_auth".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: None,
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let id1 = compute_episode_id(&media_id, &ep1, None, None);
    let id2 = compute_episode_id(&media_id, &ep2, None, None);

    assert_eq!(id1, format!("{media_id}::movie"));
    assert_eq!(id1, id2, "Movie synthetic episode ID must stay stable");
}

#[test]
fn duplicate_and_unnumbered_episodes_never_overwrite_each_other() {
    let media_id = compute_media_id("provider:tv", "https://example.com/specials", None);

    // Three unnumbered bonus OVA episodes without numbers
    let ep0 = Episode {
        name: "Bonus Special".to_string(),
        data: "data_special_0".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: Some(0),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep1 = Episode {
        name: "Bonus Special".to_string(),
        data: "data_special_1".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: Some(1),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let ep2 = Episode {
        name: "Bonus Special".to_string(),
        data: "data_special_2".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: Some(2),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let id0 = compute_episode_id(&media_id, &ep0, None, Some(0));
    let id1 = compute_episode_id(&media_id, &ep1, None, Some(1));
    let id2 = compute_episode_id(&media_id, &ep2, None, Some(2));

    assert_ne!(id0, id1);
    assert_ne!(id1, id2);
    assert_ne!(id0, id2);
}

#[test]
fn missing_provider_does_not_delete_progress() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("missing_provider.db")).unwrap();

    let media_id = compute_media_id("uninstalled:provider", "https://example.com/film", None);
    let ep = Episode {
        name: "Movie".to_string(),
        data: "data".to_string(),
        season: None,
        episode: None,
        poster_url: None,
        id: None,
        dub_status: None,
        index: None,
        extra: BTreeMap::new(),
        ..Default::default()
    };
    let ep_id = compute_episode_id(&media_id, &ep, None, None);

    let item = MediaItem {
        provider_id: "uninstalled:provider".to_string(),
        name: "Orphaned Movie".to_string(),
        url: "https://example.com/film".to_string(),
        poster_url: None,
        poster_headers: BTreeMap::new(),
        media_type: Some("Movie".to_string()),
        year: Some(2021),
        unique_url: None,
        media_id: Some(media_id.clone()),
        extra: BTreeMap::new(),
        ..Default::default()
    };

    let watch = WatchEntry {
        id: ep_id.clone(),
        item,
        episode: ep,
        position: 120.0,
        duration: 5400.0,
        watched_at: timestamp_ms(),
    };

    store.save_watch_entry(&watch, "default").unwrap();

    // Verify history lists the entry even though provider is not installed
    let history = store.history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, ep_id);
    assert_eq!(history[0].item.provider_id, "uninstalled:provider");

    // Progress is intact
    let progress = store.get_watch_progress("default", &ep_id).unwrap();
    assert!(progress.is_some());
}
