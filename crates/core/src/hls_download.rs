//! The HLS download engine (T21).
//!
//! # Output strategy
//!
//! A finished HLS download is a **folder of local files**, not a remuxed single file:
//!
//! ```text
//! cloudstream-<job>.hls/
//!   index.m3u8            master playlist, local relative names only
//!   manifest.json         every file this download owns, with size and SHA-256
//!   video/  audio/  subs-0/   one local media playlist per track, plus its segments
//! ```
//!
//! mpv (through FFmpeg's HLS demuxer) plays that folder directly, with separate audio and
//! subtitle tracks, and no network. Segments are never concatenated, so fragmented MP4 is never
//! passed off as a plain MP4. Encrypted (AES-128) segments are **decrypted as they are stored**,
//! so no key is ever written to disk and the local playlist has no `EXT-X-KEY`; the keys are held
//! in memory only for the length of the run and fetched again on resume.
//!
//! # Resume
//!
//! The plan (which tracks, how many segments, their durations) is derived from freshly fetched
//! playlists on every start. An append-only journal in the job's private folder records each stored
//! segment (file, size, SHA-256). On resume, each journalled file is checked against its recorded
//! size and hash; anything missing or altered is downloaded again. If the playlists no longer have
//! the same *structure* (addresses are ignored, since signed addresses are renewed) the stored
//! parts are discarded and the download starts over. Before using any saved parts, the engine
//! fetches them again and compares their contents, including decrypted bytes. A changed part
//! invalidates the saved plan and asks the user to retry from the beginning. This costs network
//! bandwidth but avoids relying on sources to provide reliable validators.
//!
//! Nothing here stores a playlist, segment or key address on disk.
use crate::downloads::{read_record, write_record, CompletionMetadata, Destination, DestinationIdentity};
use crate::hls::{self, Container, HlsError, LocalMaster, LocalRendition, LocalSegment, MapRef, MediaPlaylist, Playlist, RenditionKind, SelectionPolicy, Variant};
use anyhow::{bail, Context, Result};
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufRead, Write},
    os::{fd::AsRawFd, unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt}},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet};
use url::Url;

const CONCURRENCY: usize = 4;
const MAX_SEGMENT_BYTES: usize = 128 * 1024 * 1024;
const MAX_KEY_BYTES: usize = 4096;
const ATTEMPTS: u32 = 3;

// ---------------------------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------------------------

/// Why an HLS download stopped. Every message is fixed text plus counts, so it can be saved and
/// shown without ever containing an address, a key or a provider's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsFailure {
    Unsupported(String),
    Unreadable(String),
    PlaylistUnavailable,
    KeyUnavailable,
    RangesUnsupported,
    SourceTooLarge,
    Segment { track: &'static str, number: usize, problem: SegmentProblem },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentProblem {
    /// The source answered with a definite refusal (not found, gone, forbidden...).
    Gone(u16),
    /// The connection failed or the source kept answering with errors.
    Unreachable,
    /// The bytes received are not the length the playlist promised.
    WrongSize,
    /// The bytes are not media (typically an error page returned with a success status).
    NotMedia,
    CannotDecrypt,
    Changed,
}

impl std::fmt::Display for HlsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(message) | Self::Unreadable(message) => f.write_str(message),
            Self::PlaylistUnavailable => f.write_str("The stream's playlist could not be loaded. Check the connection and retry."),
            Self::KeyUnavailable => f.write_str("The decryption key could not be loaded, or it is not a valid key."),
            Self::RangesUnsupported => f.write_str("The source does not support partial requests, which this stream needs."),
            Self::SourceTooLarge => f.write_str("A part of this stream is larger than this app will download."),
            Self::Segment { track, number, problem } => match problem {
                SegmentProblem::Gone(status) => write!(f, "{track} segment {number} is no longer available at the source (HTTP {status}). Retry later or choose another source."),
                SegmentProblem::Unreachable => write!(f, "{track} segment {number} could not be downloaded. Check the connection and retry; finished parts are kept."),
                SegmentProblem::WrongSize => write!(f, "{track} segment {number} is not the size the playlist promised, so the source may have changed."),
                SegmentProblem::NotMedia => write!(f, "{track} segment {number} is not media data. The source may be returning an error page."),
                SegmentProblem::CannotDecrypt => write!(f, "{track} segment {number} could not be decrypted with the key the source provided."),
                SegmentProblem::Changed => write!(f, "{track} segment {number} has changed at the source. Retry to download the current version from the beginning."),
            },
        }
    }
}
impl std::error::Error for HlsFailure {}

