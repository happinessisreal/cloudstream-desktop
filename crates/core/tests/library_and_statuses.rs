use cloudstream_core::{
    models::{
        compute_episode_id, compute_media_id, DubStatus, Episode, MediaItem,
        WatchEntry, WatchStatus,
    },
    store::{execute_migrations, standard_migrations, Store},
};
use rusqlite::Connection;
use tempfile::tempdir;

fn sample_media_item(title: &str, provider: &str, path: &str) -> MediaItem {
    MediaItem {
        provider_id: provider.to_string(),
        url: format!("https://example.com/item/{path}"),
        name: title.to_string(),
        media_type: Some("TvSeries".into()),
        poster_url: Some(format!("https://example.com/poster/{path}.jpg")),
        poster_headers: Default::default(),
        year: Some(2024),
        score: Some(8.5),
        rating: None,
        quality: None,
        id: None,
        other_name: None,
        episodes_count: None,
        dub_status: None,
        dub_episodes: None,
        unique_url: None,
        media_id: None,
        extra: Default::default(),
    }
}

fn sample_episode(
    media_id: &str,
    name: &str,
    season: i32,
    episode: i32,
    index: i32,
    dub: DubStatus,
) -> Episode {
    let mut ep = Episode {
        name: name.to_string(),
        season: Some(season),
        episode: Some(episode),
        poster_url: None,
        id: None,
        dub_status: Some(dub),
        data: format!("data://{season}_{episode}_{index}"),
        index: Some(index),
        score: None,
        rating: None,
        description: None,
        date: None,
        run_time: None,
        extra: Default::default(),
    };
    let ep_id = compute_episode_id(media_id, &ep, Some(dub), Some(index as usize));
    ep.id = Some(ep_id);
    ep
}

#[test]
fn status_favorite_and_subscription_coexist_independently() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let store = Store::open(&db_path).unwrap();

    let item = sample_media_item("Show A", "provA", "show_a");
    let media_id = compute_media_id(&item.provider_id, &item.url, None);

    // 1. Initial set status to Watching
    let res1 = store
        .set_library_status(item.clone(), WatchStatus::Watching, Some("default"))
        .unwrap();
    assert_eq!(res1.watch_status, WatchStatus::Watching);
    assert!(!res1.is_favorite);
    assert!(!res1.is_subscribed);

    // 2. Toggle favorite to true; status must remain Watching
    let res2 = store
        .set_library_favorite(item.clone(), true, Some("default"))
        .unwrap();
    assert_eq!(res2.watch_status, WatchStatus::Watching);
    assert!(res2.is_favorite);
    assert!(!res2.is_subscribed);

    // 3. Toggle subscribed to true; status and favorite must remain
    let res3 = store
        .set_library_subscribed(item.clone(), true, Some("default"))
        .unwrap();
    assert_eq!(res3.watch_status, WatchStatus::Watching);
    assert!(res3.is_favorite);
    assert!(res3.is_subscribed);

    // 4. Change status to Completed; favorite and subscription must remain true
    let res4 = store
        .set_library_status(item.clone(), WatchStatus::Completed, Some("default"))
        .unwrap();
    assert_eq!(res4.watch_status, WatchStatus::Completed);
    assert!(res4.is_favorite);
    assert!(res4.is_subscribed);

    // 5. Verify retrieval from DB
    let retrieved = store.get_library_item(&media_id).unwrap().unwrap();
    assert_eq!(retrieved.watch_status, WatchStatus::Completed);
    assert!(retrieved.is_favorite);
    assert!(retrieved.is_subscribed);

    // 6. Untoggle favorite; status remains Completed and subscribed remains true
    let res5 = store
        .set_library_favorite(item.clone(), false, Some("default"))
        .unwrap();
    assert_eq!(res5.watch_status, WatchStatus::Completed);
    assert!(!res5.is_favorite);
    assert!(res5.is_subscribed);
}

