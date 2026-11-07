//! Entry points from outside the app (T27): deep links, shared links, command-line arguments,
//! "open with" files and a second launch of the app.
//!
//! Everything here is parsing and queueing. Parsing is strict and total: an argument either
//! becomes exactly one `EntryAction` or a readable error, never a partial guess. Nothing in this
//! module installs, plays or deletes anything; the UI always shows what an action would do and
//! asks before the ones that change state or start playback.
//!
//! Link formats come from Cloudstream Android's `MainActivity.handleAppIntentUrl`:
//! `https://cs.repo/?host/path`, `cloudstreamrepo://…`, `cloudstreamsearch://<query>`,
//! `cloudstreamplayer://<encoded url>?name=…`, `cloudstreamcontinuewatching://<id>`,
//! and `csshare:<base64 source>?<base64 url>`.
use crate::repository::network_url;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf, sync::Mutex};

/// URL schemes this app handles. The packaged `.desktop` file must list exactly these (a test
/// keeps the two in step).
pub const HANDLED_SCHEMES: [&str; 5] = ["cloudstreamrepo", "cloudstreamsearch", "cloudstreamplayer", "cloudstreamcontinuewatching", "csshare"];

pub const MAX_ARGUMENT_BYTES: usize = 8 * 1024;
pub const MAX_QUEUED_ACTIONS: usize = 20;
const MAX_QUERY_CHARS: usize = 200;
const MAX_NAME_CHARS: usize = 200;

/// File types the app will open from "open with" or the command line. Anything else is refused,
/// so the app is not an arbitrary-file opener.
pub const MEDIA_EXTENSIONS: [&str; 17] = [
    "mkv", "mp4", "m4v", "webm", "avi", "mov", "ts", "m2ts", "mpg", "mpeg", "ogv", "mp3", "flac", "ogg", "opus", "m4a", "wav",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum EntryAction {
    /// Offer to add a repository. The input is passed to the normal repository resolver
    /// (short codes, redirects and validation live there), and is never added without asking.
    AddRepository { input: String },
    Search { query: String },
    /// A shared title: the source's display name and the title's page URL.
    OpenTitle { provider_name: String, url: String },
    /// An address to open: a title page of an installed source, or otherwise a direct stream
    /// the user is asked to confirm.
    OpenUrl { url: String },
    PlayUrl { url: String, name: Option<String> },
    PlayFile { path: String },
    ResumeLatest,
    /// Opened by clicking a new-episode notification. Created only inside the app, never by
    /// parsing text, so a link cannot produce it.
    OpenMedia { provider_id: String, url: String },
}

/// Percent-decode UTF-8. Invalid sequences are an error rather than silently kept, so a mangled
/// link is reported instead of acted on.
fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) else {
                    bail!("The link contains an invalid % sequence");
                };
                out.push(value);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| anyhow::anyhow!("The link is not valid UTF-8 text"))
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=').collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in cleaned {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => bail!("The shared link is not valid"),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn clean_text(value: &str, limit: usize, what: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("The {what} is empty");
    }
    if value.chars().count() > limit {
        bail!("The {what} is longer than {limit} characters");
    }
    if value.chars().any(char::is_control) {
        bail!("The {what} contains control characters");
    }
    Ok(value.to_string())
}

fn http_url(value: &str, what: &str) -> Result<String> {
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        bail!("The {what} is not a valid web address");
    }
    network_url(value).map_err(|e| anyhow::anyhow!("The {what} is not usable: {e}"))?;
    Ok(value.to_string())
}

/// Validate a local media file and resolve links, so what is confirmed is what is played.
pub fn checked_local_media(path: &std::path::Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Open files by their full path");
    }
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        bail!("{} is not a video or audio file this app opens", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "That file".into()));
    }
    let canonical = std::fs::canonicalize(path).map_err(|_| anyhow::anyhow!("The file {} does not exist or cannot be read", path.display()))?;
    let meta = std::fs::metadata(&canonical)?;
    if !meta.is_file() {
        bail!("{} is not a regular file", canonical.display());
    }
    // A resolved link must still point at a media file (a link named .mp4 to a document is refused).
    let resolved = canonical.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !MEDIA_EXTENSIONS.contains(&resolved.as_str()) {
        bail!("{} is not a video or audio file this app opens", canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "That file".into()));
    }
    Ok(canonical)
}

fn local_file(path: PathBuf) -> Result<EntryAction> {
    Ok(EntryAction::PlayFile { path: checked_local_media(&path)?.display().to_string() })
}

