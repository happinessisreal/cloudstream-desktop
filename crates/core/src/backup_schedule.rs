//! Scheduled backups (T26).
//!
//! Lifetime, stated plainly: backups are made only while the app is running. A backup that came
//! due while the app was closed is made shortly after the next start (catch-up). Running while
//! the app is fully closed would need a separate opt-in user service and is not provided.
//!
//! The decision of *when* is a pure function of the settings, the recorded status and a `Clock`,
//! so it is testable with a fake clock. A failed attempt is recorded too and retried only after a
//! back-off, which is what keeps a broken destination from causing an export loop.
use crate::{backup::*, models::*, App};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Prefix of every file this scheduler creates. Retention only ever touches names that match
/// `cloudstream-auto-backup-YYYYMMDD-HHMMSS[-N].json`, so a user's other files are safe.
pub const BACKUP_PREFIX: &str = "cloudstream-auto-backup-";
/// After a failed attempt, wait this long before trying again.
pub const RETRY_BACKOFF_MS: u64 = 15 * 60_000;
const STATUS_KIND: &str = "backup_status";
const STATUS_ID: &str = "schedule";

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        timestamp_ms()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bytes: Option<u64>,
    /// Why the most recent attempt failed; cleared by the next success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// When the next scheduled attempt will happen, for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    Disabled,
    NotYet(u64),
    Now,
}

/// When is the next backup due? Pure.
pub fn due(settings: &BackupSettings, status: &BackupStatus, now_ms: u64) -> Due {
    if !settings.enabled || settings.directory.is_none() {
        return Due::Disabled;
    }
    let interval = settings.frequency.interval_ms();
    let next = match (status.last_success_ms, status.last_attempt_ms) {
        // A failed attempt after the last success: back off, do not wait a whole interval.
        (success, Some(attempt)) if status.last_error.is_some() && success.map_or(true, |s| attempt > s) => attempt + RETRY_BACKOFF_MS,
        (Some(success), _) => success + interval,
        (None, _) => 0,
    };
    if now_ms >= next {
        Due::Now
    } else {
        Due::NotYet(next)
    }
}

