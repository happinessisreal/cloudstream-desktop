//! Direct-file transfer primitive used by the download queue.
//! Partial data lives in a private per-job directory under an opened destination directory.
use anyhow::{bail, Context, Result};
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{File, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, os::{fd::AsRawFd, unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt}}, path::{Path, PathBuf}, time::Duration};
use tokio::{io::AsyncWriteExt, sync::watch};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
enum Validator { Etag(String), LastModified(String) }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartialMetadata { version: u32, source_hash: String, validator: Option<Validator>, total: Option<u64> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompletionMetadata {
    pub(crate) version: u32,
    pub(crate) source_hash: String,
    pub(crate) name: String,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) bytes: u64,
    /// Present for folder outputs: the hash of the manifest that lists every file this job owns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) manifest_sha256: Option<String>,
    /// SHA-256 of a finished direct file; absent on older downloads and folder outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferProgress { pub bytes: u64, pub total: Option<u64> }

#[derive(Debug)]
struct RetryableTransfer(&'static str);
impl std::fmt::Display for RetryableTransfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}
impl std::error::Error for RetryableTransfer {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferResult { Completed { path: PathBuf, bytes: u64 }, Paused { bytes: u64 } }

/// Persist alongside a job to detect a destination folder replaced between app launches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DestinationIdentity { pub device: u64, pub inode: u64 }
impl DestinationIdentity {
    pub fn from_path(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path).context("Download folder is unavailable")?;
        if !metadata.is_dir() { bail!("Choose a download folder"); }
        Ok(Self { device: metadata.dev(), inode: metadata.ino() })
    }
}

/// A path anchored to an open directory descriptor. On Linux `/proc/self/fd` prevents a renamed
/// or replaced directory path from redirecting subsequent writes elsewhere.
pub(crate) struct Destination { _root: File, _lock: File, pub(crate) anchored: PathBuf, pub(crate) job: PathBuf, pub(crate) final_path: PathBuf }

/// The extension that marks a job whose output is a folder of local HLS files.
pub const HLS_EXTENSION: &str = "hls";

pub fn supported_direct_extension(extension: &str) -> bool {
    matches!(extension, "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ts" | "video")
}

impl Destination {
    pub(crate) fn open(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str) -> Result<Self> {
        if !supported_direct_extension(extension) { bail!("Unsupported direct video format"); }
        Self::open_any(root, identity, job_id, extension)
    }
    /// A segmented (HLS) download publishes a folder named `cloudstream-<id>.hls`.
    pub(crate) fn open_hls(root: &Path, identity: DestinationIdentity, job_id: &str) -> Result<Self> {
        Self::open_any(root, identity, job_id, HLS_EXTENSION)
    }
    fn open_any(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str) -> Result<Self> {
        let id = uuid::Uuid::parse_str(job_id).context("Invalid download identifier")?;
        if id.to_string() != job_id { bail!("Invalid download identifier"); }
        let display = std::fs::canonicalize(root).context("Download folder is unavailable")?;
        let root_file = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&display).context("Could not open the download folder")?;
        let actual = root_file.metadata()?;
        if actual.dev() != identity.device || actual.ino() != identity.inode { bail!("The download folder has changed. Choose it again"); }
        let anchored = PathBuf::from(format!("/proc/self/fd/{}", root_file.as_raw_fd()));
        let job = anchored.join(format!(".cloudstream-{job_id}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&job) {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&job)?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
                    bail!("The download's temporary folder is unsafe");
                }
            },
            Err(e) => return Err(e).context("Could not create the download's temporary folder"),
        }
        let lock = OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(job.join("lock"))?;
        if !lock.metadata()?.is_file() || lock.metadata()?.nlink() != 1 { bail!("The download lock is unsafe"); }
        // The lock can be held for a moment after its owner closed it: a process forked elsewhere in
        // the app carries a copy of every open descriptor until it execs. Wait briefly before
        // concluding that the download is genuinely running.
        let mut locked = false;
        for attempt in 0..40 {
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 { locked = true; break; }
            if attempt < 39 { std::thread::sleep(Duration::from_millis(10)); }
        }
        if !locked { bail!("This download is already running"); }
        let final_path = display.join(format!("cloudstream-{job_id}.{extension}"));
        Ok(Self { _root: root_file, _lock: lock, anchored, job, final_path })
    }
    pub(crate) fn part(&self) -> PathBuf { self.job.join("video.part") }
    pub(crate) fn metadata(&self) -> PathBuf { self.job.join("resume.json") }
    pub(crate) fn completion(&self) -> PathBuf { self.job.join("completed.json") }
    pub(crate) fn destination(&self) -> PathBuf { self.anchored.join(self.final_path.file_name().unwrap()) }
}