impl From<HlsError> for HlsFailure {
    fn from(error: HlsError) -> Self {
        match error {
            HlsError::Unsupported(message) => Self::Unsupported(message),
            HlsError::Invalid(message) => Self::Unreadable(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HlsProgress {
    pub bytes: u64,
    /// An estimate until the download finishes, then the exact size.
    pub total: Option<u64>,
    pub segments_done: usize,
    pub segments_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsResult {
    Completed { path: PathBuf, bytes: u64 },
    Paused { bytes: u64 },
}

/// A subtitle file from the source's own list, already reachable at `url`.
#[derive(Debug, Clone)]
pub struct ExternalSubtitle {
    pub language: String,
    pub name: String,
    pub url: String,
    /// Whether this address is a local provider session that must bypass system proxies.
    pub no_proxy: bool,
}

/// Fetch one provider subtitle with its own network policy. Unsupported or unavailable tracks
/// return `None`; the caller checks the stop signal separately.
pub(crate) async fn fetch_external_subtitle(subtitle: &ExternalSubtitle, stop: &watch::Receiver<bool>) -> Result<Option<(Vec<u8>, &'static str)>> {
    let Ok(url) = crate::repository::network_url(&subtitle.url) else { return Ok(None) };
    let net = Net::new(subtitle.no_proxy)?;
    let Ok((data, _)) = net.get(&url, None, 4 * 1024 * 1024, stop.clone()).await else { return Ok(None) };
    let text = String::from_utf8_lossy(&data);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let extension = if trimmed.starts_with("WEBVTT") { "vtt" }
        else if trimmed.starts_with("[Script Info]") { "ass" }
        else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains("-->") { "srt" }
        else { return Ok(None) };
    Ok(Some((data, extension)))
}

pub struct HlsRequest<'a> {
    pub root: &'a Path,
    pub identity: DestinationIdentity,
    pub job_id: &'a str,
    /// Identifies the logical stream across renewed addresses.
    pub source_identity: &'a str,
    pub url: &'a str,
    /// True when `url` is a local transport session (no system proxy).
    pub no_proxy: bool,
    pub policy: SelectionPolicy,
    pub subtitles: Vec<ExternalSubtitle>,
    /// Subtitle tracks omitted before transfer (unsupported, unsafe, or above the limit).
    pub skipped_subtitles: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestTrack {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// A local playlist (for in-stream tracks) or a plain subtitle file (for the source's own list).
    pub path: String,
}

/// What a finished download contains. Written to `manifest.json` inside the folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    pub duration_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ManifestTrack>,
    #[serde(default)]
    pub subtitles: Vec<ManifestTrack>,
    #[serde(default)]
    pub external_subtitles: Vec<ManifestTrack>,
    #[serde(default)]
    pub missing_external_subtitles: usize,
    pub files: Vec<ManifestFile>,
}

impl Manifest {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

pub const MANIFEST_NAME: &str = "manifest.json";
pub const MASTER_NAME: &str = "index.m3u8";

/// A file name this module wrote: lower-case letters, digits, dots, dashes, one folder level.
pub(crate) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('/').count() <= 2
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.starts_with('.') && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'))
}

// ---------------------------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
enum NetError {
    Status(u16),
    Transient,
    TooLarge,
    RangeRefused,
    Stopped,
}

struct Net {
    client: Client,
}

fn valid_content_range(value: &str, length: u64, offset: u64) -> bool {
    let Some(expected_end) = length.checked_sub(1).and_then(|n| offset.checked_add(n)) else { return false };
    let Some((bounds, total)) = value.strip_prefix("bytes ").and_then(|value| value.split_once('/')) else { return false };
    let Some((start, end)) = bounds.split_once('-') else { return false };
    if start.parse::<u64>().ok() != Some(offset) || end.parse::<u64>().ok() != Some(expected_end) { return false; }
    total == "*" || total.parse::<u64>().is_ok_and(|total| total > expected_end)
}

impl Net {
    fn new(no_proxy: bool) -> Result<Self> {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || crate::repository::network_url(attempt.url().as_str()).is_err() {
                attempt.error("Invalid download redirect")
            } else {
                attempt.follow()
            }
        });
        let mut builder = Client::builder().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30)).redirect(policy);
        if no_proxy {
            builder = builder.no_proxy();
        }
        Ok(Self { client: builder.build()? })
    }

    async fn once(&self, url: &Url, range: Option<(u64, u64)>, limit: usize) -> Result<(Vec<u8>, Url), NetError> {
        let mut request = self.client.get(url.clone()).header(header::ACCEPT_ENCODING, "identity");
        if let Some((length, offset)) = range {
            let end = length.checked_sub(1).and_then(|n| offset.checked_add(n)).ok_or(NetError::RangeRefused)?;
            request = request.header(header::RANGE, format!("bytes={offset}-{end}"));
        }
        let mut response = request.send().await.map_err(|_| NetError::Transient)?;
        let status = response.status();
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(NetError::Transient);
        }
        if range.is_some() {
            match status {
                StatusCode::PARTIAL_CONTENT => {
                    let (length, offset) = range.expect("range was checked above");
                    if !response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).is_some_and(|value| valid_content_range(value, length, offset)) {
                        return Err(NetError::RangeRefused);
                    }
                }
                // A server that ignores the range would send the whole file for every segment.
                StatusCode::OK => return Err(NetError::RangeRefused),
                other => return Err(NetError::Status(other.as_u16())),
            }
        } else if !status.is_success() {
            return Err(NetError::Status(status.as_u16()));
        }
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(NetError::TooLarge);
        }
        let announced = response.content_length();
        let final_url = response.url().clone();
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > limit {
                        return Err(NetError::TooLarge);
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => return Err(NetError::Transient),
            }
        }
        if announced.is_some_and(|n| n != body.len() as u64) {
            return Err(NetError::Transient);
        }
        Ok((body, final_url))
    }

    /// Retry transient failures with a short, stop-aware backoff. Definite refusals return at once.
    async fn get(&self, url: &Url, range: Option<(u64, u64)>, limit: usize, mut stop: watch::Receiver<bool>) -> Result<(Vec<u8>, Url), NetError> {
        let mut last = NetError::Transient;
        for attempt in 0..ATTEMPTS {
            let outcome = tokio::select! {
                _ = cancelled(&mut stop) => return Err(NetError::Stopped),
                outcome = self.once(url, range, limit) => outcome,
            };
            match outcome {
                Err(NetError::Transient) => last = NetError::Transient,
                other => return other,
            }
            if attempt + 1 < ATTEMPTS {
                tokio::select! {
                    _ = cancelled(&mut stop) => return Err(NetError::Stopped),
                    _ = tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))) => {}
                }
            }
        }
        Err(last)
    }
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackKind {
    Video,
    Audio,
    Subtitles,
}

struct Track {
    id: String,
    label: &'static str,
    kind: TrackKind,
    name: String,
    language: Option<String>,
    default: bool,
    forced: bool,
    playlist: MediaPlaylist,
    /// Unique initialisation segments, in order of first use.
    maps: Vec<MapRef>,
}

