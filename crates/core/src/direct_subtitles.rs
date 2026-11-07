//! Subtitle sidecars for direct downloads. Addresses and provider headers never reach disk.
use crate::{downloads::{read_record, write_record, CompletionMetadata, Destination, DestinationIdentity}, hls_download::{fetch_external_subtitle, rename_no_replace, ExternalSubtitle}, models::Subtitle};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{File, OpenOptions}, io::Read, os::{fd::AsRawFd, unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt}}, path::{Path, PathBuf}};
use tokio::sync::watch;

const RECORD: &str = "direct-subtitles.json";
const STAGE: &str = "subtitle-stage";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Track { name: String, language: Option<String>, file: String, bytes: u64, sha256: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record { version: u32, folder: String, device: u64, inode: u64, video_sha256: String, tracks: Vec<Track>, missing: usize }

fn folder_name(id: &str) -> String { format!("cloudstream-{id}.subtitles") }
fn safe_file(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("sub-") else { return false };
    let Some((number, ext)) = rest.split_once('.') else { return false };
    !number.is_empty() && number.len() <= 2 && number.bytes().all(|b| b.is_ascii_digit()) && matches!(ext, "srt" | "vtt" | "ass")
}
fn open_folder(dest: &Destination, id: &str, record: &Record) -> Result<File> {
    if record.version != 1 || record.folder != folder_name(id) || record.tracks.len() > 16 || record.tracks.iter().any(|t| !safe_file(&t.file) || t.bytes > 4 * 1024 * 1024) { bail!("Downloaded subtitle information is invalid"); }
    let marker = read_record::<CompletionMetadata>(&dest.completion())?.context("Download information is unavailable")?;
    if marker.file_sha256.as_deref() != Some(record.video_sha256.as_str()) { bail!("Downloaded subtitle information does not match the video"); }
    let folder = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(dest.anchored.join(&record.folder)).context("Downloaded subtitle folder is unavailable")?;
    let meta = folder.metadata()?;
    if meta.dev() != record.device || meta.ino() != record.inode { bail!("Downloaded subtitle folder was replaced"); }
    Ok(folder)
}
fn file_matches(folder: &File, track: &Track) -> Result<bool> {
    let path = PathBuf::from(format!("/proc/self/fd/{}", folder.as_raw_fd())).join(&track.file);
    let mut file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) { Ok(file) => file, Err(_) => return Ok(false) };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() != track.bytes { return Ok(false); }
    let mut bytes = Vec::new();
    file.by_ref().take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    Ok(bytes.len() as u64 == track.bytes && format!("{:x}", Sha256::digest(&bytes)) == track.sha256)
}