/// Turn one argument into one action.
pub fn parse_entry(argument: &str) -> Result<EntryAction> {
    let argument = argument.trim();
    if argument.is_empty() {
        bail!("The link is empty");
    }
    if argument.len() > MAX_ARGUMENT_BYTES {
        bail!("The link is too long");
    }
    if argument.chars().any(char::is_control) {
        bail!("The link contains control characters");
    }
    let lower = argument.to_ascii_lowercase();

    if lower.starts_with("https://cs.repo") {
        let Some((_, rest)) = argument.split_once('?') else { bail!("This repository link has no address") };
        let rest = rest.trim();
        if rest.is_empty() {
            bail!("This repository link has no address");
        }
        let url = format!("https://{rest}");
        http_url(&url, "repository address")?;
        return Ok(EntryAction::AddRepository { input: url });
    }
    if lower.starts_with("cloudstreamrepo://") {
        let rest = &argument["cloudstreamrepo://".len()..];
        if rest.trim().is_empty() {
            bail!("This repository link has no address");
        }
        // Short codes ("!code", "cutt.ly/code") and full addresses are both resolved later by the
        // repository code; only the shape is checked here.
        if rest.contains(char::is_whitespace) {
            bail!("The repository address is not valid");
        }
        return Ok(EntryAction::AddRepository { input: argument.to_string() });
    }
    if lower.starts_with("cloudstreamsearch://") {
        let query = percent_decode(&argument["cloudstreamsearch://".len()..])?;
        return Ok(EntryAction::Search { query: clean_text(&query, MAX_QUERY_CHARS, "search")? });
    }
    if lower.starts_with("cloudstreamplayer://") {
        let rest = &argument["cloudstreamplayer://".len()..];
        let (target, query) = rest.split_once('?').map(|(t, q)| (t, Some(q))).unwrap_or((rest, None));
        let url = http_url(&percent_decode(target)?, "stream address")?;
        let name = query
            .into_iter()
            .flat_map(|q| q.split('&'))
            .find_map(|pair| pair.strip_prefix("name="))
            .map(|n| percent_decode(n).and_then(|n| clean_text(&n, MAX_NAME_CHARS, "name")))
            .transpose()?;
        return Ok(EntryAction::PlayUrl { url, name });
    }
    if lower.starts_with("cloudstreamcontinuewatching://") {
        let id = &argument["cloudstreamcontinuewatching://".len()..];
        if id.eq_ignore_ascii_case("latest") {
            return Ok(EntryAction::ResumeLatest);
        }
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
            bail!("This link points to an item number from the Android app, which cannot be matched here. Use \"cloudstreamcontinuewatching://latest\" to resume the last thing you watched");
        }
        bail!("This continue-watching link is not valid");
    }
    if lower.starts_with("csshare:") {
        let data = &argument["csshare:".len()..];
        let Some((source, url)) = data.split_once('?') else { bail!("This shared link is incomplete") };
        let source = String::from_utf8(base64_decode(source)?).map_err(|_| anyhow::anyhow!("The shared link is not valid"))?;
        let url = String::from_utf8(base64_decode(url)?).map_err(|_| anyhow::anyhow!("The shared link is not valid"))?;
        return Ok(EntryAction::OpenTitle { provider_name: clean_text(&source, MAX_NAME_CHARS, "source name")?, url: http_url(&url, "shared address")? });
    }
    if lower.starts_with("cloudstreamapp:") {
        bail!("Account sign-in links are handled by the sign-in flow and cannot be opened this way");
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(EntryAction::OpenUrl { url: http_url(argument, "web address")? });
    }
    if lower.starts_with("file://") {
        let path = percent_decode(argument["file://".len()..].trim_start_matches("localhost"))?;
        return local_file(PathBuf::from(path));
    }
    if lower.starts_with("magnet:") {
        bail!("Magnet links need the torrent backend, which is not available in this version");
    }
    if let Some(scheme_end) = argument.find(':') {
        let scheme = &argument[..scheme_end];
        let looks_like_scheme = scheme.len() > 1 && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
        if looks_like_scheme && !argument.starts_with('/') {
            bail!("Links starting with \"{scheme}:\" are not supported");
        }
    }
    local_file(PathBuf::from(argument))
}

/// A parsed batch: valid actions in order, and a message for every argument that was refused.
#[derive(Debug, Default)]
pub struct ParsedArguments {
    pub actions: Vec<EntryAction>,
    pub errors: Vec<String>,
}

/// Parse command-line arguments (`argv[1..]`). Flags are not links and are skipped.
pub fn parse_arguments(arguments: &[String]) -> ParsedArguments {
    let mut parsed = ParsedArguments::default();
    for argument in arguments {
        if argument.starts_with("--") && !argument.contains(':') && !argument.contains('/') {
            continue;
        }
        match parse_entry(argument) {
            Ok(action) => parsed.actions.push(action),
            Err(error) => parsed.errors.push(error.to_string()),
        }
    }
    parsed
}

/// Actions waiting for the interface. A launch can deliver links before the window, storage and
/// worker are ready; they wait here and are handed over once, in order.
#[derive(Default)]
pub struct EntryQueue {
    pending: Mutex<VecDeque<EntryAction>>,
}

impl EntryQueue {
    /// Queue an action. Returns `false` when it was dropped because the queue is full or the same
    /// action is already waiting (a double-click on a link must not act twice).
    pub fn push(&self, action: EntryAction) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.contains(&action) || pending.len() >= MAX_QUEUED_ACTIONS {
            return false;
        }
        pending.push_back(action);
        true
    }

    pub fn take(&self) -> Vec<EntryAction> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