struct Prepared {
    tracks: Vec<Track>,
    variant: Variant,
    structure: String,
    estimate: u64,
    segments_total: usize,
}

fn unique_maps(playlist: &MediaPlaylist) -> Vec<MapRef> {
    let mut maps: Vec<MapRef> = Vec::new();
    for segment in &playlist.segments {
        if let Some(map) = &segment.map {
            if !maps.contains(map) {
                maps.push(map.clone());
            }
        }
    }
    maps
}

fn map_index(maps: &[MapRef], map: &MapRef) -> usize {
    maps.iter().position(|m| m == map).unwrap_or(0)
}

fn net_failure(error: NetError) -> HlsFailure {
    match error {
        NetError::TooLarge => HlsFailure::SourceTooLarge,
        NetError::RangeRefused => HlsFailure::RangesUnsupported,
        NetError::Stopped | NetError::Transient | NetError::Status(_) => HlsFailure::PlaylistUnavailable,
    }
}

async fn fetch_playlist(net: &Net, url: &Url, stop: &watch::Receiver<bool>) -> Result<(String, Url), HlsFailure> {
    let (body, final_url) = net.get(url, None, hls::MAX_PLAYLIST_BYTES, stop.clone()).await.map_err(net_failure)?;
    Ok((String::from_utf8_lossy(&body).into_owned(), final_url))
}

async fn fetch_media(net: &Net, url: &Url, stop: &watch::Receiver<bool>) -> Result<MediaPlaylist, HlsFailure> {
    let (text, final_url) = fetch_playlist(net, url, stop).await?;
    Ok(hls::parse_media(&text, &final_url)?)
}

async fn prepare(net: &Net, url: &Url, policy: &SelectionPolicy, stop: &watch::Receiver<bool>) -> Result<Prepared, HlsFailure> {
    let (text, final_url) = fetch_playlist(net, url, stop).await?;
    let mut tracks = Vec::new();
    let variant;
    match hls::parse(&text, &final_url)? {
        Playlist::Media(playlist) => {
            let maps = unique_maps(&playlist);
            variant = Variant { uri: final_url, bandwidth: 0, average_bandwidth: None, resolution: None, codecs: None, audio: None, subtitles: None };
            tracks.push(Track { id: "video".into(), label: "Video", kind: TrackKind::Video, name: "Video".into(), language: None, default: true, forced: false, playlist, maps });
        }
        Playlist::Master(master) => {
            let selection = hls::select(&master, policy)?;
            let video = fetch_media(net, &selection.variant.uri, stop).await?;
            let maps = unique_maps(&video);
            tracks.push(Track { id: "video".into(), label: "Video", kind: TrackKind::Video, name: "Video".into(), language: None, default: true, forced: false, playlist: video, maps });
            if let Some(audio) = &selection.audio {
                let playlist = fetch_media(net, audio.uri.as_ref().expect("selected audio has an address"), stop).await?;
                let maps = unique_maps(&playlist);
                tracks.push(Track { id: "audio".into(), label: "Audio", kind: TrackKind::Audio, name: audio.name.clone(), language: audio.language.clone(), default: true, forced: false, playlist, maps });
            }
            for (n, subtitle) in selection.subtitles.iter().enumerate() {
                let playlist = fetch_media(net, subtitle.uri.as_ref().expect("selected subtitle has an address"), stop).await?;
                let maps = unique_maps(&playlist);
                tracks.push(Track { id: format!("subs-{n}"), label: "Subtitle", kind: TrackKind::Subtitles, name: subtitle.name.clone(), language: subtitle.language.clone(), default: subtitle.default, forced: subtitle.forced, playlist, maps });
            }
            variant = selection.variant;
        }
    }
    let mut hasher = Sha256::new();
    for track in &tracks {
        hasher.update(hls::structure_hash(&track.id, &track.playlist).as_bytes());
        hasher.update(format!("|{}|{:?}|{}|{}\n", track.name, track.language, track.default, track.forced).as_bytes());
    }
    let structure = hex(&hasher.finalize());
    // The variant's bandwidth already covers its audio, so only the video track is estimated from it.
    let estimate = tracks.iter().filter(|t| t.kind == TrackKind::Video).map(|t| hls::estimated_bytes(&variant, &t.playlist)).sum::<u64>()
        + tracks.iter().filter(|t| t.kind == TrackKind::Audio).map(|t| hls::ranged_bytes(&t.playlist).unwrap_or(0)).sum::<u64>();
    let segments_total = tracks.iter().map(|t| t.playlist.segments.len() + t.maps.len()).sum();
    Ok(Prepared { tracks, variant, structure, estimate, segments_total })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------
// Stored state
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanRecord {
    version: u32,
    source_hash: String,
    structure: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Done {
    /// Track index, then `s` (segment) or `m` (initialisation map), then its index in the track.
    t: usize,
    k: char,
    i: usize,
    f: String,
    n: u64,
    h: String,
}

struct Journal {
    file: std::fs::File,
}

impl Journal {
    fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).context("Could not open the download journal")?;
        Ok(Self { file })
    }
    fn append(&mut self, done: &Done) -> Result<()> {
        let mut line = serde_json::to_vec(done)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.file.sync_data().context("Could not save download progress")
    }
}

fn read_journal(path: &Path) -> HashMap<(usize, char, usize), Done> {
    let mut out = HashMap::new();
    let Ok(file) = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path) else { return out };
    for line in std::io::BufReader::new(file).lines().map_while(|l| l.ok()) {
        if let Ok(done) = serde_json::from_str::<Done>(&line) {
            out.insert((done.t, done.k, done.i), done);
        }
    }
    out
}

fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        bail!("Not a regular file");
    }
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok((hex(&hasher.finalize()), meta.len()))
}

fn remove_tree(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Could not clear old download data"),
    }
}