fn open_part(path: &Path) -> Result<File> {
    let f = OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)
        .context("Could not open the partial download")?;
    if !f.metadata()?.is_file() { bail!("The partial download is not a regular file"); }
    Ok(f)
}

fn hash_file(mut file: File) -> Result<(String, std::fs::Metadata)> {
    let before = file.metadata()?;
    if !before.is_file() { bail!("The downloaded file is not a regular file"); }
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let after = file.metadata()?;
    if before.dev() != after.dev() || before.ino() != after.ino() || before.len() != after.len()
        || before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec() {
        bail!("The downloaded file changed while it was being checked");
    }
    Ok((format!("{:x}", hasher.finalize()), after))
}

fn hash_matches(path: &Path, marker: &CompletionMetadata) -> Result<bool> {
    let Some(expected) = marker.file_sha256.as_deref() else { return Ok(false) };
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) {
        Ok(file) => file,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.dev() != marker.device || metadata.ino() != marker.inode || metadata.len() != marker.bytes { return Ok(false); }
    let (actual, _) = hash_file(file)?;
    Ok(actual == expected)
}

pub(crate) fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("Could not read download resume information"),
    };
    let mut bytes = Vec::new(); file.take(16_385).read_to_end(&mut bytes)?;
    if bytes.len() > 16_384 { bail!("Download resume information is invalid"); }
    Ok(serde_json::from_slice(&bytes).ok())
}

pub(crate) fn write_record<T: Serialize>(path: &Path, metadata: &T) -> Result<()> {
    let parent = path.parent().context("Invalid resume path")?;
    let temp = parent.join(format!(".resume-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&temp)?;
        file.write_all(&serde_json::to_vec(metadata)?)?; file.sync_all()?;
        std::fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temp.exists() { let _ = std::fs::remove_file(temp); }
    result.context("Could not save download resume information")
}

fn response_validator(headers: &header::HeaderMap) -> Option<Validator> {
    if let Some(tag) = headers.get(header::ETAG) {
        let tag = tag.to_str().ok()?.trim();
        if !tag.starts_with("W/") && tag.starts_with('"') && tag.ends_with('"') && tag.len() >= 2 && tag.len() <= 1024 {
            return Some(Validator::Etag(tag.into()));
        }
        // An HTTP-date If-Range is not permitted when an entity tag is present.
        return None;
    }
    let modified = headers.get(header::LAST_MODIFIED)?.to_str().ok()?.trim();
    let date = headers.get(header::DATE)?.to_str().ok()?.trim();
    let modified_time = httpdate::parse_http_date(modified).ok()?;
    let date_time = httpdate::parse_http_date(date).ok()?;
    if date_time.duration_since(modified_time).ok()? < Duration::from_secs(60) { return None; }
    Some(Validator::LastModified(modified.into()))
}

fn validator_value(validator: &Validator) -> &str { match validator { Validator::Etag(v) | Validator::LastModified(v) => v } }
fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    if start > end || end >= total { return None; }
    Some((start, end, total))
}
fn unsatisfied_total(value: &str) -> Option<u64> { value.strip_prefix("bytes */")?.parse().ok() }

fn looks_like_manifest_or_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let text = prefix.trim_start_matches('\u{feff}').trim_start().to_ascii_lowercase();
    text.starts_with("#extm3u") || text.starts_with("<!doctype html") || text.starts_with("<html")
        || (text.starts_with("<?xml") && text.contains("<mpd"))
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop { if *receiver.borrow() { return; } if receiver.changed().await.is_err() { std::future::pending::<()>().await; } }
}

/// Stream a single HTTP file. No credentials are saved in the resume record; callers own the
/// resource session and may supply a fresh transport URL after restart. `source_identity` must
/// identify the same logical resource across renewed URLs. A pause keeps only resumable bytes.
#[allow(clippy::too_many_arguments)]
pub async fn transfer_direct<F>(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str, source_identity: &str, url: &str,
    no_proxy: bool, mut pause: watch::Receiver<bool>, mut progress: F) -> Result<TransferResult>