/// UTC civil time from epoch milliseconds (proleptic Gregorian), for file names.
pub fn civil_utc(ms: u64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

pub fn backup_file_name(ms: u64, suffix: u32) -> String {
    let (y, mo, d, h, mi, s) = civil_utc(ms);
    let tail = if suffix == 0 { String::new() } else { format!("-{suffix}") };
    format!("{BACKUP_PREFIX}{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}{tail}.json")
}

/// Is this exactly a name the scheduler would have created?
pub fn is_owned_backup_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(BACKUP_PREFIX).and_then(|r| r.strip_suffix(".json")) else {
        return false;
    };
    let mut parts = rest.splitn(3, '-');
    let (date, time, suffix) = (parts.next(), parts.next(), parts.next());
    let digits = |s: Option<&str>, n: usize| s.is_some_and(|s| s.len() == n && s.bytes().all(|b| b.is_ascii_digit()));
    digits(date, 8) && digits(time, 6) && suffix.map_or(true, |s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

/// Free bytes available to an unprivileged writer on the destination's file system.
fn available_bytes(directory: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(directory.as_os_str().as_bytes()).context("Invalid backup folder")?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Could not check free space in the backup folder");
    }
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

/// Leave room for the backup, its temporary copy and some headroom.
pub fn check_space(available: u64, body_len: usize) -> Result<()> {
    let needed = (body_len as u64).saturating_mul(2).saturating_add(1024 * 1024);
    if available < needed {
        bail!("Not enough free space in the backup folder: {} MiB free, about {} MiB needed", available / (1024 * 1024), needed.div_ceil(1024 * 1024));
    }
    Ok(())
}

fn check_destination(directory: &Path) -> Result<()> {
    let metadata = std::fs::metadata(directory).with_context(|| format!("The backup folder {} is not available", directory.display()))?;
    if !metadata.is_dir() {
        bail!("{} is not a folder", directory.display());
    }
    // A real write test: permissions alone do not say whether a network or read-only mount works.
    let probe = directory.join(format!(".cloudstream-write-test-{}", uuid::Uuid::new_v4().simple()));
    let result = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).and_then(|f| f.sync_all());
    let _ = std::fs::remove_file(&probe);
    result.with_context(|| format!("The backup folder {} is not writable", directory.display()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupRun {
    pub file: String,
    pub bytes: u64,
    pub pruned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tick {
    Disabled,
    NotYet(u64),
    /// Another backup (manual or scheduled) was already running.
    Busy,
    Ran(BackupRun),
    Failed(String),
}

impl App {
    pub fn backup_status(&self) -> Result<BackupStatus> {
        Ok(self.store.get::<BackupStatus>(STATUS_KIND, STATUS_ID)?.unwrap_or_default())
    }

    fn save_backup_status(&self, status: &BackupStatus) -> Result<()> {
        self.store.put(STATUS_KIND, STATUS_ID, status)
    }

    /// Status for the UI, including when the next scheduled backup will happen.
    pub fn backup_status_with_next(&self, now_ms: u64) -> Result<BackupStatus> {
        let settings = self.store.get_settings()?.global.backup;
        let mut status = self.backup_status()?;
        status.next_due_ms = match due(&settings, &status, now_ms) {
            Due::Disabled => None,
            Due::NotYet(at) => Some(at),
            Due::Now => Some(now_ms),
        };
        Ok(status)
    }

    /// Make one scheduled-style backup right now (also used by "Back up now"). Serialised with
    /// scheduled runs so two exports never race for the same folder.
    pub async fn run_backup(&self, clock: &dyn Clock) -> Result<BackupRun> {
        let _guard = self.backup_lock.lock().await;
        self.run_backup_locked(clock).await
    }

    async fn run_backup_locked(&self, clock: &dyn Clock) -> Result<BackupRun> {
        let now = clock.now_ms();
        let settings = self.store.get_settings()?.global.backup;
        let outcome = self.write_scheduled_backup(&settings, now);
        let mut status = self.backup_status()?;
        status.last_attempt_ms = Some(now);
        match &outcome {
            Ok(run) => {
                status.last_success_ms = Some(now);
                status.last_file = Some(run.file.clone());
                status.last_bytes = Some(run.bytes);
                status.last_error = None;
            }
            // Only a fixed, readable message is stored; it never carries file contents.
            Err(error) => {
                status.last_error = Some(format!("{error:#}"));
                self.diagnostics.record("backup", &format!("{error:#}"));
            }
        }
        self.save_backup_status(&status)?;
        outcome
    }

    fn write_scheduled_backup(&self, settings: &BackupSettings, now: u64) -> Result<BackupRun> {
        settings.validate()?;
        let directory = PathBuf::from(settings.directory.as_deref().context("Choose a folder for scheduled backups first")?);
        check_destination(&directory)?;

        let file = self.store.export_backup(&ExportOptions::default(), env!("CARGO_PKG_VERSION"))?;
        let body = serde_json::to_vec_pretty(&file)?;
        check_space(available_bytes(&directory)?, body.len())?;

        let mut suffix = 0;
        let target = loop {
            let candidate = directory.join(backup_file_name(now, suffix));
            if std::fs::symlink_metadata(&candidate).is_err() {
                break candidate;
            }
            suffix += 1;
            if suffix > 1000 {
                bail!("Too many backups with the same timestamp in the backup folder");
            }
        };
        write_backup_atomically(&target, &file)?;

        // A backup only counts once it can be read back and passes its own checksum.
        let verified = std::fs::read(&target)
            .context("The new backup could not be read back")
            .and_then(|bytes| parse_backup(&bytes, u32::MAX).map(|_| bytes.len() as u64));
        let bytes = match verified {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = std::fs::remove_file(&target);
                return Err(error.context("The new backup failed verification and was discarded; your earlier backups were not touched"));
            }
        };

        // Only now, with a good new backup on disk, are old owned backups pruned.
        let pruned = prune_owned_backups(&directory, settings.retention as usize, &target);
        Ok(BackupRun { file: target.display().to_string(), bytes, pruned })
    }

    /// One scheduler step: back up if (and only if) one is due.
    pub async fn backup_tick(&self, clock: &dyn Clock) -> Result<Tick> {
        let settings = self.store.get_settings()?.global.backup;
        let status = self.backup_status()?;
        match due(&settings, &status, clock.now_ms()) {
            Due::Disabled => return Ok(Tick::Disabled),
            Due::NotYet(at) => return Ok(Tick::NotYet(at)),
            Due::Now => {}
        }
        let Ok(_guard) = self.backup_lock.try_lock() else {
            return Ok(Tick::Busy);
        };
        // Re-check under the lock: a manual run may have just finished.
        let status = self.backup_status()?;
        if due(&settings, &status, clock.now_ms()) != Due::Now {
            return Ok(Tick::NotYet(0));
        }
        Ok(match self.run_backup_locked(clock).await {
            Ok(run) => Tick::Ran(run),
            Err(error) => Tick::Failed(format!("{error:#}")),
        })
    }

    /// Check on startup and every minute while the app runs.
    pub fn start_backup_scheduler(app: &Arc<App>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(app);
        tokio::spawn(async move {
            // Give the app a moment to finish starting before the catch-up backup.
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            loop {
                let Some(app) = weak.upgrade() else { return };
                let _ = app.backup_tick(&SystemClock).await;
                drop(app);
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        })
    }
}

/// Delete the oldest scheduler-owned backups beyond `keep`. Never deletes `protect`, anything
/// that is not a regular file, or any name the scheduler did not create. Returns removed names.
pub fn prune_owned_backups(directory: &Path, keep: usize, protect: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut owned: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_owned_backup_name(&name) {
                return None;
            }
            let meta = std::fs::symlink_metadata(entry.path()).ok()?;
            // Regular files owned by this user only: never follow or remove links or directories.
            (meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() }).then(|| (name, entry.path()))
        })
        .collect();
    // Names sort chronologically (UTC timestamp), newest last.
    owned.sort_by(|a, b| a.0.cmp(&b.0));
    let mut removed = Vec::new();
    let excess = owned.len().saturating_sub(keep.max(1));
    for (name, path) in owned.into_iter().take(excess) {
        if path == protect {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed.push(name);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_time_matches_known_instants() {
        assert_eq!(civil_utc(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_utc(951_782_400_000), (2000, 2, 29, 0, 0, 0), "leap day");
        assert_eq!(civil_utc(1_793_986_000_000), (2026, 11, 6, 17, 26, 40));
        assert_eq!(backup_file_name(1_793_986_000_000, 0), "cloudstream-auto-backup-20261106-172640.json");
        assert_eq!(backup_file_name(1_793_986_000_000, 2), "cloudstream-auto-backup-20261106-172640-2.json");
    }

    #[test]
    fn low_disk_space_is_reported_before_anything_is_written() {
        check_space(10 * 1024 * 1024, 1024).unwrap();
        let error = check_space(1024 * 1024, 4 * 1024 * 1024).unwrap_err().to_string();
        assert!(error.contains("Not enough free space") && error.contains("1 MiB free") && error.contains("9 MiB needed"), "{error}");
        assert!(check_space(0, 0).is_err(), "even an empty backup needs headroom");
    }

    #[test]
    fn only_exact_owned_names_match() {
        assert!(is_owned_backup_name("cloudstream-auto-backup-20261106-172640.json"));
        assert!(is_owned_backup_name("cloudstream-auto-backup-20261106-172640-12.json"));
        for other in [
            "cloudstream-auto-backup-20261106-172640.json.bak",
            "cloudstream-auto-backup-2026110-172640.json",
            "cloudstream-auto-backup-20261106-17264.json",
            "cloudstream-auto-backup-20261106-172640-.json",
            "cloudstream-auto-backup-20261106-172640-x.json",
            "my-cloudstream-auto-backup-20261106-172640.json",
            "cloudstream-backup-2026-11-06.json",
            "notes.json",
        ] {
            assert!(!is_owned_backup_name(other), "{other}");
        }
    }
}