// ---------------------------------------------------------------------------------------------
// Work
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Work {
    track: usize,
    label: &'static str,
    dir: String,
    kind: char,
    index: usize,
    /// The number shown in messages: 1-based among this track's segments.
    number: usize,
    url: Url,
    range: Option<(u64, u64)>,
    key: Option<(Url, [u8; 16])>,
    subtitles: bool,
}

fn work_for(track_index: usize, track: &Track) -> Vec<Work> {
    let mut work = Vec::new();
    for (i, map) in track.maps.iter().enumerate() {
        work.push(Work { track: track_index, label: track.label, dir: track.id.clone(), kind: 'm', index: i, number: i + 1, url: map.uri.clone(), range: map.range, key: None, subtitles: false });
    }
    for (i, segment) in track.playlist.segments.iter().enumerate() {
        let key = segment.key.as_ref().map(|k| (k.uri.clone(), hls::iv_for(segment)));
        work.push(Work { track: track_index, label: track.label, dir: track.id.clone(), kind: 's', index: i, number: i + 1, url: segment.uri.clone(), range: segment.range, key, subtitles: track.kind == TrackKind::Subtitles });
    }
    work
}

type KeyCache = Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::OnceCell<[u8; 16]>>>>>;

/// Fetch a key once per run, however many segments need it at the same moment: some sources hand
/// out a key address that works only once, or rate-limit repeated requests.
async fn fetch_key(net: &Net, cache: &KeyCache, url: &Url, stop: &watch::Receiver<bool>) -> Result<[u8; 16], HlsFailure> {
    let cell = cache.lock().await.entry(url.to_string()).or_default().clone();
    cell.get_or_try_init(|| async {
        let (body, _) = net.get(url, None, MAX_KEY_BYTES, stop.clone()).await.map_err(|_| HlsFailure::KeyUnavailable)?;
        <[u8; 16]>::try_from(body.as_slice()).map_err(|_| HlsFailure::KeyUnavailable)
    })
    .await
    .copied()
}

async fn fetch_work(net: &Net, keys: &KeyCache, work: &Work, stop: &watch::Receiver<bool>) -> Result<(Vec<u8>, Container), HlsFailure> {
    let problem = |problem| HlsFailure::Segment { track: work.label, number: work.number, problem };
    let (mut data, _) = net.get(&work.url, work.range, MAX_SEGMENT_BYTES, stop.clone()).await.map_err(|e| match e {
        NetError::Status(status) => problem(SegmentProblem::Gone(status)),
        NetError::TooLarge => HlsFailure::SourceTooLarge,
        NetError::RangeRefused => HlsFailure::RangesUnsupported,
        NetError::Transient | NetError::Stopped => problem(SegmentProblem::Unreachable),
    })?;
    if let Some((length, _)) = work.range {
        if data.len() as u64 != length {
            return Err(problem(SegmentProblem::WrongSize));
        }
    }
    if let Some((key_url, iv)) = &work.key {
        let key = fetch_key(net, keys, key_url, stop).await?;
        data = hls::decrypt_aes128(&key, iv, &data).map_err(|_| problem(SegmentProblem::CannotDecrypt))?;
    }
    let container = hls::sniff(&data).ok_or_else(|| problem(SegmentProblem::NotMedia))?;
    let acceptable = if work.kind == 'm' {
        container == Container::Fmp4
    } else if work.subtitles {
        container == Container::WebVtt
    } else {
        container != Container::WebVtt
    };
    if !acceptable {
        return Err(problem(SegmentProblem::NotMedia));
    }
    Ok((data, container))
}

async fn run_work(net: Arc<Net>, keys: KeyCache, stage: PathBuf, work: Work, stop: watch::Receiver<bool>) -> Result<Done, HlsFailure> {
    let (data, container) = fetch_work(&net, &keys, &work, &stop).await?;
    let extension = container.extension(work.kind == 'm');
    let name = if work.kind == 'm' { format!("init-{}.{extension}", work.index) } else { format!("seg-{:06}.{extension}", work.index) };
    let directory = stage.join(&work.dir);
    let io = |_: std::io::Error| HlsFailure::Unreadable("Could not write the download; check available disk space".into());
    let temporary = directory.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&temporary, &data).await.map_err(io)?;
    if let Err(error) = tokio::fs::rename(&temporary, directory.join(&name)).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(io(error));
    }
    Ok(Done { t: work.track, k: work.kind, i: work.index, f: name, n: data.len() as u64, h: hex(&Sha256::digest(&data)) })
}

// ---------------------------------------------------------------------------------------------
// The download
// ---------------------------------------------------------------------------------------------