where F: FnMut(TransferProgress) {
    if source_identity.is_empty() || source_identity.len() > 4096 { bail!("Invalid download source identity"); }
    let parsed = crate::repository::network_url(url)?;
    if parsed.path().to_ascii_lowercase().ends_with(".m3u8") { bail!("HLS playlists require the HLS download engine"); }
    let dest = Destination::open(root, identity, job_id, extension)?;
    let original_hash = crate::models::digest_string(source_identity);
    let mut marker_matches = false;
    if let Ok(published) = std::fs::symlink_metadata(dest.destination()) {
        let marker = read_record::<CompletionMetadata>(&dest.completion())?;
        marker_matches = marker.as_ref().is_some_and(|marker| marker.version == 1 && marker.source_hash == original_hash
            && dest.final_path.file_name().and_then(|n| n.to_str()) == Some(marker.name.as_str())
            && published.is_file() && !published.file_type().is_symlink() && published.dev() == marker.device
            && published.ino() == marker.inode && published.len() == marker.bytes && marker.bytes > 0);
        if marker_matches && marker.as_ref().is_some_and(|marker| marker.file_sha256.is_some()) {
            let marker = marker.expect("the matching marker exists");
            let path = dest.destination();
            marker_matches = tokio::task::spawn_blocking(move || hash_matches(&path, &marker)).await.context("Could not check the completed download")??;
        }
        if std::fs::symlink_metadata(dest.part()).is_err() {
            if marker_matches { return Ok(TransferResult::Completed { path: dest.final_path.clone(), bytes: published.len() }); }
            bail!("The destination already exists");
        }
    }
    let mut file = open_part(&dest.part())?;
    let partial_stat = file.metadata()?;
    let metadata = read_record::<PartialMetadata>(&dest.metadata())?.filter(|m| m.version == 2 && m.source_hash == original_hash);
    if partial_stat.nlink() != 1 {
        // A durable completion marker can survive deletion of resume metadata across a crash.
        let published = std::fs::symlink_metadata(dest.destination()).ok().is_some_and(|m|
            m.is_file() && !m.file_type().is_symlink() && m.dev() == partial_stat.dev() && m.ino() == partial_stat.ino());
        if !published || (!marker_matches && metadata.as_ref().is_none_or(|m| m.total.is_some_and(|total| total != partial_stat.len()))) {
            bail!("The partial download has an unsafe hard link");
        }
        return complete(&dest, file, partial_stat.len(), &original_hash).await;
    }
    if std::fs::symlink_metadata(dest.destination()).is_ok() { bail!("The destination already exists"); }
    let mut offset = file.metadata()?.len();
    if metadata.as_ref().is_none_or(|m| m.validator.is_none() || m.total.is_some_and(|total| offset > total)) {
        file.set_len(0)?; offset = 0;
    }
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 || crate::repository::network_url(attempt.url().as_str()).is_err() { attempt.error("Invalid download redirect") } else { attempt.follow() }
    });
    let mut builder = Client::builder().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30)).redirect(policy);
    if no_proxy { builder = builder.no_proxy(); }
    let client = builder.build()?;
    let mut validator = metadata.as_ref().and_then(|m| m.validator.clone());
    let mut forced_restart = false;
    loop {
        if *pause.borrow() { file.sync_all()?; return Ok(TransferResult::Paused { bytes: offset }); }
        let mut request = client.get(parsed.clone()).header(header::ACCEPT_ENCODING, "identity");
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
            request = request.header(header::IF_RANGE, validator_value(validator.as_ref().context("Missing download validator")?));
        }
        let mut response = tokio::select! {
            _ = cancelled(&mut pause) => { file.sync_all()?; return Ok(TransferResult::Paused { bytes: offset }); },
            response = request.send() => response.map_err(|_| RetryableTransfer("Could not connect to the download source"))?,
        };
        if response.status().is_server_error() || response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(RetryableTransfer("The download source is temporarily unavailable").into());
        }
        let received_validator = response_validator(response.headers());
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
            let total = response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(unsatisfied_total);
            if total == Some(offset) && received_validator == validator {
                return complete(&dest, file, offset, &original_hash).await;
            }
            if forced_restart { bail!("The server rejected the download range"); }
            file.set_len(0)?; file.seek(SeekFrom::Start(0))?; offset = 0; validator = None; forced_restart = true; continue;
        }
        if response.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity")) {
            bail!("The download source returned an unsupported content encoding");
        }
        let content_type = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if matches!(content_type.as_str(), "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "audio/mpegurl" | "audio/x-mpegurl" | "application/dash+xml" | "text/html") { bail!("The source is not a direct video file"); }
        let total = if response.status() == StatusCode::PARTIAL_CONTENT && offset > 0 {
            let range = response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(content_range);
            match range {
                Some((start, end, total)) if start == offset && received_validator == validator
                    && response.content_length().is_none_or(|length| length == end - start + 1)
                    && metadata.as_ref().and_then(|m| m.total).is_none_or(|previous| previous == total) => Some(total),
                _ => {
                    if forced_restart { bail!("The server returned an invalid download range"); }
                    file.set_len(0)?; file.seek(SeekFrom::Start(0))?; offset = 0; validator = None; forced_restart = true; continue;
                }
            }
        } else if response.status() == StatusCode::OK {
            if offset > 0 { file.set_len(0)?; file.seek(SeekFrom::Start(0))?; offset = 0; }
            validator = received_validator;
            response.content_length()
        } else { bail!("Download source returned HTTP {}", response.status().as_u16()); };
        let record = PartialMetadata { version: 2, source_hash: original_hash.clone(), validator, total };
        write_record(&dest.metadata(), &record)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut file = tokio::fs::File::from_std(file);
        progress(TransferProgress { bytes: offset, total });
        let mut prefix = Vec::new();
        let mut prefix_checked = offset > 0;
        loop {
            let next = tokio::select! {
                _ = cancelled(&mut pause) => { file.sync_all().await?; return Ok(TransferResult::Paused { bytes: offset }); },
                chunk = response.chunk() => chunk.map_err(|_| RetryableTransfer("The download connection was interrupted"))?,
            };
            let Some(chunk) = next else { break };
            let buffered = u64::try_from(prefix.len()).context("Download size overflow")?;
            let next_offset = offset.checked_add(buffered).and_then(|value| value.checked_add(chunk.len() as u64)).context("Download size overflow")?;
            if total.is_some_and(|size| next_offset > size) { bail!("The server sent more data than declared"); }
            if !prefix_checked {
                prefix.extend_from_slice(&chunk);
                if prefix.len() < 1024 { continue; }
                if looks_like_manifest_or_html(&prefix) { bail!("The source is not a direct video file"); }
                file.write_all(&prefix).await.context("Could not write the download; check available disk space")?;
                offset += prefix.len() as u64;
                prefix.clear(); prefix_checked = true;
            } else {
                file.write_all(&chunk).await.context("Could not write the download; check available disk space")?;
                offset = next_offset;
            }
            progress(TransferProgress { bytes: offset, total });
        }
        if !prefix_checked && !prefix.is_empty() {
            if looks_like_manifest_or_html(&prefix) { bail!("The source is not a direct video file"); }
            file.write_all(&prefix).await.context("Could not write the download; check available disk space")?;
            offset += prefix.len() as u64;
            progress(TransferProgress { bytes: offset, total });
        }
        file.sync_all().await?;
        let file = file.into_std().await;
        if total.is_some_and(|size| size != offset) { return Err(RetryableTransfer("The download ended before all bytes arrived").into()); }
        if offset == 0 { bail!("The downloaded file is empty"); }
        return complete(&dest, file, offset, &original_hash).await;
    }
}

