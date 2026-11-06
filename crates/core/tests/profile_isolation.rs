use cloudstream_core::{
    keyring::{FallbackKeyring, KeyringProvider, MockKeyring},
    models::*,
    App,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[test]
fn migration_creates_default_profile_and_migrates_unscoped_records() {
    let dir = tempfile::tempdir().unwrap();

    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    // Verify default profile was created by migration
    let profiles = app.list_profiles().unwrap();
    assert!(!profiles.is_empty());
    let default_prof = profiles.iter().find(|p| p.id == "default").unwrap();
    assert_eq!(default_prof.name, "Default Profile");
    assert!(!default_prof.has_pin);
}

#[test]
fn profile_crud_and_pin_protection() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    // 1. Create a profile without PIN
    let input1 = CreateProfileInput {
        name: "Kids".into(),
        avatar: Some("popcorn".into()),
        pin: None,
    };
    let prof1 = app.create_profile(input1).unwrap();
    assert_eq!(prof1.name, "Kids");
    assert_eq!(prof1.avatar, "popcorn");
    assert!(!prof1.has_pin);

    // Cannot create with empty name
    assert!(app
        .create_profile(CreateProfileInput {
            name: "   ".into(),
            avatar: None,
            pin: None,
        })
        .is_err());

    // 2. Create a profile with PIN
    let input2 = CreateProfileInput {
        name: "Parents".into(),
        avatar: Some("lock".into()),
        pin: Some("1234".into()),
    };
    let prof2 = app.create_profile(input2).unwrap();
    assert_eq!(prof2.name, "Parents");
    assert!(prof2.has_pin);

    // Verify PIN verification
    assert!(app.verify_profile_pin(&prof2.id, "1234").unwrap());
    assert!(!app.verify_profile_pin(&prof2.id, "0000").unwrap());

    // 3. Update profile PIN
    let update_input = UpdateProfileInput {
        name: Some("Adults".into()),
        avatar: None,
        pin: Some("5678".into()),
        remove_pin: None,
    };
    let updated = app.update_profile(&prof2.id, update_input).unwrap();
    assert_eq!(updated.name, "Adults");
    assert!(updated.has_pin);
    assert!(app.verify_profile_pin(&prof2.id, "5678").unwrap());
    assert!(!app.verify_profile_pin(&prof2.id, "1234").unwrap());

    // 4. Remove PIN
    let remove_pin_input = UpdateProfileInput {
        name: None,
        avatar: None,
        pin: None,
        remove_pin: Some(true),
    };
    let unpinned = app.update_profile(&prof2.id, remove_pin_input).unwrap();
    assert!(!unpinned.has_pin);
    assert!(app.verify_profile_pin(&prof2.id, "").unwrap());
}

#[test]
fn two_profiles_library_and_history_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    let alice = app
        .create_profile(CreateProfileInput {
            name: "Alice".into(),
            avatar: None,
            pin: None,
        })
        .unwrap();

    let bob = app
        .create_profile(CreateProfileInput {
            name: "Bob".into(),
            avatar: None,
            pin: None,
        })
        .unwrap();

    let item = MediaItem {
        provider_id: "prov".into(),
        name: "Shared Movie".into(),
        url: "https://example.com/shared-movie".into(),
        media_id: Some("shared_movie_id".into()),
        ..Default::default()
    };

    // Alice saves as Watching
    app.store
        .set_library_status(item.clone(), WatchStatus::Watching, Some(&alice.id))
        .unwrap();

    // Bob saves as Completed and Favorited
    app.store
        .set_library_status(item.clone(), WatchStatus::Completed, Some(&bob.id))
        .unwrap();
    app.store
        .set_library_favorite(item.clone(), true, Some(&bob.id))
        .unwrap();

    // Alice watches episode to 50s
    let watch_alice = WatchEntry {
        id: "shared_movie_id:Ep 1".into(),
        item: item.clone(),
        episode: Episode {
            name: "Ep 1".into(),
            data: "data1".into(),
            id: Some("shared_movie_id:Ep 1".into()),
            ..Default::default()
        },
        position: 50.0,
        duration: 3000.0,
        watched_at: timestamp_ms(),
    };
    app.store.save_watch_entry(&watch_alice, &alice.id).unwrap();

    // Bob watches episode to 1200s
    let watch_bob = WatchEntry {
        id: "shared_movie_id:Ep 1".into(),
        item: item.clone(),
        episode: Episode {
            name: "Ep 1".into(),
            data: "data1".into(),
            id: Some("shared_movie_id:Ep 1".into()),
            ..Default::default()
        },
        position: 1200.0,
        duration: 3000.0,
        watched_at: timestamp_ms(),
    };
    app.store.save_watch_entry(&watch_bob, &bob.id).unwrap();

    // 1. Verify Library isolation
    let alice_lib = app.store.library(Some(&alice.id)).unwrap();
    let bob_lib = app.store.library(Some(&bob.id)).unwrap();

    assert_eq!(alice_lib.len(), 1);
    assert_eq!(alice_lib[0].watch_status, WatchStatus::Watching);
    assert!(!alice_lib[0].is_favorite);

    assert_eq!(bob_lib.len(), 1);
    assert_eq!(bob_lib[0].watch_status, WatchStatus::Completed);
    assert!(bob_lib[0].is_favorite);

    // 2. Verify History isolation
    let alice_hist = app.store.history(Some(&alice.id)).unwrap();
    let bob_hist = app.store.history(Some(&bob.id)).unwrap();

    assert_eq!(alice_hist.len(), 1);
    assert_eq!(alice_hist[0].position, 50.0);

    assert_eq!(bob_hist.len(), 1);
    assert_eq!(bob_hist[0].position, 1200.0);

    // 3. Alice clears history: Bob's history is unaffected!
    app.clear_history(Some(&alice.id)).unwrap();
    assert!(app.store.history(Some(&alice.id)).unwrap().is_empty());
    assert_eq!(app.store.history(Some(&bob.id)).unwrap().len(), 1);

    // 4. Alice clears library: Bob's library is unaffected!
    app.clear_library(None, Some(&alice.id)).unwrap();
    assert!(app.store.library(Some(&alice.id)).unwrap().is_empty());
    assert_eq!(app.store.library(Some(&bob.id)).unwrap().len(), 1);

    // 5. Deleting Alice profile does not touch Bob
    app.delete_profile(&alice.id).unwrap();
    assert_eq!(app.store.library(Some(&bob.id)).unwrap().len(), 1);
    assert_eq!(app.store.history(Some(&bob.id)).unwrap().len(), 1);
}