fn free_space(path: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
    let result = unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Download an HLS stream into a local folder. Pausing (or the stop signal) keeps everything stored
/// so far; the same call resumes it.
pub async fn download_hls<F>(request: HlsRequest<'_>, stop: watch::Receiver<bool>, mut progress: F) -> Result<HlsResult>
where
    F: FnMut(HlsProgress),
{
    if request.source_identity.is_empty() || request.source_identity.len() > 4096 {
        bail!("Invalid download source identity");
    }
    let url = crate::repository::network_url(request.url)?;
    let dest = Destination::open_hls(request.root, request.identity, request.job_id)?;
    let source_hash = crate::models::digest_string(request.source_identity);
    let stage = dest.job.join("stage");
    let published = dest.destination();

    // Already published by an earlier run (including one interrupted before the database caught up).
    if let Ok(meta) = std::fs::symlink_metadata(&published) {
        let marker = read_record::<CompletionMetadata>(&dest.completion())?;
        let ours = marker.is_some_and(|m| {
            m.version == 2 && m.source_hash == source_hash && dest.final_path.file_name().and_then(|n| n.to_str()) == Some(m.name.as_str()) && meta.is_dir() && !meta.file_type().is_symlink() && meta.dev() == m.device && meta.ino() == m.inode && m.bytes > 0
        });
        if ours && !stage.exists() {
            let bytes = read_record::<CompletionMetadata>(&dest.completion())?.map_or(0, |m| m.bytes);
            return Ok(HlsResult::Completed { path: dest.final_path.clone(), bytes });
        }
        bail!("The destination already exists");
    }

    let net = Arc::new(Net::new(request.no_proxy)?);
    let prepared = match prepare(&net, &url, &request.policy, &stop).await {
        Ok(prepared) => prepared,
        Err(HlsFailure::PlaylistUnavailable) if *stop.borrow() => return Ok(HlsResult::Paused { bytes: 0 }),
        Err(failure) => return Err(failure.into()),
    };

    // Same structure as the stored parts? Otherwise they belong to a different version of the source.
    let plan_path = dest.job.join("hls-plan.json");
    let journal_path = dest.job.join("hls-journal");
    let wanted = PlanRecord { version: 1, source_hash: source_hash.clone(), structure: prepared.structure.clone() };
    let same = read_record::<PlanRecord>(&plan_path)?.is_some_and(|stored| stored.version == wanted.version && stored.source_hash == wanted.source_hash && stored.structure == wanted.structure);
    if !same {
        remove_tree(&stage)?;
        match std::fs::remove_file(&journal_path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error).context("Could not reset download progress"),
        }
        write_record(&plan_path, &wanted)?;
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&stage).context("Could not create the download folder")?;
    for track in &prepared.tracks {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(stage.join(&track.id)).context("Could not create the download folder")?;
        // Half-written files from an interrupted run are never trusted.
        if let Ok(entries) = std::fs::read_dir(stage.join(&track.id)) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(".tmp-") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    // Replay the journal, trusting only parts that are still exactly what was stored.
    let mut done: HashMap<(usize, char, usize), Done> = HashMap::new();
    for (key, entry) in read_journal(&journal_path) {
        let valid = prepared.tracks.get(key.0).is_some_and(|track| match key.1 {
            'm' => key.2 < track.maps.len(),
            's' => key.2 < track.playlist.segments.len(),
            _ => false,
        }) && safe_relative(&entry.f)
            && !entry.f.contains('/')
            && sha256_file(&stage.join(&prepared.tracks[key.0].id).join(&entry.f)).is_ok_and(|(hash, len)| hash == entry.h && len == entry.n);
        if valid {
            done.insert(key, entry);
        }
    }
    // Compact the journal so repaired entries do not accumulate.
    {
        let _ = std::fs::remove_file(&journal_path);
        let mut journal = Journal::open(&journal_path)?;
        let mut kept: Vec<&Done> = done.values().collect();
        kept.sort_by_key(|d| (d.t, d.k, d.i));
        for entry in kept {
            journal.append(entry)?;
        }
    }

    let all: Vec<Work> = prepared.tracks.iter().enumerate().flat_map(|(i, t)| work_for(i, t)).collect();
    let keys: KeyCache = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    // A playlist can retain the same paths and durations while the source silently replaces
    // the media. Recheck every saved part before trusting any of them, including AES decryption.
    // On mismatch invalidate the plan durably before reporting it; the next retry
    // starts afresh. Missing or temporarily unreachable parts leave the journal intact.
    for work in all.iter().filter(|w| done.contains_key(&(w.track, w.kind, w.index))) {
        let fetched = fetch_work(&net, &keys, work, &stop).await;
        if *stop.borrow() { return Ok(HlsResult::Paused { bytes: done.values().map(|d| d.n).sum() }); }
        let (data, _) = fetched?;
        let entry = &done[&(work.track, work.kind, work.index)];
        if data.len() as u64 != entry.n || hex(&Sha256::digest(&data)) != entry.h {
            std::fs::remove_file(&plan_path).context("Could not reset download progress")?;
            std::fs::File::open(&dest.job)?.sync_all().context("Could not reset download progress")?;
            return Err(HlsFailure::Segment { track: work.label, number: work.number, problem: SegmentProblem::Changed }.into());
        }
    }
    let pending: Vec<Work> = all.iter().filter(|w| !done.contains_key(&(w.track, w.kind, w.index))).cloned().collect();
    let mut stored_bytes: u64 = done.values().map(|d| d.n).sum();
    let total = (prepared.estimate > 0).then_some(prepared.estimate.max(stored_bytes));
    if let (Some(total), Some(free)) = (total, free_space(&dest.anchored)) {
        let needed = total.saturating_sub(stored_bytes);
        if needed.saturating_add(needed / 10) > free {
            return Err(anyhow::Error::from(std::io::Error::from_raw_os_error(libc::ENOSPC)).context("Not enough free space for this download"));
        }
    }
    let report = |bytes: u64, finished: usize| HlsProgress { bytes, total: total.map(|estimate| estimate.max(bytes)), segments_done: finished, segments_total: prepared.segments_total };
    progress(report(stored_bytes, done.len()));

    let mut journal = Journal::open(&journal_path)?;
    let mut queue = pending.into_iter();
    let mut running: JoinSet<Result<Done, HlsFailure>> = JoinSet::new();
    let mut pause = stop.clone();
    loop {
        // A pause requested while a segment was being recorded wins over finishing: otherwise a
        // fast source could race past the request and publish a download the person had paused.
        if *pause.borrow() {
            running.abort_all();
            while running.join_next().await.is_some() {}
            return Ok(HlsResult::Paused { bytes: stored_bytes });
        }
        while running.len() < CONCURRENCY {
            let Some(work) = queue.next() else { break };
            running.spawn(run_work(net.clone(), keys.clone(), stage.clone(), work, stop.clone()));
        }
        if running.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            _ = cancelled(&mut pause) => {
                running.abort_all();
                while running.join_next().await.is_some() {}
                return Ok(HlsResult::Paused { bytes: stored_bytes });
            }
            joined = running.join_next() => match joined {
                Some(Ok(Ok(entry))) => {
                    journal.append(&entry)?;
                    stored_bytes += entry.n;
                    done.insert((entry.t, entry.k, entry.i), entry);
                    progress(report(stored_bytes, done.len()));
                }
                Some(Ok(Err(failure))) => {
                    running.abort_all();
                    while running.join_next().await.is_some() {}
                    if *stop.borrow() && matches!(failure, HlsFailure::Segment { problem: SegmentProblem::Unreachable, .. }) {
                        return Ok(HlsResult::Paused { bytes: stored_bytes });
                    }
                    return Err(failure.into());
                }
                Some(Err(_)) => bail!("A download task failed unexpectedly"),
                None => break,
            }
        }
    }

    finalize(&dest, &stage, &prepared, &done, &request.subtitles, request.skipped_subtitles, &stop, &source_hash).await
}

/// Write the local playlists and manifest, record completion, and publish the folder in one
/// atomic step that refuses to replace anything.
#[allow(clippy::too_many_arguments)]
async fn finalize(dest: &Destination, stage: &Path, prepared: &Prepared, done: &HashMap<(usize, char, usize), Done>, external: &[ExternalSubtitle], skipped_subtitles: usize, stop: &watch::Receiver<bool>, source_hash: &str) -> Result<HlsResult> {
    let mut files: Vec<ManifestFile> = Vec::new();
    let mut playlist_files: Vec<(String, String)> = Vec::new();
    let mut local_tracks: Vec<(usize, String)> = Vec::new();
    for (t, track) in prepared.tracks.iter().enumerate() {
        let mut segments = Vec::with_capacity(track.playlist.segments.len());
        for (i, segment) in track.playlist.segments.iter().enumerate() {
            let entry = done.get(&(t, 's', i)).context("A segment is missing")?;
            let map = match &segment.map {
                Some(m) => Some(done.get(&(t, 'm', map_index(&track.maps, m))).context("An initialisation segment is missing")?.f.clone()),
                None => None,
            };
            files.push(ManifestFile { path: format!("{}/{}", track.id, entry.f), bytes: entry.n, sha256: entry.h.clone() });
            segments.push(LocalSegment { file: entry.f.clone(), duration: segment.duration, discontinuity: segment.discontinuity, map });
        }
        for i in 0..track.maps.len() {
            let entry = done.get(&(t, 'm', i)).context("An initialisation segment is missing")?;
            files.push(ManifestFile { path: format!("{}/{}", track.id, entry.f), bytes: entry.n, sha256: entry.h.clone() });
        }
        let text = hls::local_media_playlist(&segments, track.playlist.target_duration.max(segments.iter().map(|s| s.duration).fold(0.0, f64::max)));
        playlist_files.push((format!("{}/index.m3u8", track.id), text));
        local_tracks.push((t, format!("{}/index.m3u8", track.id)));
    }

    // The source's own subtitle files, fetched last: a missing one never fails the video.
    let mut external_tracks = Vec::new();
    let mut missing_external_subtitles = skipped_subtitles;
    let directory = stage.join("external");
    remove_tree(&directory)?;
    if !external.is_empty() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&directory)?;
        for (n, subtitle) in external.iter().enumerate() {
            if *stop.borrow() {
                return Ok(HlsResult::Paused { bytes: done.values().map(|d| d.n).sum() });
            }
            let fetched = fetch_external_subtitle(subtitle, stop).await?;
            if *stop.borrow() { return Ok(HlsResult::Paused { bytes: done.values().map(|d| d.n).sum() }); }
            let Some((data, extension)) = fetched else { missing_external_subtitles += 1; continue };
            let name = format!("sub-{n}.{extension}");
            std::fs::write(directory.join(&name), &data)?;
            files.push(ManifestFile { path: format!("external/{name}"), bytes: data.len() as u64, sha256: hex(&Sha256::digest(&data)) });
            external_tracks.push(ManifestTrack { name: subtitle.name.chars().filter(|c| !c.is_control()).take(64).collect(), language: Some(subtitle.language.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect()).filter(|l: &String| !l.is_empty()), default: false, forced: false, path: format!("external/{name}") });
        }
    }

    let mut audio_rendition = None;
    let mut subtitle_renditions = Vec::new();
    let mut manifest_audio = None;
    let mut manifest_subs = Vec::new();
    for (t, playlist) in &local_tracks {
        let track = &prepared.tracks[*t];
        let rendition = |kind, group: &str| LocalRendition { kind, group: group.into(), name: track.name.clone(), language: track.language.clone(), default: track.default, forced: track.forced, playlist: playlist.clone() };
        let info = ManifestTrack { name: track.name.clone(), language: track.language.clone(), default: track.default, forced: track.forced, path: playlist.clone() };
        match track.kind {
            TrackKind::Audio => {
                audio_rendition = Some(rendition(RenditionKind::Audio, "audio"));
                manifest_audio = Some(info);
            }
            TrackKind::Subtitles => {
                subtitle_renditions.push(rendition(RenditionKind::Subtitles, "subs"));
                manifest_subs.push(info);
            }
            TrackKind::Video => {}
        }
    }
    let duration = prepared.tracks.first().map_or(0.0, |t| t.playlist.total_duration());
    let video_bytes: u64 = files.iter().filter(|f| f.path.starts_with("video/")).map(|f| f.bytes).sum();
    let mut variant = prepared.variant.clone();
    if variant.bandwidth == 0 {
        variant.bandwidth = if duration > 0.0 { ((video_bytes as f64 * 8.0) / duration) as u64 } else { 0 }.max(1);
    }
    variant.audio = audio_rendition.as_ref().map(|_| "audio".to_string());
    let master = hls::local_master_playlist(&LocalMaster { variant: &variant, video_playlist: "video/index.m3u8".into(), audio: audio_rendition, subtitles: subtitle_renditions });
    playlist_files.push((MASTER_NAME.to_string(), master));
    for (path, text) in &playlist_files {
        std::fs::write(stage.join(path), text.as_bytes())?;
        files.push(ManifestFile { path: path.clone(), bytes: text.len() as u64, sha256: hex(&Sha256::digest(text.as_bytes())) });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = Manifest { version: 1, duration_seconds: duration, width: prepared.variant.resolution.map(|r| r.0), height: prepared.variant.resolution.map(|r| r.1), audio: manifest_audio, subtitles: manifest_subs, external_subtitles: external_tracks, missing_external_subtitles, files };
    if manifest.files.iter().any(|f| !safe_relative(&f.path)) {
        bail!("The download produced an unexpected file name");
    }
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(stage.join(MANIFEST_NAME), &manifest_bytes)?;
    let bytes = manifest.bytes() + manifest_bytes.len() as u64;

    // Make everything durable before the folder becomes visible.
    for file in manifest.files.iter().map(|f| f.path.as_str()).chain([MANIFEST_NAME]) {
        std::fs::File::open(stage.join(file))?.sync_all()?;
    }
    let stage_meta = std::fs::metadata(stage)?;
    let marker = CompletionMetadata {
        version: 2,
        source_hash: source_hash.into(),
        name: dest.final_path.file_name().and_then(|n| n.to_str()).context("Invalid destination name")?.into(),
        device: stage_meta.dev(),
        inode: stage_meta.ino(),
        bytes,
        manifest_sha256: Some(hex(&Sha256::digest(&manifest_bytes))),
        file_sha256: None,
    };
    write_record(&dest.completion(), &marker)?;
    rename_no_replace(stage, &dest.destination()).context("Could not publish the finished download; the destination may already exist")?;
    std::fs::File::open(&dest.anchored)?.sync_all()?;
    let _ = std::fs::remove_file(dest.job.join("hls-journal"));
    let _ = std::fs::remove_file(dest.job.join("hls-plan.json"));
    Ok(HlsResult::Completed { path: dest.final_path.clone(), bytes })
}