/// Retry transient connection and server failures with bounded, pause-aware backoff. Non-network
/// errors such as invalid ranges, unsafe paths and disk failures return immediately.
#[allow(clippy::too_many_arguments)]
pub async fn transfer_with_retry<F>(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str, source_identity: &str, url: &str,
    no_proxy: bool, mut pause: watch::Receiver<bool>, max_attempts: u8, mut progress: F) -> Result<TransferResult>
where F: FnMut(TransferProgress) {
    if !(1..=5).contains(&max_attempts) { bail!("Download retry attempts must be between 1 and 5"); }
    for attempt in 0..max_attempts {
        match transfer_direct(root, identity, job_id, extension, source_identity, url, no_proxy, pause.clone(), &mut progress).await {
            Err(error) if error.downcast_ref::<RetryableTransfer>().is_some() && attempt + 1 < max_attempts => {
                let backoff = Duration::from_millis(250 * (1u64 << attempt));
                tokio::select! {
                    _ = cancelled(&mut pause) => {
                        // Reopen through the normal path to return the accurate partial byte count.
                        return transfer_direct(root, identity, job_id, extension, source_identity, url, no_proxy, pause, &mut progress).await;
                    },
                    _ = tokio::time::sleep(backoff) => {},
                }
            }
            result => return result,
        }
    }
    unreachable!()
}

