//! Scheduled backups (T26): fake-clock scheduling, restart catch-up, retention that only touches
//! owned files, and failure handling that never harms earlier backups.
use cloudstream_core::{backup::*, backup_schedule::*, models::*, App};
use serde_json::json;
use std::{os::unix::fs::PermissionsExt, path::Path, sync::{atomic::{AtomicU64, Ordering}, Arc}};

struct FakeClock(AtomicU64);
impl FakeClock {
    fn at(ms: u64) -> Self { Self(AtomicU64::new(ms)) }
    fn set(&self, ms: u64) { self.0.store(ms, Ordering::SeqCst); }
}
impl Clock for FakeClock { fn now_ms(&self) -> u64 { self.0.load(Ordering::SeqCst) } }

const HOUR: u64 = 3_600_000;
const T0: u64 = 1_793_986_000_000; // 2026-11-06 17:26:40 UTC

fn open(data: &Path) -> Arc<App> { App::open(data.join("data"), data.join("resources")).unwrap() }

fn enable(app: &App, folder: &Path, frequency: &str, retention: u32) {
    app.store.update_settings(json!({ "backup": { "enabled": true, "directory": folder.display().to_string(), "frequency": frequency, "retention": retention } }), Some("global"), None).unwrap();
}

fn owned(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| is_owned_backup_name(n)).collect();
    names.sort();
    names
}

#[test]
fn due_time_is_a_pure_function_of_settings_status_and_clock() {
    let mut settings = BackupSettings { enabled: true, directory: Some("/backups".into()), frequency: BackupFrequency::Daily, retention: 7, extra: Default::default() };
    let mut status = BackupStatus::default();
    assert_eq!(due(&BackupSettings::default(), &status, T0), Due::Disabled);
    assert_eq!(due(&settings, &status, T0), Due::Now, "nothing has ever run");
    status.last_attempt_ms = Some(T0);
    status.last_success_ms = Some(T0);
    assert_eq!(due(&settings, &status, T0 + 24 * HOUR - 1), Due::NotYet(T0 + 24 * HOUR));
    assert_eq!(due(&settings, &status, T0 + 24 * HOUR), Due::Now);
    settings.frequency = BackupFrequency::Weekly;
    assert_eq!(due(&settings, &status, T0 + 6 * 24 * HOUR), Due::NotYet(T0 + 7 * 24 * HOUR));
    // A failure after the last success retries after the back-off, not after a whole interval.
    status.last_attempt_ms = Some(T0 + 24 * HOUR);
    status.last_error = Some("The backup folder is not writable".into());
    assert_eq!(due(&settings, &status, T0 + 24 * HOUR + RETRY_BACKOFF_MS - 1), Due::NotYet(T0 + 24 * HOUR + RETRY_BACKOFF_MS));
    assert_eq!(due(&settings, &status, T0 + 24 * HOUR + RETRY_BACKOFF_MS), Due::Now);
}

#[tokio::test]
async fn one_backup_per_interval_and_a_restart_neither_repeats_nor_loops() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("backups");
    std::fs::create_dir(&folder).unwrap();
    let clock = FakeClock::at(T0);
    let app = open(dir.path());
    app.store.create_profile(CreateProfileInput { name: "Kids".into(), avatar: None, pin: Some("4321".into()) }).unwrap();
    enable(&app, &folder, "daily", 7);

    let Tick::Ran(first) = app.backup_tick(&clock).await.unwrap() else { panic!("the first backup is due immediately") };
    assert!(first.file.ends_with("cloudstream-auto-backup-20261106-172640.json"), "{}", first.file);
    assert_eq!(owned(&folder).len(), 1);
    // The file is a complete, verifiable backup and holds no PIN material.
    let text = std::fs::read_to_string(&first.file).unwrap();
    assert!(!text.contains("pin_hash") && !text.contains("4321"));
    let file = parse_backup(text.as_bytes(), u32::MAX).unwrap();
    assert!(matches!(file, ParsedBackup::Desktop(_)));
    let status = app.backup_status_with_next(T0).unwrap();
    assert_eq!((status.last_success_ms, status.next_due_ms, status.last_error.clone()), (Some(T0), Some(T0 + 24 * HOUR), None));

    // Time passes within the interval: nothing happens, however often it is checked.
    for hours in [1, 5, 23] {
        clock.set(T0 + hours * HOUR);
        assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::NotYet(_)));
    }
    assert_eq!(owned(&folder).len(), 1);

    // Restart the app: the saved status prevents an immediate repeat.
    drop(app);
    let app = open(dir.path());
    clock.set(T0 + 23 * HOUR);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::NotYet(_)));

    // The interval passes: exactly one more backup, however many times it is checked.
    clock.set(T0 + 24 * HOUR);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::Ran(_)));
    for _ in 0..3 {
        assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::NotYet(_)));
    }
    assert_eq!(owned(&folder).len(), 2);
}