// ---------------------------------------------------------------------------------------------
// Checking and removing a finished download
// ---------------------------------------------------------------------------------------------

/// The result of looking at a finished folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderCheck {
    /// The folder is the one this job published, and its manifest is the one recorded.
    pub intact_identity: bool,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
}

impl FolderCheck {
    pub fn healthy(&self) -> bool {
        self.intact_identity && self.missing.is_empty() && self.changed.is_empty()
    }
}

/// Read the manifest of a finished folder, but only if the folder is the one this job published.
pub(crate) fn trusted_manifest(dest: &Destination) -> Result<Option<Manifest>> {
    let Some(marker) = read_record::<CompletionMetadata>(&dest.completion())? else { return Ok(None) };
    let Ok(meta) = std::fs::symlink_metadata(dest.destination()) else { return Ok(None) };
    if marker.version != 2 || !meta.is_dir() || meta.file_type().is_symlink() || meta.dev() != marker.device || meta.ino() != marker.inode {
        return Ok(None);
    }
    let path = dest.destination().join(MANIFEST_NAME);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if bytes.len() <= 16 * 1024 * 1024 => bytes,
        _ => return Ok(None),
    };
    if marker.manifest_sha256.as_deref() != Some(hex(&Sha256::digest(&bytes)).as_str()) {
        return Ok(None);
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.files.iter().any(|f| !safe_relative(&f.path)) {
        return Ok(None);
    }
    Ok(Some(manifest))
}