/// Returns false if paused. A published video is retained and retried on resume.
pub async fn save(root: &Path, identity: DestinationIdentity, id: &str, extension: &str, subtitles: &[ExternalSubtitle], skipped: usize, stop: &watch::Receiver<bool>) -> Result<bool> {
    if subtitles.len() > 16 { bail!("Too many subtitle tracks"); }
    let dest = Destination::open(root, identity, id, extension)?;
    let marker = read_record::<CompletionMetadata>(&dest.completion())?.context("Download information is unavailable")?;
    // Older published files have no trusted content digest. Keep their existing recovery
    // behavior without creating sidecars that cannot be bound to the original file.
    let Some(video_sha256) = marker.file_sha256 else { return Ok(true) };
    let record_path = dest.job.join(RECORD);
    let stage = dest.job.join(STAGE);
    let final_path = dest.anchored.join(folder_name(id));
    if final_path.symlink_metadata().is_ok() {
        let record = read_record::<Record>(&record_path)?.context("A subtitle folder already exists")?;
        let folder = open_folder(&dest, id, &record)?;
        if record.tracks.iter().all(|track| file_matches(&folder, track).unwrap_or(false)) { return Ok(true); }
        bail!("The downloaded subtitles have changed");
    }
    match std::fs::remove_dir_all(&stage) { Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(error) => return Err(error).context("Could not clear incomplete subtitles") }
    if subtitles.is_empty() && skipped == 0 {
        match std::fs::remove_file(&record_path) { Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(error) => return Err(error.into()) }
        return Ok(true);
    }
    std::fs::DirBuilder::new().mode(0o700).create(&stage)?;
    let mut tracks = Vec::new();
    let mut missing = skipped;
    for (index, subtitle) in subtitles.iter().enumerate() {
        if *stop.borrow() { return Ok(false); }
        let fetched = fetch_external_subtitle(subtitle, stop).await?;
        if *stop.borrow() { return Ok(false); }
        let Some((data, extension)) = fetched else { missing += 1; continue };
        let file = format!("sub-{index}.{extension}");
        let mut output = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(stage.join(&file))?;
        std::io::Write::write_all(&mut output, &data)?;
        output.sync_all()?;
        tracks.push(Track { name: subtitle.name.chars().filter(|c| !c.is_control()).take(64).collect(), language: Some(subtitle.language.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect()).filter(|s: &String| !s.is_empty()), file, bytes: data.len() as u64, sha256: format!("{:x}", Sha256::digest(&data)) });
    }
    let metadata = std::fs::metadata(&stage)?;
    let record = Record { version: 1, folder: folder_name(id), device: metadata.dev(), inode: metadata.ino(), video_sha256, tracks, missing };
    File::open(&stage)?.sync_all()?;
    write_record(&record_path, &record)?;
    rename_no_replace(&stage, &final_path).context("Could not publish downloaded subtitles; the destination may already exist")?;
    File::open(&dest.anchored)?.sync_all()?;
    Ok(true)
}

/// Only app-published, unchanged sidecars are returned. Old downloads have no sidecar record.
pub fn read(root: &Path, identity: DestinationIdentity, id: &str, extension: &str) -> Result<Option<(Vec<Subtitle>, usize)>> {
    let dest = Destination::open(root, identity, id, extension)?;
    let Some(record) = read_record::<Record>(&dest.job.join(RECORD))? else {
        if dest.anchored.join(folder_name(id)).symlink_metadata().is_ok() { bail!("A subtitle folder exists without its ownership information"); }
        return Ok(None);
    };
    let folder = open_folder(&dest, id, &record)?;
    let mut subtitles = Vec::new();
    for track in record.tracks {
        if !file_matches(&folder, &track)? { bail!("The downloaded subtitle files are missing or have changed"); }
        let path = dest.final_path.parent().context("Invalid download folder")?.join(&record.folder).join(&track.file);
        subtitles.push(Subtitle { lang: track.name, lang_tag: track.language, url: path.to_str().context("Invalid subtitle path")?.into(), ..Default::default() });
    }
    Ok(Some((subtitles, record.missing)))
}

/// Called while the download lock is already held. Unknown and edited files are kept.
pub(crate) fn cleanup(dest: &Destination, id: &str, delete_output: bool, notes: &mut Vec<String>) -> Result<()> {
    let stage = dest.job.join(STAGE);
    match std::fs::remove_dir_all(stage) { Ok(()) => {}, Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}, Err(_) => notes.push("Incomplete subtitle files could not be cleaned.".into()) }
    if !delete_output { return Ok(()); }
    let Some(record) = read_record::<Record>(&dest.job.join(RECORD))? else { return Ok(()) };
    let folder = match open_folder(dest, id, &record) { Ok(folder) => folder, Err(_) => { notes.push("The subtitle folder was missing or replaced, so it was left in place.".into()); return Ok(()) } };
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", folder.as_raw_fd()));
    for track in &record.tracks {
        if file_matches(&folder, track)? { std::fs::remove_file(anchored.join(&track.file))?; }
        else { notes.push("A subtitle was missing or changed, so it was left in place.".into()); }
    }
    if std::fs::remove_dir(dest.anchored.join(&record.folder)).is_err() { notes.push("The subtitle folder contains other files, so it was left in place.".into()); }
    Ok(())
}
