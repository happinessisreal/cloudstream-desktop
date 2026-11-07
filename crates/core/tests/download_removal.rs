//! Removing a download deletes only what the job owns and never follows or replaces user data.
mod support;
use cloudstream_core::downloads::{remove_job_assets, transfer_direct, verify_completed_file, DestinationIdentity, TransferResult};
use std::{os::fd::AsRawFd, path::{Path, PathBuf}};
use tokio::sync::watch;

fn job_dir(root: &Path, id: &str) -> PathBuf { root.join(format!(".cloudstream-{id}")) }

async fn complete(root: &Path, server: &support::Fixture) -> (String, PathBuf) {
    let id = uuid::Uuid::new_v4().to_string();
    let (_tx, rx) = watch::channel(false);
    let result = transfer_direct(root, DestinationIdentity::from_path(root).unwrap(), &id, "mp4", "logical", &server.url("/v.wav"), false, rx, |_| {}).await.unwrap();
    let TransferResult::Completed { path, .. } = result else { panic!("expected completion") };
    (id, path)
}

#[tokio::test]
async fn removing_a_finished_download_deletes_its_file_and_temporary_folder() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    assert!(path.exists() && job_dir(dir.path(), &id).exists());
    let report = remove_job_assets(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", true).unwrap();
    assert!(report.file_deleted && report.temp_cleaned, "{report:?}");
    assert!(report.notes.is_empty(), "{report:?}");
    assert!(!path.exists() && !job_dir(dir.path(), &id).exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing may be left in the user's folder");
}

#[tokio::test]
async fn removing_the_record_only_keeps_the_user_file() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let report = remove_job_assets(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", false).unwrap();
    assert!(!report.file_deleted && report.temp_cleaned);
    assert!(path.exists(), "the downloaded file belongs to the user");
    assert!(!job_dir(dir.path(), &id).exists());
}

#[tokio::test]
async fn a_replaced_or_edited_file_is_never_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    // The user replaces the file with their own, same name.
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"my own file").unwrap();
    let report = remove_job_assets(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", true).unwrap();
    assert!(!report.file_deleted);
    assert!(report.notes.iter().any(|n| n.contains("left in place")), "{report:?}");
    assert_eq!(std::fs::read(&path).unwrap(), b"my own file");
    assert!(report.temp_cleaned);
}

#[tokio::test]
async fn same_size_in_place_edits_are_detected_and_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(verify_completed_file(dir.path(), identity, &id, "mp4", bytes.len() as u64).unwrap());
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(!verify_completed_file(dir.path(), identity, &id, "mp4", bytes.len() as u64).unwrap());
    let report = remove_job_assets(dir.path(), identity, &id, "mp4", true).unwrap();
    assert!(!report.file_deleted && report.temp_cleaned);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[tokio::test]
async fn legacy_downloads_remain_playable_but_are_not_deleted_without_a_content_hash() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let marker = job_dir(dir.path(), &id).join("completed.json");
    let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    record.as_object_mut().unwrap().remove("file_sha256");
    std::fs::write(&marker, serde_json::to_vec(&record).unwrap()).unwrap();
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    assert!(verify_completed_file(dir.path(), identity, &id, "mp4", std::fs::metadata(&path).unwrap().len()).unwrap());
    let report = remove_job_assets(dir.path(), identity, &id, "mp4", true).unwrap();
    assert!(!report.file_deleted && report.temp_cleaned);
    assert!(path.exists());
}

#[tokio::test]
async fn unexpected_items_and_symlinks_in_the_temporary_folder_survive() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let victim = dir.path().join("precious.txt");
    std::fs::write(&victim, b"keep me").unwrap();
    std::os::unix::fs::symlink(&victim, job_dir(dir.path(), &id).join("resume.json")).unwrap();
    std::fs::write(job_dir(dir.path(), &id).join("notes.txt"), b"x").unwrap();
    let report = remove_job_assets(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", true).unwrap();
    assert!(report.file_deleted && !report.temp_cleaned, "{report:?}");
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me", "a symlink must never be followed");
    assert!(job_dir(dir.path(), &id).join("notes.txt").exists());
    assert!(job_dir(dir.path(), &id).join("resume.json").symlink_metadata().is_ok());
    assert!(!path.exists());
}

#[tokio::test]
async fn a_running_transfer_holds_the_job_lock_and_blocks_removal() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let lock = std::fs::OpenOptions::new().read(true).write(true).open(job_dir(dir.path(), &id).join("lock")).unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
    let error = remove_job_assets(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", true).unwrap_err();
    assert!(error.to_string().contains("already running"), "{error}");
    assert!(path.exists(), "a blocked removal must change nothing");
    drop(lock);
}

#[tokio::test]
async fn a_replaced_download_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0))]).await;
    let (id, path) = complete(dir.path(), &server).await;
    let other = tempfile::tempdir().unwrap();
    let wrong = DestinationIdentity::from_path(other.path()).unwrap();
    let error = remove_job_assets(dir.path(), wrong, &id, "mp4", true).unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
    assert!(path.exists());
}

#[tokio::test]
async fn removing_a_paused_download_discards_its_partial_data() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(20.0))]).await;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = watch::channel(false);
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let result = transfer_direct(dir.path(), identity, &id, "mp4", "logical", &server.url("/v.wav"), false, rx, move |p| {
        if p.bytes >= 1024 { let _ = tx.send(true); }
    }).await.unwrap();
    assert!(matches!(result, TransferResult::Paused { .. }));
    assert!(job_dir(dir.path(), &id).join("video.part").exists());
    let report = remove_job_assets(dir.path(), identity, &id, "mp4", true).unwrap();
    assert!(!report.file_deleted && report.temp_cleaned, "{report:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