#[tokio::test]
async fn backups_missed_while_the_app_was_closed_catch_up_once() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("backups");
    std::fs::create_dir(&folder).unwrap();
    let clock = FakeClock::at(T0);
    let app = open(dir.path());
    enable(&app, &folder, "daily", 7);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::Ran(_)));
    drop(app);
    // Closed for five days, then started again.
    let app = open(dir.path());
    clock.set(T0 + 5 * 24 * HOUR + 3 * HOUR);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::Ran(_)), "catch-up runs at startup");
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::NotYet(_)), "but only once, not once per missed day");
    assert_eq!(owned(&folder).len(), 2);
}

#[tokio::test]
async fn retention_removes_only_old_owned_backups_after_a_new_one_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("backups");
    std::fs::create_dir(&folder).unwrap();
    // Older scheduler backups, plus things that merely look similar or belong to the user.
    for name in ["cloudstream-auto-backup-20260101-000000.json", "cloudstream-auto-backup-20260102-000000.json", "cloudstream-auto-backup-20260103-000000.json"] {
        std::fs::write(folder.join(name), b"old").unwrap();
    }
    std::fs::write(folder.join("holiday-photos.json"), b"mine").unwrap();
    std::fs::write(folder.join("cloudstream-auto-backup-notes.json"), b"mine").unwrap();
    std::fs::write(folder.join("cloudstream-backup-2026-11-06.json"), b"manual").unwrap();
    std::fs::create_dir(folder.join("cloudstream-auto-backup-20250101-000000.json")).unwrap();
    let precious = dir.path().join("precious.txt");
    std::fs::write(&precious, b"do not delete").unwrap();
    std::os::unix::fs::symlink(&precious, folder.join("cloudstream-auto-backup-20250202-000000.json")).unwrap();

    let clock = FakeClock::at(T0);
    let app = open(dir.path());
    enable(&app, &folder, "daily", 2);
    let Tick::Ran(run) = app.backup_tick(&clock).await.unwrap() else { panic!("expected a backup") };
    // Keeping two means the new backup and the newest old one; only the two oldest go.
    assert_eq!(run.pruned, vec!["cloudstream-auto-backup-20260101-000000.json".to_string(), "cloudstream-auto-backup-20260102-000000.json".to_string()]);
    let remaining = owned(&folder);
    assert_eq!(remaining.len(), 4, "two newest owned backups plus the directory and the symlink that only look owned: {remaining:?}");
    assert!(remaining.contains(&"cloudstream-auto-backup-20260103-000000.json".to_string()));
    assert!(remaining.contains(&"cloudstream-auto-backup-20261106-172640.json".to_string()));
    for kept in ["holiday-photos.json", "cloudstream-auto-backup-notes.json", "cloudstream-backup-2026-11-06.json"] {
        assert!(folder.join(kept).exists(), "{kept} is not ours");
    }
    assert!(folder.join("cloudstream-auto-backup-20250101-000000.json").is_dir());
    assert_eq!(std::fs::read(&precious).unwrap(), b"do not delete", "a symlink is never followed");
    assert!(std::fs::symlink_metadata(folder.join("cloudstream-auto-backup-20250202-000000.json")).unwrap().file_type().is_symlink());
}

