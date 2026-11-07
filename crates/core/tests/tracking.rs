use cloudstream_core::{
    models::WatchStatus,
    tracking::{
        RemoteStatus, TrackingAccount, TrackingManager, TrackingService,
    },
};

#[test]
fn status_mapping_is_explicit_and_bidirectional() {
    assert_eq!(RemoteStatus::from_local(&WatchStatus::Watching), Some(RemoteStatus::Watching));
    assert_eq!(RemoteStatus::from_local(&WatchStatus::Completed), Some(RemoteStatus::Completed));
    assert_eq!(RemoteStatus::from_local(&WatchStatus::OnHold), Some(RemoteStatus::OnHold));
    assert_eq!(RemoteStatus::from_local(&WatchStatus::Dropped), Some(RemoteStatus::Dropped));
    assert_eq!(RemoteStatus::from_local(&WatchStatus::PlanToWatch), Some(RemoteStatus::PlanToWatch));
    assert_eq!(RemoteStatus::from_local(&WatchStatus::None), None);

    assert_eq!(RemoteStatus::Watching.to_local(), WatchStatus::Watching);
    assert_eq!(RemoteStatus::Rewatching.to_local(), WatchStatus::Watching);
    assert_eq!(RemoteStatus::Completed.to_local(), WatchStatus::Completed);
    assert_eq!(RemoteStatus::OnHold.to_local(), WatchStatus::OnHold);
    assert_eq!(RemoteStatus::Dropped.to_local(), WatchStatus::Dropped);
    assert_eq!(RemoteStatus::PlanToWatch.to_local(), WatchStatus::PlanToWatch);
}

#[test]
fn tracking_accounts_are_strictly_profile_scoped() {
    let manager = TrackingManager::new();

    let account1 = TrackingAccount {
        profile_id: "profile-a".into(),
        service: TrackingService::AniList,
        account_id: "user-123".into(),
        username: "Alice".into(),
        avatar_url: None,
        connected_at_ms: 1000,
    };

    let account2 = TrackingAccount {
        profile_id: "profile-b".into(),
        service: TrackingService::MyAnimeList,
        account_id: "user-456".into(),
        username: "Bob".into(),
        avatar_url: None,
        connected_at_ms: 2000,
    };

    manager.add_account(account1);
    manager.add_account(account2);

    let list_a = manager.list_accounts("profile-a");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].username, "Alice");
    assert_eq!(list_a[0].service, TrackingService::AniList);

    let list_b = manager.list_accounts("profile-b");
    assert_eq!(list_b.len(), 1);
    assert_eq!(list_b[0].username, "Bob");
    assert_eq!(list_b[0].service, TrackingService::MyAnimeList);

    let list_c = manager.list_accounts("profile-c");
    assert!(list_c.is_empty());

    manager.remove_account("profile-a", TrackingService::AniList);
    assert!(manager.list_accounts("profile-a").is_empty());
    assert_eq!(manager.list_accounts("profile-b").len(), 1);
}

#[test]
fn sync_mutations_are_idempotent_and_deduplicated() {
    let manager = TrackingManager::new();

    // Enqueue initial progress
    manager.queue_sync("profile-1", TrackingService::AniList, "media-1", "ext-100", 1, Some(RemoteStatus::Watching), None);
    let pending = manager.pending_mutations("profile-1");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].progress, 1);

    // Enqueue updated progress for the same target - should replace earlier pending mutation
    manager.queue_sync("profile-1", TrackingService::AniList, "media-1", "ext-100", 2, Some(RemoteStatus::Watching), Some(8.5));
    let pending_updated = manager.pending_mutations("profile-1");
    assert_eq!(pending_updated.len(), 1);
    assert_eq!(pending_updated[0].progress, 2);
    assert_eq!(pending_updated[0].score, Some(8.5));

    // Clear mutations for profile
    manager.clear_mutations("profile-1");
    assert!(manager.pending_mutations("profile-1").is_empty());
}
