mod support;
use cloudstream_core::{download_queue::{DownloadRequest, DownloadSourcePolicy, DownloadState, QueueLease}, models::{CreateProfileInput, Episode, MediaItem}, store::Store};
use std::path::Path;

fn request(folder: &Path, n: i32, data: &str) -> DownloadRequest {
    let item: MediaItem = serde_json::from_value(serde_json::json!({"providerId":"fixture", "name":"../A show", "url":"https://example.test/title"})).unwrap();
    DownloadRequest { item, episode: Episode { name: format!("Episode {n}"), season: Some(1), episode: Some(n), data: data.into(), ..Default::default() }, destination: folder.into(), extension: "mp4".into(), policy: DownloadSourcePolicy { source_name: Some("Source".into()), stream_name: None, quality: Some(720) } }
}

#[test]
fn queue_is_persistent_idempotent_profile_scoped_and_keeps_payloads_separate() {
    let dir = tempfile::tempdir().unwrap(); let folder = dir.path().join("selected downloads"); std::fs::create_dir(&folder).unwrap();
    let db = dir.path().join("app.db"); let store = Store::open(&db).unwrap();
    let profile = store.create_profile(CreateProfileInput { name: "Other".into(), avatar: None, pin: None }).unwrap();
    let original = store.enqueue_download("default", request(&folder, 1, "token-old")).unwrap();
    let duplicate = store.enqueue_download("default", request(&folder, 1, "token-new")).unwrap();
    assert_eq!(original.id, duplicate.id);
    assert_eq!(store.get_episode_payload(&original.episode_id).unwrap().as_deref(), Some("token-new"));
    let visible = serde_json::to_string(&original).unwrap();
    assert!(!visible.contains("token-old"));
    let metadata = store.get::<cloudstream_core::download_queue::DownloadMetadata>("download_metadata", &original.id).unwrap().unwrap();
    assert_eq!(metadata.item.media_id.as_deref(), Some(original.media_id.as_str()));
    assert_eq!(metadata.episode.id.as_deref(), Some(original.episode_id.as_str()));
    assert!(metadata.episode.data.is_empty(), "provider payload stays in its separate store");
    assert_eq!(store.enqueue_download(&profile.id, request(&folder, 1, "token-new")).unwrap().profile_id, profile.id);
    assert_eq!(store.download_jobs(Some("default")).unwrap().len(), 1);
    assert!(store.enqueue_download("missing", request(&folder, 2, "payload")).is_err());
    let mut invalid = request(&folder, 2, "payload"); invalid.extension = "../../mp4".into();
    assert!(store.enqueue_download("default", invalid).is_err());
    drop(store);
    let reopened = Store::open(&db).unwrap();
    let jobs = reopened.download_jobs(None).unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].state, DownloadState::Queued);
    assert_eq!(jobs[0].destination, std::fs::canonicalize(folder).unwrap());
    assert!(reopened.get::<cloudstream_core::download_queue::DownloadMetadata>("download_metadata", &original.id).unwrap().is_some());
}