#[tokio::test]
async fn an_unwritable_or_missing_destination_fails_cleanly_and_backs_off() {
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("SKIPPED: permission checks do not apply to root");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("backups");
    std::fs::create_dir(&folder).unwrap();
    let clock = FakeClock::at(T0);
    let app = open(dir.path());
    enable(&app, &folder, "daily", 7);
    let Tick::Ran(good) = app.backup_tick(&clock).await.unwrap() else { panic!("expected a backup") };
    let good_bytes = std::fs::read(&good.file).unwrap();

    // The folder becomes read-only; the previous backup is untouched and the failure is explained.
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    clock.set(T0 + 24 * HOUR);
    let Tick::Failed(message) = app.backup_tick(&clock).await.unwrap() else { panic!("expected a failure") };
    assert!(message.contains("not writable"), "{message}");
    assert_eq!(std::fs::read(&good.file).unwrap(), good_bytes);
    let status = app.backup_status().unwrap();
    assert!(status.last_error.as_deref().is_some_and(|e| e.contains("not writable")));
    assert_eq!(status.last_success_ms, Some(T0), "a failure does not look like a success");

    // No retry storm: nothing for 15 minutes, then one more attempt.
    clock.set(T0 + 24 * HOUR + RETRY_BACKOFF_MS - 1);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::NotYet(_)));
    clock.set(T0 + 24 * HOUR + RETRY_BACKOFF_MS);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::Failed(_)));

    // Fixing the folder lets the next attempt succeed and clears the error.
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    clock.set(T0 + 24 * HOUR + 2 * RETRY_BACKOFF_MS);
    assert!(matches!(app.backup_tick(&clock).await.unwrap(), Tick::Ran(_)));
    assert_eq!(app.backup_status().unwrap().last_error, None);

    // A folder that disappeared is reported, not created behind the user's back.
    std::fs::remove_dir_all(&folder).unwrap();
    clock.set(T0 + 3 * 24 * HOUR);
    let Tick::Failed(message) = app.backup_tick(&clock).await.unwrap() else { panic!("expected a failure") };
    assert!(message.contains("not available"), "{message}");
    assert!(!folder.exists());
}

#[tokio::test]
async fn a_backup_in_progress_blocks_a_second_one_and_manual_runs_are_serialised() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("backups");
    std::fs::create_dir(&folder).unwrap();
    let clock = FakeClock::at(T0);
    let app = open(dir.path());
    enable(&app, &folder, "daily", 7);
    let guard = app.backup_lock.lock().await;
    assert_eq!(app.backup_tick(&clock).await.unwrap(), Tick::Busy);
    drop(guard);
    // Two "back up now" requests in the same second get distinct files, never an overwrite.
    let (a, b) = tokio::join!(app.run_backup(&clock), app.run_backup(&clock));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.file, b.file);
    assert_eq!(owned(&folder).len(), 2);
}

#[test]
fn schedule_settings_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    let app = open(dir.path());
    let bad = |patch: serde_json::Value, needle: &str| {
        let error = app.store.update_settings(json!({ "backup": patch }), Some("global"), None).unwrap_err().to_string();
        assert!(error.contains(needle), "'{error}' should mention '{needle}'");
    };
    bad(json!({ "enabled": true }), "Choose a folder");
    bad(json!({ "directory": "relative/path" }), "full folder path");
    bad(json!({ "retention": 0 }), "between 1 and 100");
    bad(json!({ "retention": 101 }), "between 1 and 100");
    bad(json!({ "frequency": "hourly" }), "hourly");
    let ok = app.store.update_settings(json!({ "backup": { "enabled": true, "directory": "/tmp/backups", "frequency": "weekly", "retention": 30 } }), Some("global"), None).unwrap();
    assert_eq!((ok.global.backup.frequency, ok.global.backup.retention), (BackupFrequency::Weekly, 30));
    // A rejected change leaves the saved settings alone.
    assert_eq!(app.store.get_settings().unwrap().global.backup.retention, 30);
}