/// Compare every file against the manifest. With `hashes`, contents are checked too; without it
/// only existence and size are, which is quick enough to do whenever the catalog is shown.
pub fn check_folder(root: &Path, identity: DestinationIdentity, job_id: &str, hashes: bool) -> Result<FolderCheck> {
    let dest = Destination::open_hls(root, identity, job_id)?;
    let Some(manifest) = trusted_manifest(&dest)? else { return Ok(FolderCheck::default()) };
    let root = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(dest.destination())?;
    let marker = read_record::<CompletionMetadata>(&dest.completion())?.context("Download identity is missing")?;
    let root_meta = root.metadata()?;
    if root_meta.dev() != marker.device || root_meta.ino() != marker.inode { return Ok(FolderCheck::default()); }
    let base = PathBuf::from(format!("/proc/self/fd/{}", root.as_raw_fd()));
    let mut check = FolderCheck { intact_identity: true, ..Default::default() };
    for file in &manifest.files {
        let (child, name) = if let Some((directory, name)) = file.path.split_once('/') {
            match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(base.join(directory)) {
                Ok(child) => (Some(child), name),
                Err(_) => { check.changed.push(file.path.clone()); continue; }
            }
        } else { (None, file.path.as_str()) };
        let path = child.as_ref().map(|dir| PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()))).unwrap_or_else(|| base.clone()).join(name);
        match std::fs::symlink_metadata(&path) {
            Err(_) => check.missing.push(file.path.clone()),
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != file.bytes => check.changed.push(file.path.clone()),
            Ok(_) if hashes => match sha256_file(&path) {
                Ok((hash, _)) if hash == file.sha256 => {}
                _ => check.changed.push(file.path.clone()),
            },
            Ok(_) => {}
        }
    }
    Ok(check)
}

pub fn read_manifest(root: &Path, identity: DestinationIdentity, job_id: &str) -> Result<Option<Manifest>> {
    let dest = Destination::open_hls(root, identity, job_id)?;
    trusted_manifest(&dest)
}