#[test]
fn claims_enforce_global_limit_lease_recovery_and_stale_revision_protection() {
    let dir = tempfile::tempdir().unwrap(); let db = dir.path().join("app.db"); let store = Store::open(&db).unwrap();
    let initial_a = store.enqueue_download("default", request(dir.path(), 1, "a")).unwrap();
    let b = store.enqueue_download("default", request(dir.path(), 2, "b")).unwrap();
    let lease = QueueLease::acquire(dir.path()).unwrap();
    assert!(QueueLease::acquire(dir.path()).is_err());
    assert!(store.claim_next_download(&lease, 2).is_err());
    assert_eq!(store.recover_download_queue(&lease).unwrap(), 0);
    assert!(store.recover_download_queue(&lease).is_err());
    let a = store.claim_next_download(&lease, 1).unwrap().unwrap(); assert_eq!(a.id, initial_a.id);
    assert!(store.claim_next_download(&lease, 1).unwrap().is_none());
    let b_claimed = store.claim_next_download(&lease, 2).unwrap().unwrap(); assert_eq!(b_claimed.id, b.id);
    assert!(store.claim_next_download(&lease, 2).unwrap().is_none());
    let downloading = store.transition_download(&a.id, "default", a.revision, DownloadState::Downloading, None, None, 0, Some(100)).unwrap();
    let progress = store.update_download_progress(&a.id, "default", downloading.revision, 50, Some(100)).unwrap();
    let paused = store.transition_download(&a.id, "default", progress.revision, DownloadState::Paused, None, None, 50, Some(100)).unwrap();
    assert!(store.update_download_progress(&a.id, "default", progress.revision, 70, Some(100)).is_err());
    assert!(store.transition_download(&a.id, "default", paused.revision, DownloadState::Completed, None, None, 100, Some(100)).is_err());
    let canceled = store.transition_download(&a.id, "default", paused.revision, DownloadState::Canceled, None, None, 50, Some(100)).unwrap();
    assert!(store.transition_download(&a.id, "default", paused.revision, DownloadState::Queued, None, None, 50, Some(100)).is_err());
    let paused_again = store.transition_download(&a.id, "default", canceled.revision, DownloadState::Queued, None, None, 50, Some(100)).unwrap();
    let paused_again = store.transition_download(&a.id, "default", paused_again.revision, DownloadState::Paused, None, None, 50, Some(100)).unwrap();
    assert_eq!(paused_again.state, DownloadState::Paused);
    drop(lease);
    let lease = QueueLease::acquire(dir.path()).unwrap();
    assert_eq!(store.recover_download_queue(&lease).unwrap(), 1);
    let jobs = store.download_jobs(Some("default")).unwrap();
    assert_eq!(jobs.iter().find(|j| j.id == a.id).unwrap().state, DownloadState::Paused);
    assert_eq!(jobs.iter().find(|j| j.id == b.id).unwrap().state, DownloadState::Queued);
    assert!(store.transition_download(&b.id, "default", b_claimed.revision, DownloadState::Downloading, None, None, 0, None).is_err());
}

#[test]
fn reorder_is_profile_isolated_and_completion_requires_the_expected_file() {
    let dir = tempfile::tempdir().unwrap(); let store = Store::open(&dir.path().join("app.db")).unwrap();
    let profile = store.create_profile(CreateProfileInput { name: "Other".into(), avatar: None, pin: None }).unwrap();
    let a = store.enqueue_download("default", request(dir.path(), 1, "a")).unwrap();
    let other = store.enqueue_download(&profile.id, request(dir.path(), 2, "b")).unwrap();
    let c = store.enqueue_download("default", request(dir.path(), 3, "c")).unwrap();
    assert!(store.reorder_downloads("default", &[a.id.clone(), a.id.clone()]).is_err());
    store.reorder_downloads("default", &[c.id.clone(), a.id.clone()]).unwrap();
    let jobs = store.download_jobs(None).unwrap();
    assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), vec![c.id.as_str(), other.id.as_str(), a.id.as_str()]);
    let lease = QueueLease::acquire(dir.path()).unwrap(); store.recover_download_queue(&lease).unwrap();
    let claimed = store.claim_next_download(&lease, 1).unwrap().unwrap(); assert_eq!(claimed.id, c.id);
    let active = store.transition_download(&claimed.id, "default", claimed.revision, DownloadState::Downloading, None, None, 0, Some(4)).unwrap();
    let wrong = dir.path().join("user-file.mp4"); std::fs::write(&wrong, b"keep").unwrap();
    assert!(store.transition_download(&active.id, "default", active.revision, DownloadState::Completed, None, Some(&wrong), 4, Some(4)).is_err());
    let output = dir.path().join(format!("cloudstream-{}.mp4", active.id)); std::fs::write(&output, b"done").unwrap();
    assert!(store.transition_download(&active.id, "default", active.revision, DownloadState::Completed, None, Some(&output), 4, Some(4)).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"done");
    assert_eq!(std::fs::read(wrong).unwrap(), b"keep");
}