#[test]
fn legacy_bookmarks_preserve_accessible_with_default_none_status() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("legacy.db");

    // Manually create schema up to v3 with a legacy bookmark JSON missing watch_status/favorite/subscribed
    {
        let mut conn = Connection::open(&db_path).unwrap();
        execute_migrations(&mut conn, &standard_migrations()[0..3]).unwrap();

        let legacy_json = r#"{
            "id": "prov1::hash123",
            "item": {
                "providerId": "prov1",
                "url": "https://example.com/legacy",
                "name": "Legacy Bookmark Title"
            },
            "savedAt": 1690000000000
        }"#;

        conn.execute(
            "INSERT INTO records (kind, id, value, updated) VALUES ('library', 'prov1::hash123', ?1, 1690000000000)",
            [legacy_json],
        )
        .unwrap();
    }

    // Now open with Store (which triggers migration 4)
    let store = Store::open(&db_path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 4);

    let items = store.library().unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.id, "prov1::hash123");
    assert_eq!(item.item.name, "Legacy Bookmark Title");
    assert_eq!(item.watch_status, WatchStatus::None);
    assert!(!item.is_favorite);
    assert!(!item.is_subscribed);
}

#[test]
fn watched_overrides_survive_resume_updates() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_overrides.db");
    let store = Store::open(&db_path).unwrap();

    let media = sample_media_item("Movie X", "provM", "movie_x");
    let media_id = compute_media_id(&media.provider_id, &media.url, None);
    let ep = sample_episode(&media_id, "Episode 1", 1, 1, 0, DubStatus::Subbed);
    let ep_id = ep.id.clone().unwrap();

    // 1. Manually mark episode as watched (position 0)
    let marked = store
        .mark_episode_watched("default", &media_id, &ep_id, true)
        .unwrap();
    assert!(marked.is_watched);
    assert_eq!(marked.watched_override, Some(true));

    // Verify stored
    let p1 = store.get_watch_progress("default", &ep_id).unwrap().unwrap();
    assert!(p1.is_watched);
    assert_eq!(p1.watched_override, Some(true));

    // 2. Playback resumes at position 10s of a 1000s duration (1% progress, would normally be unwatched)
    let entry = WatchEntry {
        id: ep_id.clone(),
        item: media.clone(),
        episode: ep.clone(),
        position: 10.0,
        duration: 1000.0,
        watched_at: 1700000000000,
    };
    store.save_watch_entry(&entry, "default").unwrap();

    // Verify that the manual override of 'watched' survived!
    let p2 = store.get_watch_progress("default", &ep_id).unwrap().unwrap();
    assert_eq!(p2.position_seconds, 10.0);
    assert_eq!(p2.duration_seconds, Some(1000.0));
    assert!(p2.is_watched, "Watched override must survive resume update");
    assert_eq!(p2.watched_override, Some(true));

    // 3. Now manually mark episode as unwatched
    let unmarked = store
        .mark_episode_watched("default", &media_id, &ep_id, false)
        .unwrap();
    assert!(!unmarked.is_watched);
    assert_eq!(unmarked.watched_override, Some(false));

    // 4. Playback resumes at position 950s of a 1000s duration (95% progress, would normally be watched)
    let entry2 = WatchEntry {
        id: ep_id.clone(),
        item: media.clone(),
        episode: ep.clone(),
        position: 950.0,
        duration: 1000.0,
        watched_at: 1700000050000,
    };
    store.save_watch_entry(&entry2, "default").unwrap();

    // Verify that the manual override of 'unwatched' survived!
    let p3 = store.get_watch_progress("default", &ep_id).unwrap().unwrap();
    assert_eq!(p3.position_seconds, 950.0);
    assert!(!p3.is_watched, "Unwatched override must survive resume update at 95% position");
    assert_eq!(p3.watched_override, Some(false));
}