/// Verify the recorded output identity and size before the queue marks a job complete.
pub fn verify_completed_folder(root: &Path, identity: DestinationIdentity, job_id: &str, bytes: u64) -> Result<bool> {
    let dest = Destination::open_hls(root, identity, job_id)?;
    let marker = read_record::<CompletionMetadata>(&dest.completion())?;
    let valid = bytes > 0 && marker.is_some_and(|marker| marker.bytes == bytes)
        && trusted_manifest(&dest)?.is_some();
    drop(dest);
    Ok(valid && check_folder(root, identity, job_id, false)?.healthy())
}

/// Remove what one HLS job owns. Only files named in the verified manifest are deleted, and only
/// as regular files; anything else found in the folder is left alone and reported. Nothing is
/// removed when the folder is not the one this job published.
pub fn remove_folder(root: &Path, identity: DestinationIdentity, job_id: &str, delete_output: bool) -> Result<crate::downloads::RemovalReport> {
    let dest = Destination::open_hls(root, identity, job_id)?;
    let mut report = crate::downloads::RemovalReport::default();
    if delete_output {
        match trusted_manifest(&dest)? {
            None => report.notes.push("No finished download folder is recorded for this download, or it was replaced, so nothing was deleted.".into()),
            Some(manifest) => {
                // Keep the verified directory open and resolve each child through a separately
                // opened directory descriptor. A replaced track directory must never redirect
                // deletion into a user's unrelated folder.
                let root = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(dest.destination())?;
                let marker = read_record::<CompletionMetadata>(&dest.completion())?.context("Download identity is missing")?;
                let root_meta = root.metadata()?;
                if root_meta.dev() != marker.device || root_meta.ino() != marker.inode { bail!("The download folder was replaced"); }
                let base = PathBuf::from(format!("/proc/self/fd/{}", root.as_raw_fd()));
                let mut folders: Vec<PathBuf> = Vec::new();
                for file in &manifest.files {
                    let (directory, name) = match file.path.split_once('/') {
                        Some((directory, name)) => (Some(directory), name),
                        None => (None, file.path.as_str()),
                    };
                    let child = if let Some(directory) = directory {
                        if !folders.contains(&base.join(directory)) { folders.push(base.join(directory)); }
                        match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(base.join(directory)) {
                            Ok(child) => Some(child),
                            Err(_) => { report.notes.push("A download subfolder is missing or unsafe, so its files were left in place.".into()); continue; }
                        }
                    } else { None };
                    let path = child.as_ref().map(|dir| PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()))).unwrap_or_else(|| base.clone()).join(name);
                    match std::fs::symlink_metadata(&path) {
                        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink()
                            && sha256_file(&path).is_ok_and(|(hash, len)| hash == file.sha256 && len == file.bytes) =>
                            std::fs::remove_file(&path).context("Could not delete a downloaded file")?,
                        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => report.notes.push("A downloaded file was changed, so it was left in place.".into()),
                        Ok(_) => report.notes.push("An item that is not an ordinary file was left in place.".into()),
                        Err(_) => {}
                    }
                }
                let _ = std::fs::remove_file(base.join(MANIFEST_NAME));
                for folder in folders {
                    let _ = std::fs::remove_dir(folder);
                }
                match std::fs::remove_dir(dest.destination()) {
                    Ok(()) => report.file_deleted = true,
                    Err(_) => report.notes.push("The folder still contains files this app did not create, so it was left in place.".into()),
                }
            }
        }
    }
    // Private bookkeeping and any partial data.
    let _ = std::fs::remove_dir_all(dest.job.join("stage"));
    for entry in std::fs::read_dir(&dest.job).context("Could not read the download's temporary folder")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let meta = std::fs::symlink_metadata(entry.path())?;
        let known = matches!(name.as_ref(), "lock" | "completed.json" | "hls-plan.json" | "hls-journal") || name.starts_with(".resume-");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_range_response_must_match_the_requested_offsets() {
        assert!(valid_content_range("bytes 20-29/100", 10, 20));
        assert!(valid_content_range("bytes 20-29/*", 10, 20));
        for value in ["bytes 21-30/100", "bytes 20-29/29", "bytes 20-30/100", "bytes */100", "20-29/100"] {
            assert!(!valid_content_range(value, 10, 20), "{value}");
        }
        assert!(!valid_content_range("bytes 0-0/1", 0, 0));
        assert!(!valid_content_range("bytes 0-0/1", 2, u64::MAX));
    }

    #[test]
    fn failure_messages_never_carry_addresses_and_name_the_part() {
        let gone = HlsFailure::Segment { track: "Video", number: 7, problem: SegmentProblem::Gone(404) }.to_string();
        assert!(gone.contains("Video segment 7") && gone.contains("404") && !gone.contains("://"));
        for problem in [SegmentProblem::Unreachable, SegmentProblem::WrongSize, SegmentProblem::NotMedia, SegmentProblem::CannotDecrypt] {
            let message = HlsFailure::Segment { track: "Audio", number: 1, problem }.to_string();
            assert!(message.starts_with("Audio segment 1") && !message.contains("://"), "{message}");
        }
    }

    #[test]
    fn only_names_this_module_writes_are_treated_as_owned() {
        for ok in ["index.m3u8", "video/seg-000001.ts", "subs-0/index.m3u8", "external/sub-0.vtt", "manifest.json"] {
            assert!(safe_relative(ok), "{ok}");
        }
        for bad in ["", "../x", "video/../x", "/etc/passwd", "a/b/c", ".hidden", "video/.tmp-1", "Video/Seg.ts", "video//x", "a b"] {
            assert!(!safe_relative(bad), "{bad}");
        }
    }

    #[test]
    fn a_stop_requested_before_any_request_is_reported_as_pause_not_failure() {
        let (tx, rx) = watch::channel(true);
        drop(tx);
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let net = Net::new(false).unwrap();
            let result = net.get(&Url::parse("http://127.0.0.1:9/never").unwrap(), None, 10, rx).await;
            assert!(matches!(result, Err(NetError::Stopped)));
        });
    }
}