#[test]
fn deleted_profile_jobs_remain_owned_but_are_not_automatically_claimed() {
    let dir = tempfile::tempdir().unwrap(); let store = Store::open(&dir.path().join("app.db")).unwrap();
    let profile = store.create_profile(CreateProfileInput { name: "Other".into(), avatar: None, pin: None }).unwrap();
    let orphan = store.enqueue_download(&profile.id, request(dir.path(), 1, "payload")).unwrap();
    store.delete_profile(&profile.id).unwrap();
    let lease = QueueLease::acquire(dir.path()).unwrap(); store.recover_download_queue(&lease).unwrap();
    assert!(store.claim_next_download(&lease, 1).unwrap().is_none());
    assert_eq!(store.download_jobs(None).unwrap()[0].id, orphan.id);
}

#[tokio::test]
async fn completed_transfer_can_be_recorded_after_queue_restart() {
    use cloudstream_core::downloads::{transfer_direct, TransferResult};
    let dir = tempfile::tempdir().unwrap(); let db = dir.path().join("app.db");
    let body = support::wav(1.0);
    let server = support::serve(vec![("/video.wav", body.clone())]).await;
    let store = Store::open(&db).unwrap();
    let job = store.enqueue_download("default", request(dir.path(), 1, "opaque-payload")).unwrap();
    let lease = QueueLease::acquire(dir.path()).unwrap(); store.recover_download_queue(&lease).unwrap();
    let claimed = store.claim_next_download(&lease, 1).unwrap().unwrap();
    let active = store.transition_download(&job.id, "default", claimed.revision, DownloadState::Downloading, None, None, 0, None).unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let result = transfer_direct(&active.destination, active.destination_identity, &active.id, &active.extension, &active.episode_id, &server.url("/video.wav"), false, rx, |_| {}).await.unwrap();
    let TransferResult::Completed { path, bytes } = result else { panic!("expected completion") };
    assert_eq!(std::fs::read(&path).unwrap(), body);
    // Simulate exit after atomic publication but before the queue state was committed.
    drop(lease); drop(store);
    let store = Store::open(&db).unwrap(); let lease = QueueLease::acquire(dir.path()).unwrap();
    assert_eq!(store.recover_download_queue(&lease).unwrap(), 1);
    let claimed = store.claim_next_download(&lease, 1).unwrap().unwrap();
    let active = store.transition_download(&job.id, "default", claimed.revision, DownloadState::Downloading, None, None, bytes, Some(bytes)).unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let recovered = transfer_direct(&active.destination, active.destination_identity, &active.id, &active.extension, &active.episode_id, &server.url("/video.wav"), false, rx, |_| {}).await.unwrap();
    assert_eq!(recovered, TransferResult::Completed { path: path.clone(), bytes });
    let completed = store.transition_download(&job.id, "default", active.revision, DownloadState::Completed, None, Some(&path), bytes, Some(bytes)).unwrap();
    assert_eq!(completed.state, DownloadState::Completed);
    assert_eq!(store.get_episode_payload(&job.episode_id).unwrap().as_deref(), Some("opaque-payload"));
}

#[test]
fn queue_lease_cannot_be_used_for_another_database() {
    let one = tempfile::tempdir().unwrap(); let two = tempfile::tempdir().unwrap();
    let store = Store::open(&two.path().join("app.db")).unwrap();
    let lease = QueueLease::acquire(one.path()).unwrap();
    assert!(store.recover_download_queue(&lease).unwrap_err().to_string().contains("another database"));
    let correct = QueueLease::acquire(two.path()).unwrap();
    assert_eq!(store.recover_download_queue(&correct).unwrap(), 0);
}