#[test]
fn mark_episodes_up_to_respects_dub_group_and_deterministic_ordering() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_mark_up_to.db");
    let store = Store::open(&db_path).unwrap();

    let media = sample_media_item("Anime Series", "provA", "anime_series");
    let media_id = compute_media_id(&media.provider_id, &media.url, None);

    // Create episodes:
    // Sub: S1E1, S1E2, S1E3, S1E4
    // Dub: S1E1, S1E2, S1E3, S1E4
    let sub1 = sample_episode(&media_id, "Ep 1", 1, 1, 0, DubStatus::Subbed);
    let sub2 = sample_episode(&media_id, "Ep 2", 1, 2, 1, DubStatus::Subbed);
    let sub3 = sample_episode(&media_id, "Ep 3", 1, 3, 2, DubStatus::Subbed);
    let sub4 = sample_episode(&media_id, "Ep 4", 1, 4, 3, DubStatus::Subbed);

    let dub1 = sample_episode(&media_id, "Ep 1 Dub", 1, 1, 0, DubStatus::Dubbed);
    let dub2 = sample_episode(&media_id, "Ep 2 Dub", 1, 2, 1, DubStatus::Dubbed);
    let dub3 = sample_episode(&media_id, "Ep 3 Dub", 1, 3, 2, DubStatus::Dubbed);
    let dub4 = sample_episode(&media_id, "Ep 4 Dub", 1, 4, 3, DubStatus::Dubbed);

    // Unordered mix of episodes
    let all_episodes = vec![
        dub4.clone(),
        sub2.clone(),
        dub1.clone(),
        sub4.clone(),
        sub1.clone(),
        dub3.clone(),
        sub3.clone(),
        dub2.clone(),
    ];

    // Mark up to sub3 (Sub S1E3)
    let updated = store
        .mark_episodes_up_to(
            "default",
            &media_id,
            &sub3.id.clone().unwrap(),
            &all_episodes,
        )
        .unwrap();

    assert_eq!(updated.len(), 3); // sub1, sub2, sub3

    // Check Sub episodes:
    assert!(store
        .get_watch_progress("default", &sub1.id.unwrap())
        .unwrap()
        .unwrap()
        .is_watched);
    assert!(store
        .get_watch_progress("default", &sub2.id.unwrap())
        .unwrap()
        .unwrap()
        .is_watched);
    assert!(store
        .get_watch_progress("default", &sub3.id.unwrap())
        .unwrap()
        .unwrap()
        .is_watched);
    // sub4 must NOT be watched
    assert!(store
        .get_watch_progress("default", &sub4.id.unwrap())
        .unwrap()
        .is_none());

    // Check Dub episodes: NONE of the Dub episodes should be marked!
    assert!(store
        .get_watch_progress("default", &dub1.id.unwrap())
        .unwrap()
        .is_none());
    assert!(store
        .get_watch_progress("default", &dub2.id.unwrap())
        .unwrap()
        .is_none());
    assert!(store
        .get_watch_progress("default", &dub3.id.unwrap())
        .unwrap()
        .is_none());
    assert!(store
        .get_watch_progress("default", &dub4.id.unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn single_item_removal_and_confirmed_bulk_clear() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_clear.db");
    let store = Store::open(&db_path).unwrap();

    let item1 = sample_media_item("Item 1", "p1", "1");
    let item2 = sample_media_item("Item 2", "p1", "2");
    let item3 = sample_media_item("Item 3", "p1", "3");

    let l1 = store
        .set_library_status(item1.clone(), WatchStatus::Watching, None)
        .unwrap();
    let l2 = store
        .set_library_status(item2.clone(), WatchStatus::Completed, None)
        .unwrap();
    let l3 = store
        .set_library_status(item3.clone(), WatchStatus::Watching, None)
        .unwrap();

    assert_eq!(store.library().unwrap().len(), 3);

    // Remove single item (l2)
    store.remove_library_item(&l2.id).unwrap();
    assert_eq!(store.library().unwrap().len(), 2);
    assert!(store.get_library_item(&l2.id).unwrap().is_none());

    // Clear specific status (Watching)
    // l1 is also favorited
    store.set_library_favorite(item1.clone(), true, None).unwrap();
    store.clear_library(Some(WatchStatus::Watching)).unwrap();

    // l3 had only Watching status and no fav/sub, so it was deleted
    assert!(store.get_library_item(&l3.id).unwrap().is_none());
    // l1 was favorited, so its status reverted to None while remaining in library
    let l1_after = store.get_library_item(&l1.id).unwrap().unwrap();
    assert_eq!(l1_after.watch_status, WatchStatus::None);
    assert!(l1_after.is_favorite);

    // Full clear
    store.clear_library(None).unwrap();
    assert_eq!(store.library().unwrap().len(), 0);
}
