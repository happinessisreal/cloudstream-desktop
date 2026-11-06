use cloudstream_core::{models::*, player::Player, App};
use std::collections::BTreeMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn make_sample_queue(current_index: usize) -> (PlaybackQueue, MediaItem, Vec<Episode>) {
    let media = MediaItem {
        provider_id: "sample_provider".into(),
        name: "Queue Series".into(),
        url: "https://example.com/series".into(),
        media_id: Some("sample_provider::https://example.com/series".into()),
        media_type: Some("TvSeries".into()),
        year: Some(2024),
        ..Default::default()
    };

    let mut episodes = Vec::new();
    for i in 1..=3 {
        episodes.push(Episode {
            name: format!("Episode {i}"),
            data: format!("ep_{i}_data"),
            season: Some(1),
            episode: Some(i),
            id: Some(format!("sample_provider::https://example.com/series::s1e{i}")),
            dub_status: Some(DubStatus::Subbed),
            ..Default::default()
        });
    }

    let queue = PlaybackQueue {
        profile_id: "default".into(),
        media: media.clone(),
        episodes: episodes.clone(),
        current_index,
        dub_status: Some(DubStatus::Subbed),
        season: Some(1),
        extra: BTreeMap::new(),
    };

    (queue, media, episodes)
}

#[tokio::test]
async fn queue_navigation_and_state_updates() {
    let temp_dir = tempfile::tempdir().unwrap();
    let player = Player::new(temp_dir.path().to_path_buf());

    let (queue, _, _) = make_sample_queue(0);
    player.set_queue(Some(queue)).await;

    // First episode
    let state = player.state().await;
    assert!(state.has_next);
    assert!(!state.has_prev);
    assert_eq!(state.queue_index, Some(0));
    assert_eq!(state.queue_count, Some(3));

    // Middle episode
    let (queue_mid, _, _) = make_sample_queue(1);
    player.set_queue(Some(queue_mid)).await;
    let state_mid = player.state().await;
    assert!(state_mid.has_next);
    assert!(state_mid.has_prev);
    assert_eq!(state_mid.queue_index, Some(1));
    assert_eq!(state_mid.queue_count, Some(3));

    // Last episode
    let (queue_last, _, _) = make_sample_queue(2);
    player.set_queue(Some(queue_last)).await;
    let state_last = player.state().await;
    assert!(!state_last.has_next);
    assert!(state_last.has_prev);
    assert_eq!(state_last.queue_index, Some(2));
    assert_eq!(state_last.queue_count, Some(3));

    // Clear queue
    player.set_queue(None).await;
    let state_none = player.state().await;
    assert!(!state_none.has_next);
    assert!(!state_none.has_prev);
    assert_eq!(state_none.queue_index, None);
    assert_eq!(state_none.queue_count, None);
}

#[tokio::test]
async fn completion_and_resume_threshold_policy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let app = App::open(temp_dir.path().join("data"), temp_dir.path().join("resources")).unwrap();

    let item = MediaItem {
        provider_id: "sample_provider".into(),
        name: "Threshold Movie".into(),
        url: "https://example.com/movie".into(),
        media_id: Some("sample_provider::https://example.com/movie".into()),
        ..Default::default()
    };
    let episode = Episode {
        name: "Full Feature".into(),
        data: "data".into(),
        id: Some("sample_provider::https://example.com/movie::ep1".into()),
        ..Default::default()
    };

    // 1. Position at 50% of 100s -> not watched
    let watch_half = WatchEntry {
        id: "sample_provider::https://example.com/movie::ep1".into(),
        item: item.clone(),
        episode: episode.clone(),
        position: 50.0,
        duration: 100.0,
        watched_at: 1000,
    };
    app.store.save_watch_entry(&watch_half, "default").unwrap();
    let prog = app.store.get_watch_progress("default", &watch_half.id).unwrap().unwrap();
    assert!(!prog.is_watched);

    // 2. Position at 92% of 100s (>= 90% default threshold) -> watched
    let watch_watched = WatchEntry {
        id: "sample_provider::https://example.com/movie::ep1".into(),
        item: item.clone(),
        episode: episode.clone(),
        position: 92.0,
        duration: 100.0,
        watched_at: 2000,
    };
    app.store.save_watch_entry(&watch_watched, "default").unwrap();
    let prog2 = app.store.get_watch_progress("default", &watch_watched.id).unwrap().unwrap();
    assert!(prog2.is_watched);

    // 3. User manual override to unwatched survives progress updates
    app.store.mark_episode_watched("default", &item.media_id.clone().unwrap(), &watch_watched.id, false).unwrap();
    let prog3 = app.store.get_watch_progress("default", &watch_watched.id).unwrap().unwrap();
    assert!(!prog3.is_watched);
    assert_eq!(prog3.watched_override, Some(false));

    // Even if position is 95%, manual override keeps it unwatched
    app.store.save_watch_entry(&watch_watched, "default").unwrap();
    let prog4 = app.store.get_watch_progress("default", &watch_watched.id).unwrap().unwrap();
    assert!(!prog4.is_watched);
}

#[tokio::test]
async fn auto_next_only_on_eof_and_profile_isolation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("autonext_test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        for _ in 0..6 {
            let _ = reader.next_line().await;
        }

        // Simulate normal EOF
        write_half
            .write_all(b"{\"event\":\"end-file\",\"reason\":\"eof\"}\n")
            .await
            .unwrap();
    });

    let app = App::open(temp_dir.path().join("data"), temp_dir.path().join("resources")).unwrap();
    let client_stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();

    let (queue, media, episodes) = make_sample_queue(0);
    app.player.set_queue(Some(queue)).await;

    let watch = WatchEntry {
        id: episodes[0].id.clone().unwrap(),
        item: media.clone(),
        episode: episodes[0].clone(),
        position: 100.0,
        duration: 100.0,
        watched_at: 1000,
    };

    app.player
        .start_session(
            "session-autonext".into(),
            "default".into(),
            None,
            socket_path.clone(),
            client_stream,
            Some(watch),
            100.0,
        )
        .await;

    server_task.await.unwrap();

    // Give background event listener time to process EndFile
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Episode 1 must be marked watched on normal EOF
    let prog = app.store.get_watch_progress("default", &episodes[0].id.clone().unwrap()).unwrap();
    assert!(prog.is_some());
    assert!(prog.unwrap().is_watched);

    // Profile switch clears player queue
    let new_prof = app.store.create_profile(CreateProfileInput {
        name: "Kids".into(),
        avatar: Some("default".into()),
        pin: None,
    }).unwrap();

    app.switch_profile(&new_prof.id, None).await.unwrap();
    assert!(app.player.queue().await.is_none());
}