#[tokio::test]
async fn late_player_events_during_profile_switch_save_to_original_profile() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    let alice = app
        .create_profile(CreateProfileInput {
            name: "Alice".into(),
            avatar: None,
            pin: None,
        })
        .unwrap();

    let bob = app
        .create_profile(CreateProfileInput {
            name: "Bob".into(),
            avatar: None,
            pin: None,
        })
        .unwrap();

    let socket_path = dir.path().join("late_event_test.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        for _ in 0..6 {
            let _ = reader.next_line().await;
        }

        // Alice starts watching: report position 45s
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":2,\"name\":\"duration\",\"data\":300.0}\n")
            .await
            .unwrap();
        write_half
            .write_all(b"{\"event\":\"property-change\",\"id\":1,\"name\":\"time-pos\",\"data\":45.0}\n")
            .await
            .unwrap();

        // Delay, then pause
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
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
        id: "series_url:Ep 1".into(),
        item: MediaItem {
            provider_id: "prov".into(),
            name: "Alice Series".into(),
            url: "series_url".into(),
            media_id: Some("alice_series_id".into()),
            ..Default::default()
        },
        episode: Episode {
            name: "Ep 1".into(),
            data: "data1".into(),
            id: Some("series_url:Ep 1".into()),
            ..Default::default()
        },
        position: 0.0,
        duration: 0.0,
        watched_at: 0,
    };

    // Start session explicitly under Alice
    app.player
        .start_session(
            "alice-session-1".into(),
            alice.id.clone(),
            None,
            socket_path,
            client_stream,
            Some(watch),
            0.0,
        )
        .await;

    // While playback is in flight, user switches to Bob!
    app.switch_profile(&bob.id, None).await.unwrap();

    server_task.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify: progress was saved to Alice!
    let alice_progress = app
        .store
        .get_watch_progress(&alice.id, "series_url:Ep 1")
        .unwrap();
    assert!(
        alice_progress.is_some(),
        "Late event must save progress to Alice"
    );
    assert!((alice_progress.unwrap().position_seconds - 45.0).abs() < 1.0);

    // Verify: Bob NEVER received Alice's progress!
    let bob_progress = app
        .store
        .get_watch_progress(&bob.id, "series_url:Ep 1")
        .unwrap();
    assert!(bob_progress.is_none(), "Bob must not receive Alice's progress");
}

#[test]
fn keyring_mock_and_fallback_behavior() {
    // 1. MockKeyring: in-memory secure storage
    let mock = MockKeyring::new();
    assert!(mock.is_available());
    assert!(mock.get_secret("cloudstream", "profile_1").unwrap().is_none());

    mock.set_secret("cloudstream", "profile_1", "super_secret_token")
        .unwrap();
    assert_eq!(
        mock.get_secret("cloudstream", "profile_1").unwrap().as_deref(),
        Some("super_secret_token")
    );

    mock.delete_secret("cloudstream", "profile_1").unwrap();
    assert!(mock.get_secret("cloudstream", "profile_1").unwrap().is_none());

    // 2. FallbackKeyring: non-strict allows storage, strict rejects with error
    let non_strict = FallbackKeyring::new(false);
    assert!(non_strict.is_available());
    non_strict
        .set_secret("cloudstream", "acct", "fallback_val")
        .unwrap();
    assert_eq!(
        non_strict.get_secret("cloudstream", "acct").unwrap().as_deref(),
        Some("fallback_val")
    );

    let strict = FallbackKeyring::new(true);
    assert!(!strict.is_available());
    let err = strict.set_secret("cloudstream", "acct", "val");
    assert!(err.is_err(), "Strict fallback must reject storing secrets");
}
