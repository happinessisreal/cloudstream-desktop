//! Persisted download job models and the exclusive queue-owner lease.
use crate::{downloads::DestinationIdentity, models::{Episode, MediaItem}};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs::{File, OpenOptions}, os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt}}, path::{Path, PathBuf}};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState { Queued, Resolving, Downloading, Paused, Completed, Failed, Canceled }

impl DownloadState {
    pub fn permits(self, next: Self) -> bool {
        use DownloadState::*;
        matches!((self, next),
            (Queued, Resolving | Paused | Canceled) |
            (Resolving, Downloading | Queued | Paused | Failed | Canceled) |
            (Downloading, Completed | Queued | Paused | Failed | Canceled) |
            (Paused | Failed | Canceled, Queued) |
            (Paused | Failed, Canceled))
    }
    pub fn active(self) -> bool { matches!(self, Self::Resolving | Self::Downloading) }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSourcePolicy {
    /// Optional preferred provider source and quality; fresh links are resolved when the job runs.
    pub source_name: Option<String>,
    #[serde(default)]
    pub stream_name: Option<String>,
    pub quality: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    pub item: MediaItem,
    pub episode: Episode,
    pub destination: PathBuf,
    pub extension: String,
    #[serde(default)]
    pub policy: DownloadSourcePolicy,
}

/// Private catalog metadata. The provider's episode payload is stored separately and is not
/// exposed in download-list responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadMetadata { pub item: MediaItem, pub episode: Episode }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    pub id: String,
    pub profile_id: String,
    pub media_id: String,
    pub episode_id: String,
    pub provider_id: String,
    pub title: String,
    pub episode_name: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub destination: PathBuf,
    pub destination_identity: DestinationIdentity,
    pub extension: String,
    pub policy: DownloadSourcePolicy,
    pub state: DownloadState,
    pub bytes: u64,
    pub total: Option<u64>,
    pub output: Option<PathBuf>,
    pub error: Option<String>,
    pub order: u64,
    pub revision: u64,
    /// The damaged completed job this job is replacing, if this is a repair copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_of: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// The holder is the only process allowed to recover and claim this app's download jobs.
/// The inode is retained across launches; deleting it while processes run would break exclusion.
pub struct QueueLease { _file: File, pub(crate) directory: PathBuf, pub(crate) recovered: std::sync::Mutex<bool> }
impl QueueLease {
    pub fn acquire(data_directory: &Path) -> Result<Self> {
        let directory = std::fs::canonicalize(data_directory).context("Download queue folder is unavailable")?;
        let path = directory.join("download-queue.lock");
        let file = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).context("Could not open the download queue lock")?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 || meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } { bail!("The download queue lock is unsafe"); }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { bail!("Another app instance owns the download queue"); }
        Ok(Self { _file: file, directory, recovered: std::sync::Mutex::new(false) })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadAction { Pause, Resume, Cancel, Retry }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DownloadEvent { JobChanged { job: DownloadJob }, QueueError { message: String } }