/// What `remove_job_assets` did, so the caller can tell the user exactly what happened.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemovalReport { pub file_deleted: bool, pub temp_cleaned: bool, pub notes: Vec<String> }

/// Remove what one job owns: optionally its finished file, and its private temporary folder.
///
/// The job lock is taken first, so a running transfer is never disturbed. The finished file is
/// deleted only when its recorded inode, size, name and content hash match the completion marker.
/// Older files without a content hash and files moved, replaced or edited are left alone. Only
/// names this module creates are removed from the private folder, never symlinks or anything
/// unexpected, and the folder itself is removed only once empty.
pub fn remove_job_assets(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str, delete_output: bool) -> Result<RemovalReport> {
    let dest = Destination::open(root, identity, job_id, extension)?;
    let mut report = RemovalReport::default();
    if delete_output {
        match read_record::<CompletionMetadata>(&dest.completion())? {
            None => report.notes.push("No finished file is recorded for this download, so nothing was deleted.".into()),
            Some(marker) => {
                let expected_name = dest.final_path.file_name().and_then(|n| n.to_str());
                match std::fs::symlink_metadata(dest.destination()) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => report.notes.push("The finished file was already gone.".into()),
                    Err(e) => return Err(e).context("Could not inspect the finished file"),
                    Ok(published) => {
                        let ours = marker.version == 1 && expected_name == Some(marker.name.as_str()) && published.is_file()
                            && !published.file_type().is_symlink() && published.dev() == marker.device
                            && published.ino() == marker.inode && published.len() == marker.bytes;
                        if ours && marker.file_sha256.is_none() {
                            report.notes.push("This download predates content checks, so its file was left in place. Delete it manually if you no longer need it.".into());
                        } else if ours && hash_matches(&dest.destination(), &marker)? {
                            std::fs::remove_file(dest.destination()).context("Could not delete the finished file")?;
                            report.file_deleted = true;
                        } else {
                            report.notes.push("The file was moved, replaced or changed since it was downloaded, so it was left in place.".into());
                        }
                    }
                }
            }
        }
    }
    crate::direct_subtitles::cleanup(&dest, job_id, delete_output && report.file_deleted, &mut report.notes)?;
    for entry in std::fs::read_dir(&dest.job).context("Could not read the download's temporary folder")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let known = matches!(name.as_ref(), "lock" | "video.part" | "resume.json" | "completed.json" | "direct-subtitles.json") || name.starts_with(".resume-");
        let meta = std::fs::symlink_metadata(entry.path())?;
        if known && meta.is_file() && !meta.file_type().is_symlink() {
            std::fs::remove_file(entry.path())?;
        } else {
            report.notes.push("An unexpected item in the temporary folder was left in place.".into());
        }
    }
    match std::fs::remove_dir(&dest.job) {
        Ok(()) => report.temp_cleaned = true,
        Err(_) => report.notes.push("The temporary folder was not empty, so it was left in place.".into()),
    }
    Ok(report)
}

/// Confirm the recorded inode and content. Older jobs without a digest retain their previous
/// identity-and-size check; their content cannot be retroactively verified.
pub fn verify_completed_file(root: &Path, identity: DestinationIdentity, job_id: &str, extension: &str, bytes: u64) -> Result<bool> {
    let dest = Destination::open(root, identity, job_id, extension)?;
    let marker = read_record::<CompletionMetadata>(&dest.completion())?;
    let Some(marker) = marker else { return Ok(false) };
    let published = match std::fs::symlink_metadata(dest.destination()) { Ok(value) => value, Err(_) => return Ok(false) };
    let ours = marker.version == 1 && marker.source_hash.len() == 64 && marker.source_hash.bytes().all(|c| c.is_ascii_hexdigit())
        && dest.final_path.file_name().and_then(|n| n.to_str()) == Some(marker.name.as_str())
        && published.is_file() && !published.file_type().is_symlink() && published.dev() == marker.device
        && published.ino() == marker.inode && published.len() == marker.bytes && marker.bytes == bytes && bytes > 0;
    if !ours { return Ok(false); }
    if marker.file_sha256.is_some() { hash_matches(&dest.destination(), &marker) } else { Ok(true) }
}

async fn complete(dest: &Destination, file: File, bytes: u64, source_hash: &str) -> Result<TransferResult> {
    file.sync_all()?;
    let clone = file.try_clone()?;
    let (file_sha256, hashed) = tokio::task::spawn_blocking(move || hash_file(clone)).await.context("Could not check the downloaded file")??;
    if hashed.len() != bytes { bail!("The downloaded file changed unexpectedly"); }
    // Hard-link publication is atomic and refuses to replace a pre-existing user file.
    if let Err(error) = std::fs::hard_link(dest.part(), dest.destination()) {
        let partial = file.metadata()?;
        let already_published = error.kind() == std::io::ErrorKind::AlreadyExists && std::fs::symlink_metadata(dest.destination()).ok().is_some_and(|m|
            m.is_file() && !m.file_type().is_symlink() && m.dev() == partial.dev() && m.ino() == partial.ino());
        if !already_published { return Err(error).context("Could not publish the finished download; the destination may already exist"); }
    }
    File::open(&dest.anchored)?.sync_all()?;
    let published = std::fs::symlink_metadata(dest.destination())?;
    let partial = file.metadata()?;
    if !published.is_file() || published.file_type().is_symlink() || published.dev() != partial.dev() || published.ino() != partial.ino()
        || partial.dev() != hashed.dev() || partial.ino() != hashed.ino() || partial.len() != hashed.len()
        || partial.mtime() != hashed.mtime() || partial.mtime_nsec() != hashed.mtime_nsec() {
        bail!("The published download changed unexpectedly");
    }
    let marker = CompletionMetadata { version: 1, source_hash: source_hash.into(), name: dest.final_path.file_name().and_then(|name| name.to_str()).context("Invalid destination name")?.into(), device: published.dev(), inode: published.ino(), bytes, manifest_sha256: None, file_sha256: Some(file_sha256) };
    write_record(&dest.completion(), &marker)?;
    std::fs::remove_file(dest.part())?;
    let _ = std::fs::remove_file(dest.metadata());
    // Keep the locked inode in place so a concurrent retry cannot acquire a replacement lock.
    // Queue removal can clean the empty private directory when no task can still reference it.
    Ok(TransferResult::Completed { path: dest.final_path.clone(), bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_strong_etags_or_old_enough_last_modified_dates_are_used() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::LAST_MODIFIED, "Wed, 21 Oct 2015 07:28:00 GMT".parse().unwrap());
        headers.insert(header::DATE, "Wed, 21 Oct 2015 07:28:59 GMT".parse().unwrap());
        assert!(response_validator(&headers).is_none());
        headers.insert(header::DATE, "Wed, 21 Oct 2015 07:29:00 GMT".parse().unwrap());
        assert!(matches!(response_validator(&headers), Some(Validator::LastModified(_))));
        headers.insert(header::ETAG, "W/\"weak\"".parse().unwrap());
        assert!(response_validator(&headers).is_none());
        headers.insert(header::ETAG, "\"strong\"".parse().unwrap());
        assert_eq!(response_validator(&headers), Some(Validator::Etag("\"strong\"".into())));
    }
    #[test]
    fn content_ranges_reject_invalid_or_unspecified_bounds() {
        assert_eq!(content_range("bytes 20-99/100"), Some((20, 99, 100)));
        for invalid in ["bytes 20-100/100", "bytes 21-20/100", "bytes 0-9/*", "items 0-9/10", "bytes */100"] {
            assert_eq!(content_range(invalid), None);
        }
        assert_eq!(unsatisfied_total("bytes */100"), Some(100));
    }
}
