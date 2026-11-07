//! HLS playlist handling for offline downloads (T21). Everything here is pure: no network, no
//! files, so the rules can be tested exhaustively with playlist text.
//!
//! What is supported, and what is refused instead of producing a broken file:
//!
//! * master playlists with several quality levels, separate audio renditions and subtitle
//!   renditions; media playlists with relative or absolute segment addresses, byte ranges,
//!   initialisation segments (`EXT-X-MAP`), discontinuities and AES-128 (`METHOD=AES-128`,
//!   identity key format) encryption;
//! * refused with a plain reason: live and still-growing playlists, key-frame-only playlists,
//!   `SAMPLE-AES` and every other encryption method or key format (DRM), playlists that use
//!   variables, and anything with an unreasonable number of segments.
//!
//! Segment addresses and keys are never written to disk by this module: the local playlist that
//! is produced refers only to local, relative file names.
use crate::repository::network_url;
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use url::Url;

pub const MAX_PLAYLIST_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 40_000;
pub const MAX_VARIANTS: usize = 200;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HlsError {
    /// A readable playlist that this app deliberately does not download. Safe to show as is.
    Unsupported(String),
    /// Not a playlist this app can read. Safe to show as is.
    Invalid(String),
}

impl std::fmt::Display for HlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for HlsError {}

fn unsupported<T>(message: &str) -> Result<T, HlsError> {
    Err(HlsError::Unsupported(message.to_string()))
}
fn invalid<T>(message: &str) -> Result<T, HlsError> {
    Err(HlsError::Invalid(message.to_string()))
}

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub uri: Url,
    pub bandwidth: u64,
    pub average_bandwidth: Option<u64>,
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
    pub audio: Option<String>,
    pub subtitles: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenditionKind {
    Audio,
    Subtitles,
    Video,
    ClosedCaptions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rendition {
    pub kind: RenditionKind,
    pub group: String,
    pub name: String,
    pub language: Option<String>,
    /// `None` means the rendition is carried inside the variant's own media.
    pub uri: Option<Url>,
    pub default: bool,
    pub autoselect: bool,
    pub forced: bool,
    pub channels: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Master {
    pub variants: Vec<Variant>,
    pub renditions: Vec<Rendition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub uri: Url,
    /// Explicit initialisation vector; otherwise the segment's sequence number is used.
    pub iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRef {
    pub uri: Url,
    /// `(length, offset)`.
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: Url,
    pub duration: f64,
    /// `(length, offset)`.
    pub range: Option<(u64, u64)>,
    pub sequence: u64,
    pub discontinuity: bool,
    pub key: Option<Key>,
    pub map: Option<MapRef>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaPlaylist {
    pub version: u32,
    pub target_duration: f64,
    pub media_sequence: u64,
    pub segments: Vec<Segment>,
}

impl MediaPlaylist {
    pub fn total_duration(&self) -> f64 {
        self.segments.iter().map(|s| s.duration).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master(Master),
    Media(MediaPlaylist),
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

/// Split an attribute list (`KEY=value,KEY="quoted, value"`) into upper-cased keys.
pub fn attributes(list: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let (mut key, mut value) = (String::new(), String::new());
    let (mut in_key, mut quoted) = (true, false);
    for c in list.chars() {
        match c {
            '"' if !in_key => quoted = !quoted,
            ',' if !quoted => {
                if !key.trim().is_empty() {
                    out.insert(key.trim().to_ascii_uppercase(), value.trim().to_string());
                }
                key.clear();
                value.clear();
                in_key = true;
            }
            '=' if in_key => in_key = false,
            _ if in_key => key.push(c),
            _ => value.push(c),
        }
    }
    if !key.trim().is_empty() {
        out.insert(key.trim().to_ascii_uppercase(), value.trim().to_string());
    }
    out
}

fn resolve(base: &Url, reference: &str) -> Result<Url, HlsError> {
    let mut url = base.join(reference.trim()).map_err(|_| HlsError::Invalid("A playlist entry has an invalid address".into()))?;
    url.set_fragment(None);
    network_url(url.as_str()).map_err(|_| HlsError::Invalid("A playlist entry uses an address that cannot be downloaded".into()))
}

fn parse_range(text: &str) -> Option<(u64, Option<u64>)> {
    let (length, offset) = match text.split_once('@') {
        Some((length, offset)) => (length, Some(offset.parse().ok()?)),
        None => (text, None),
    };
    let length: u64 = length.parse().ok()?;
    if length == 0 {
        return None;
    }
    Some((length, offset))
}

fn parse_iv(text: &str) -> Option<[u8; 16]> {
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    if hex.is_empty() || hex.len() > 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u128::from_str_radix(hex, 16).ok()?;
    Some(value.to_be_bytes())
}

fn body_lines(text: &str) -> Result<Vec<&str>, HlsError> {
    if text.len() > MAX_PLAYLIST_BYTES {
        return unsupported("The playlist is larger than this app will download");
    }
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return invalid("The address did not return a playlist");
    }
    Ok(lines.collect())
}

/// Parse either kind of playlist; `base` is the address the text was fetched from.
pub fn parse(text: &str, base: &Url) -> Result<Playlist, HlsError> {
    let lines = body_lines(text)?;
    if lines.iter().any(|line| line.starts_with("#EXT-X-STREAM-INF")) {
        parse_master_lines(&lines, base).map(Playlist::Master)
    } else if lines.iter().any(|line| line.starts_with("#EXTINF")) {
        parse_media_lines(&lines, base).map(Playlist::Media)
    } else {
        invalid("The playlist lists no video")
    }
}

pub fn parse_master(text: &str, base: &Url) -> Result<Master, HlsError> {
    parse_master_lines(&body_lines(text)?, base)
}

pub fn parse_media(text: &str, base: &Url) -> Result<MediaPlaylist, HlsError> {
    parse_media_lines(&body_lines(text)?, base)
}

fn parse_master_lines(lines: &[&str], base: &Url) -> Result<Master, HlsError> {
    let mut master = Master::default();
    let mut pending: Option<BTreeMap<String, String>> = None;
    for line in lines {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attributes(rest));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = attributes(rest);
            let kind = match a.get("TYPE").map(|v| v.to_ascii_uppercase()).as_deref() {
                Some("AUDIO") => RenditionKind::Audio,
                Some("SUBTITLES") => RenditionKind::Subtitles,
                Some("VIDEO") => RenditionKind::Video,
                Some("CLOSED-CAPTIONS") => RenditionKind::ClosedCaptions,
                _ => continue,
            };
            let yes = |key: &str| a.get(key).is_some_and(|v| v.eq_ignore_ascii_case("YES"));
            let uri = match a.get("URI") {
                Some(uri) if !uri.is_empty() => Some(resolve(base, uri)?),
                _ => None,
            };
            master.renditions.push(Rendition {
                kind,
                group: a.get("GROUP-ID").cloned().unwrap_or_default(),
                name: a.get("NAME").cloned().unwrap_or_default(),
                language: a.get("LANGUAGE").filter(|v| !v.is_empty()).cloned(),
                uri,
                default: yes("DEFAULT"),
                autoselect: yes("AUTOSELECT"),
                forced: yes("FORCED"),
                channels: a.get("CHANNELS").cloned(),
            });
        } else if line.starts_with('#') {
            continue;
        } else if let Some(a) = pending.take() {
            if master.variants.len() >= MAX_VARIANTS {
                return unsupported("The playlist lists too many quality levels");
            }
            let resolution = a.get("RESOLUTION").and_then(|r| {
                let (w, h) = r.split_once(['x', 'X'])?;
                Some((w.parse().ok()?, h.parse().ok()?))
            });
            master.variants.push(Variant {
                uri: resolve(base, line)?,
                bandwidth: a.get("BANDWIDTH").and_then(|v| v.parse().ok()).unwrap_or(0),
                average_bandwidth: a.get("AVERAGE-BANDWIDTH").and_then(|v| v.parse().ok()),
                resolution,
                codecs: a.get("CODECS").cloned(),
                audio: a.get("AUDIO").cloned(),
                subtitles: a.get("SUBTITLES").cloned(),
            });
        }
    }
    if master.variants.is_empty() {
        return invalid("The playlist lists no video");
    }
    Ok(master)
}

#[derive(Debug, Clone)]
struct KeyDeclaration {
    method: String,
    uri: Option<String>,
    iv: Option<[u8; 16]>,
}

fn parse_media_lines(lines: &[&str], base: &Url) -> Result<MediaPlaylist, HlsError> {
    let mut playlist = MediaPlaylist::default();
    let mut ended = false;
    let mut kind: Option<String> = None;
    let mut pending_duration: Option<f64> = None;
    let mut pending_range: Option<(u64, Option<u64>)> = None;
    let mut discontinuity = false;
    let mut keys: BTreeMap<String, KeyDeclaration> = BTreeMap::new();
    let mut map: Option<MapRef> = None;
    // The end of the last byte range, per resource, for ranges that omit their offset.
    let mut last_range_end: Option<(String, u64)> = None;

    for line in lines {
        if let Some(rest) = line.strip_prefix("#EXT-X-VERSION:") {
            playlist.version = rest.trim().parse().unwrap_or(1);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            playlist.target_duration = rest.trim().parse().unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            playlist.media_sequence = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
            kind = Some(rest.trim().to_ascii_uppercase());
        } else if line.starts_with("#EXT-X-ENDLIST") {
            ended = true;
        } else if line.starts_with("#EXT-X-I-FRAMES-ONLY") {
            return unsupported("This playlist only contains key frames and cannot be downloaded");
        } else if line.starts_with("#EXT-X-DEFINE") {
            return unsupported("This playlist uses variables, which are not supported for downloads");
        } else if line.starts_with("#EXT-X-DISCONTINUITY-SEQUENCE") {
            continue;
        } else if line.starts_with("#EXT-X-DISCONTINUITY") {
            discontinuity = true;
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let duration = rest.split(',').next().unwrap_or("").trim().parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0 && *d < 3_600.0);
            match duration {
                Some(duration) => pending_duration = Some(duration),
                None => return invalid("A segment has an invalid duration"),
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            pending_range = Some(parse_range(rest.trim()).ok_or_else(|| HlsError::Invalid("A segment has an invalid byte range".into()))?);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attributes(rest);
            let format = a.get("KEYFORMAT").map(|v| v.to_ascii_lowercase()).unwrap_or_else(|| "identity".into());
            let method = a.get("METHOD").map(|v| v.to_ascii_uppercase()).unwrap_or_default();
            let iv = match a.get("IV") {
                Some(text) => Some(parse_iv(text).ok_or_else(|| HlsError::Invalid("A key has an invalid initialisation vector".into()))?),
                None => None,
            };
            if format == "identity" && a.get("KEYFORMATVERSIONS").is_some_and(|v| v != "1") {
                return unsupported("This stream uses a key format version that is not supported");
            }
            keys.insert(format, KeyDeclaration { method, uri: a.get("URI").cloned(), iv });
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attributes(rest);
            let uri = a.get("URI").ok_or_else(|| HlsError::Invalid("An initialisation segment has no address".into()))?;
            let range = match a.get("BYTERANGE") {
                Some(text) => {
                    let (length, offset) = parse_range(text).ok_or_else(|| HlsError::Invalid("An initialisation segment has an invalid byte range".into()))?;
                    Some((length, offset.unwrap_or(0)))
                }
                None => None,
            };
            map = Some(MapRef { uri: resolve(base, uri)?, range });
        } else if line.starts_with('#') {
            continue;
        } else {
            let duration = pending_duration.take().ok_or_else(|| HlsError::Invalid("A segment has no duration".into()))?;
            if playlist.segments.len() >= MAX_SEGMENTS {
                return unsupported("This playlist has too many segments to download");
            }
            let uri = resolve(base, line)?;
            let range = match pending_range.take() {
                None => None,
                Some((length, Some(offset))) => Some((length, offset)),
                Some((length, None)) => match &last_range_end {
                    Some((resource, end)) if resource == uri.as_str() => Some((length, *end)),
                    _ => return invalid("A segment's byte range has no starting point"),
                },
            };
            last_range_end = range.map(|(length, offset)| (uri.to_string(), offset.saturating_add(length)));
            let sequence = playlist.media_sequence + playlist.segments.len() as u64;
            playlist.segments.push(Segment { uri, duration, range, sequence, discontinuity: std::mem::take(&mut discontinuity), key: effective_key(&keys, base)?, map: map.clone() });
        }
    }
    if playlist.segments.is_empty() {
        return invalid("The playlist lists no video");
    }
    // A playlist that never says it has ended may still be growing. Only an explicit VOD is trusted.
    if !ended && kind.as_deref() != Some("VOD") {
        return unsupported("This is a live stream, which cannot be downloaded");
    }
    Ok(playlist)
}

/// Decide what protects the next segment. Only the identity key format with AES-128 is something
/// this app can undo; every other protection is reported, never half-handled.
fn effective_key(keys: &BTreeMap<String, KeyDeclaration>, base: &Url) -> Result<Option<Key>, HlsError> {
    if let Some(identity) = keys.get("identity") {
        match identity.method.as_str() {
            "NONE" => {}
            "AES-128" => {
                let uri = identity.uri.as_deref().ok_or_else(|| HlsError::Invalid("An encrypted stream has no key address".into()))?;
                return Ok(Some(Key { uri: resolve(base, uri)?, iv: identity.iv }));
            }
            "SAMPLE-AES" | "SAMPLE-AES-CTR" | "SAMPLE-AES-CENC" => return unsupported("This stream uses SAMPLE-AES encryption, which cannot be downloaded"),
            _ => return unsupported("This stream uses an encryption method that is not supported"),
        }
    }
    if keys.iter().any(|(format, key)| format != "identity" && key.method != "NONE") {
        return unsupported("This stream is protected by DRM and cannot be downloaded");
    }
    Ok(None)
}

// ---------------------------------------------------------------------------------------------
// Choosing what to download
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SelectionPolicy {
    /// The tallest picture wanted. `None` takes the best available.
    pub target_height: Option<u32>,
    /// Preferred audio languages, best first (`en`, `ja`, `eng`...). The default track is used
    /// when none matches.
    pub audio_languages: Vec<String>,
    /// Subtitle languages to keep. Provider-marked default tracks are also kept.
    pub subtitle_languages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub variant: Variant,
    /// A separate audio playlist, when the audio is not already inside the video.
    pub audio: Option<Rendition>,
    pub subtitles: Vec<Rendition>,
}

const LANGUAGES: &[(&str, &[&str])] = &[
    ("en", &["eng", "english"]),
    ("ja", &["jpn", "japanese"]),
    ("es", &["spa", "spanish"]),
    ("fr", &["fra", "fre", "french"]),
    ("de", &["deu", "ger", "german"]),
    ("it", &["ita", "italian"]),
    ("pt", &["por", "portuguese"]),
    ("ru", &["rus", "russian"]),
    ("ko", &["kor", "korean"]),
    ("zh", &["zho", "chi", "chinese"]),
    ("ar", &["ara", "arabic"]),
    ("hi", &["hin", "hindi"]),
    ("tr", &["tur", "turkish"]),
    ("nl", &["nld", "dut", "dutch"]),
    ("pl", &["pol", "polish"]),
    ("sv", &["swe", "swedish"]),
    ("id", &["ind", "indonesian"]),
    ("th", &["tha", "thai"]),
    ("vi", &["vie", "vietnamese"]),
    ("he", &["heb", "hebrew"]),
    ("fa", &["fas", "per", "persian"]),
    ("uk", &["ukr", "ukrainian"]),
    ("el", &["ell", "gre", "greek"]),
];

/// Reduce `en-US`, `eng` or `English` to `en`; unknown languages compare by their primary subtag.
pub fn language_code(tag: &str) -> String {
    let lower = tag.trim().to_ascii_lowercase();
    let primary = lower.split(['-', '_']).next().unwrap_or("").to_string();
    for (code, aliases) in LANGUAGES {
        if primary == *code || aliases.iter().any(|alias| *alias == primary || *alias == lower) {
            return (*code).to_string();
        }
    }
    primary
}

fn is_audio_only(codecs: &str) -> bool {
    let lower = codecs.to_ascii_lowercase();
    !["avc", "hvc", "hev", "vp09", "vp9", "av01", "dvh", "mp4v"].iter().any(|video| lower.contains(video))
}

pub fn select(master: &Master, policy: &SelectionPolicy) -> Result<Selection, HlsError> {
    let mut candidates: Vec<&Variant> = master.variants.iter().filter(|v| v.codecs.as_deref().is_none_or(|c| !is_audio_only(c))).collect();
    if candidates.is_empty() {
        candidates = master.variants.iter().collect();
    }
    let height = |v: &Variant| v.resolution.map_or(0, |(_, h)| h);
    let chosen: Option<&Variant> = match policy.target_height.filter(|h| *h > 0) {
        Some(target) => candidates
            .iter()
            .copied()
            .filter(|v| height(v) > 0 && height(v) <= target)
            .max_by_key(|v| (height(v), v.bandwidth))
            // Nothing at or below the target: the smallest available is closest to it.
            .or_else(|| candidates.iter().copied().min_by_key(|v| (if height(v) == 0 { u32::MAX } else { height(v) }, v.bandwidth))),
        None => candidates.iter().copied().max_by_key(|v| (v.bandwidth, height(v))),
    };
    let variant = match chosen {
        Some(v) => v.clone(),
        None => return invalid("The playlist lists no video"),
    };

    let in_group = |group: &Option<String>, kind: RenditionKind| -> Vec<&Rendition> {
        match group {
            Some(group) => master.renditions.iter().filter(|r| r.kind == kind && &r.group == group).collect(),
            None => Vec::new(),
        }
    };
    let preferred = |list: &[String], rendition: &Rendition| {
        list.iter().position(|wanted| rendition.language.as_deref().is_some_and(|l| language_code(l) == language_code(wanted)))
    };

    let audio_options = in_group(&variant.audio, RenditionKind::Audio);
    let audio = audio_options
        .iter()
        .filter_map(|r| preferred(&policy.audio_languages, r).map(|rank| (rank, *r)))
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, r)| r)
        .or_else(|| audio_options.iter().copied().find(|r| r.default))
        .or_else(|| audio_options.first().copied())
        // A rendition without an address is carried inside the video, so there is nothing to fetch.
        .filter(|r| r.uri.is_some())
        .cloned();

    let subtitles = in_group(&variant.subtitles, RenditionKind::Subtitles)
        .into_iter()
        .filter(|r| r.uri.is_some() && (r.default || preferred(&policy.subtitle_languages, r).is_some()))
        .cloned()
        .collect();
    Ok(Selection { variant, audio, subtitles })
}

// ---------------------------------------------------------------------------------------------
// Segments: decryption and content checks
// ---------------------------------------------------------------------------------------------

/// The initialisation vector for a segment: the playlist's, or its sequence number.
pub fn iv_for(segment: &Segment) -> [u8; 16] {
    segment.key.as_ref().and_then(|k| k.iv).unwrap_or_else(|| u128::from(segment.sequence).to_be_bytes())
}

pub fn decrypt_aes128(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Result<Vec<u8>, HlsError> {
    let key: [u8; 16] = key.try_into().map_err(|_| HlsError::Invalid("The decryption key is not 16 bytes".into()))?;
    Aes128CbcDec::new(&key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| HlsError::Invalid("A segment could not be decrypted with the key it was given".into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    MpegTs,
    Fmp4,
    Aac,
    Mp3,
    Ac3,
    WebVtt,
}

impl Container {
    /// The extension of the local file. The HLS demuxer only accepts known extensions, so the
    /// name is chosen from the content, never from the (often disguised) remote address.
    pub fn extension(self, initialisation: bool) -> &'static str {
        match self {
            Self::MpegTs => "ts",
            Self::Fmp4 if initialisation => "mp4",
            Self::Fmp4 => "m4s",
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
            Self::Ac3 => "ac3",
            Self::WebVtt => "vtt",
        }
    }
}

pub fn sniff(bytes: &[u8]) -> Option<Container> {
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    if text.starts_with(b"WEBVTT") {
        return Some(Container::WebVtt);
    }
    if bytes.len() >= 188 && bytes[0] == 0x47 && (bytes.len() < 377 || bytes[188] == 0x47) {
        return Some(Container::MpegTs);
    }
    if bytes.len() >= 8 && [&b"ftyp"[..], b"styp", b"moof", b"moov", b"sidx", b"emsg"].contains(&&bytes[4..8]) {
        return Some(Container::Fmp4);
    }
    let mut audio = bytes;
    if bytes.starts_with(b"ID3") && bytes.len() >= 10 {
        let size = bytes[6..10].iter().fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7f));
        audio = bytes.get(10 + size..).unwrap_or(&[]);
        if audio.len() < 2 {
            return Some(Container::Aac);
        }
    }
    match audio {
        [0xFF, b, ..] if b & 0xF6 == 0xF0 => Some(Container::Aac),
        [0xFF, b, ..] if b & 0xE0 == 0xE0 => Some(Container::Mp3),
        [0x0B, 0x77, ..] => Some(Container::Ac3),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Resume identity
// ---------------------------------------------------------------------------------------------

/// A hash of playlist structure and resource paths. Hosts and query strings can change when a
/// provider renews a signed address; changing a path or byte range invalidates saved parts.
/// Byte-for-byte source verification is also required before using saved parts.
fn resource_path(url: &Url) -> String {
    // The worker rewrites protected HLS entries to ephemeral loopback session URLs. The
    // encoded subpath contains the upstream URL; using its path keeps the fingerprint stable
    // across sessions. The downloader independently verifies the bytes before resuming.
    if url.host_str() == Some("127.0.0.1") && url.path().starts_with("/media/") {
        if let Some(path) = url.query_pairs().find_map(|(key, value)| {
            (key == "subpath").then(|| network_url(&value).ok().map(|upstream| upstream.path().to_owned())).flatten()
        }) {
            return path;
        }
    }
    url.path().to_owned()
}

pub fn structure_hash(label: &str, playlist: &MediaPlaylist) -> String {
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update((playlist.segments.len() as u64).to_be_bytes());
    for s in &playlist.segments {
        hasher.update(format!("{:?}|{:?}|{}|{}|{}|", s.duration, s.range, s.sequence, s.discontinuity, resource_path(&s.uri)).as_bytes());
        if let Some(key) = &s.key {
            hasher.update(format!("key:{}:{:?}|", resource_path(&key.uri), key.iv).as_bytes());
        }
        if let Some(map) = &s.map {
            hasher.update(format!("map:{}:{:?}|", resource_path(&map.uri), map.range).as_bytes());
        }
        hasher.update(b"\n");
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The exact size, when every segment declares a byte range.
pub fn ranged_bytes(playlist: &MediaPlaylist) -> Option<u64> {
    let exact: u64 = playlist.segments.iter().filter_map(|s| s.range.map(|r| r.0)).sum();
    (playlist.segments.iter().all(|s| s.range.is_some()) && exact > 0).then_some(exact)
}

/// Rough size in bytes before the download, used only for progress and a free-space check.
pub fn estimated_bytes(variant: &Variant, playlist: &MediaPlaylist) -> u64 {
    ranged_bytes(playlist).unwrap_or_else(|| {
        let rate = variant.average_bandwidth.filter(|b| *b > 0).unwrap_or(variant.bandwidth);
        ((rate as f64 / 8.0) * playlist.total_duration()) as u64
    })
}

// ---------------------------------------------------------------------------------------------
// The local, offline playlist
// ---------------------------------------------------------------------------------------------

pub struct LocalSegment {
    pub file: String,
    pub duration: f64,
    pub discontinuity: bool,
    /// The local initialisation file this segment needs, if any.
    pub map: Option<String>,
}

fn clean_attribute(value: &str) -> String {
    value.chars().filter(|c| !c.is_control() && *c != '"' && *c != ',').take(64).collect()
}

/// A media playlist that refers only to local files in the same folder. Keys are gone because
/// segments are stored already decrypted; byte ranges are gone because each range is its own file.
pub fn local_media_playlist(segments: &[LocalSegment], target_duration: f64) -> String {
    let uses_maps = segments.iter().any(|s| s.map.is_some());
    let mut out = format!("#EXTM3U\n#EXT-X-VERSION:{}\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n", if uses_maps { 7 } else { 3 }, target_duration.ceil().max(1.0) as u64);
    let mut current_map: Option<&str> = None;
    for segment in segments {
        if segment.discontinuity {
            out.push_str("#EXT-X-DISCONTINUITY\n");
            // A new timeline may carry a different initialisation segment; repeat it.
            current_map = None;
        }
        if segment.map.as_deref() != current_map {
            if let Some(map) = &segment.map {
                out.push_str(&format!("#EXT-X-MAP:URI=\"{map}\"\n"));
            }
            current_map = segment.map.as_deref();
        }
        out.push_str(&format!("#EXTINF:{:.6},\n{}\n", segment.duration, segment.file));
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

pub struct LocalRendition {
    pub kind: RenditionKind,
    pub group: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub playlist: String,
}

pub struct LocalMaster<'a> {
    pub variant: &'a Variant,
    pub video_playlist: String,
    pub audio: Option<LocalRendition>,
    pub subtitles: Vec<LocalRendition>,
}

pub fn local_master_playlist(master: &LocalMaster) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
    for r in master.audio.iter().chain(master.subtitles.iter()) {
        let kind = if r.kind == RenditionKind::Audio { "AUDIO" } else { "SUBTITLES" };
        out.push_str(&format!("#EXT-X-MEDIA:TYPE={kind},GROUP-ID=\"{}\",NAME=\"{}\",DEFAULT={},AUTOSELECT=YES", clean_attribute(&r.group), clean_attribute(&r.name), if r.default { "YES" } else { "NO" }));
        if let Some(language) = r.language.as_deref().map(clean_attribute).filter(|l| !l.is_empty()) {
            out.push_str(&format!(",LANGUAGE=\"{language}\""));
        }
        if r.forced {
            out.push_str(",FORCED=YES");
        }
        out.push_str(&format!(",URI=\"{}\"\n", r.playlist));
    }
    out.push_str(&format!("#EXT-X-STREAM-INF:BANDWIDTH={}", master.variant.bandwidth.max(1)));
    if let Some((w, h)) = master.variant.resolution {
        out.push_str(&format!(",RESOLUTION={w}x{h}"));
    }
    if let Some(codecs) = master.variant.codecs.as_deref().map(clean_attribute).filter(|c| !c.is_empty()) {
        out.push_str(&format!(",CODECS=\"{codecs}\""));
    }
    if let Some(audio) = &master.audio {
        out.push_str(&format!(",AUDIO=\"{}\"", clean_attribute(&audio.group)));
    }
    if let Some(subtitle) = master.subtitles.first() {
        out.push_str(&format!(",SUBTITLES=\"{}\"", clean_attribute(&subtitle.group)));
    }
    out.push_str(&format!("\n{}\n", master.video_playlist));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;

    fn base() -> Url {
        Url::parse("https://cdn.example/show/ep1/master.m3u8?token=abc").unwrap()
    }

    #[test]
    fn attribute_lists_keep_quoted_commas_and_ignore_case_of_names() {
        let a = attributes(r#"bandwidth=1280000,CODECS="avc1.64001f,mp4a.40.2",RESOLUTION=1280x720,NAME="a=b""#);
        assert_eq!(a["BANDWIDTH"], "1280000");
        assert_eq!(a["CODECS"], "avc1.64001f,mp4a.40.2");
        assert_eq!(a["NAME"], "a=b");
    }

    #[test]
    fn a_master_lists_variants_and_renditions_with_addresses_resolved() {
        let text = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"audio/en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"sub\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"https://subs.example/de.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"sub\"\nlow/index.m3u8\n\
            #EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=100000,URI=\"iframe.m3u8\"\n";
        let master = parse_master(text, &base()).unwrap();
        assert_eq!(master.variants.len(), 1);
        assert_eq!(master.variants[0].uri.as_str(), "https://cdn.example/show/ep1/low/index.m3u8");
        assert_eq!(master.variants[0].resolution, Some((640, 360)));
        assert_eq!(master.renditions[0].uri.as_ref().unwrap().as_str(), "https://cdn.example/show/ep1/audio/en.m3u8");
        assert_eq!(master.renditions[1].uri.as_ref().unwrap().host_str(), Some("subs.example"));
    }

    #[test]
    fn a_media_playlist_keeps_order_ranges_maps_discontinuities_and_continued_offsets() {
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
            #EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"\n\
            #EXTINF:6.0,\n#EXT-X-BYTERANGE:1000@720\nmedia.mp4\n\
            #EXTINF:5.5,\n#EXT-X-BYTERANGE:900\nmedia.mp4\n\
            #EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"init2.mp4\"\n\
            #EXTINF:4.0,\nother.mp4\n#EXT-X-ENDLIST\n";
        let playlist = parse_media(text, &base()).unwrap();
        assert_eq!(playlist.segments.len(), 3);
        assert_eq!(playlist.segments[0].range, Some((1000, 720)));
        assert_eq!(playlist.segments[1].range, Some((900, 1720)), "an omitted offset continues from the previous range");
        assert_eq!(playlist.segments[0].map.as_ref().unwrap().range, Some((720, 0)));
        assert!(playlist.segments[2].discontinuity && !playlist.segments[1].discontinuity);
        assert_eq!(playlist.segments[2].map.as_ref().unwrap().uri.path(), "/show/ep1/init2.mp4");
        assert_eq!(playlist.segments.iter().map(|s| s.sequence).collect::<Vec<_>>(), [10, 11, 12]);
        assert!((playlist.total_duration() - 15.5).abs() < 1e-9);
    }

    #[test]
    fn a_range_without_a_starting_point_is_rejected_when_nothing_precedes_it() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\n#EXT-X-BYTERANGE:1000\na.ts\n#EXT-X-ENDLIST\n";
        assert!(matches!(parse_media(text, &base()), Err(HlsError::Invalid(_))));
    }

    #[test]
    fn aes128_keys_are_tracked_per_segment_and_clear_segments_stay_clear() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:7\n\
            #EXTINF:4,\nclear.ts\n\
            #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x000102030405060708090A0B0C0D0E0F\n#EXTINF:4,\na.ts\n\
            #EXT-X-KEY:METHOD=AES-128,URI=\"https://keys.example/k2\"\n#EXTINF:4,\nb.ts\n\
            #EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\nc.ts\n#EXT-X-ENDLIST\n";
        let playlist = parse_media(text, &base()).unwrap();
        assert!(playlist.segments[0].key.is_none() && playlist.segments[3].key.is_none());
        let first = playlist.segments[1].key.as_ref().unwrap();
        assert_eq!(first.uri.path(), "/show/ep1/key.bin");
        assert_eq!(iv_for(&playlist.segments[1]), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(playlist.segments[2].key.as_ref().unwrap().uri.host_str(), Some("keys.example"));
        assert_eq!(iv_for(&playlist.segments[2]), 9u128.to_be_bytes(), "no IV: the segment's sequence number");
    }

    #[test]
    fn unsupported_protection_live_and_odd_playlists_are_refused_with_a_reason() {
        let header = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n";
        let cases = [
            ("#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n", "SAMPLE-AES"),
            ("#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"k\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n", "DRM"),
            ("#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n", "DRM"),
            ("#EXTINF:4,\na.ts\n", "live"),
            ("#EXT-X-PLAYLIST-TYPE:EVENT\n#EXTINF:4,\na.ts\n", "live"),
            ("#EXT-X-I-FRAMES-ONLY\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n", "key frames"),
            ("#EXT-X-DEFINE:NAME=\"x\",VALUE=\"y\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n", "variables"),
        ];
        for (body, expected) in cases {
            match parse_media(&format!("{header}{body}"), &base()) {
                Err(HlsError::Unsupported(message)) => assert!(message.contains(expected), "{message}"),
                other => panic!("expected {expected}: {other:?}"),
            }
        }
        assert!(matches!(parse("<html>nope</html>", &base()), Err(HlsError::Invalid(_))));
        assert!(matches!(parse("#EXTM3U\n#EXT-X-ENDLIST\n", &base()), Err(HlsError::Invalid(_))));
        // A multi-DRM playlist that also offers the plain AES key can be downloaded through that key.
        let both = format!("{header}#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"d\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n");
        assert!(parse_media(&both, &base()).unwrap().segments[0].key.is_some());
    }

    #[test]
    fn addresses_with_credentials_or_other_schemes_are_rejected() {
        for bad in ["ftp://x.example/a.ts", "https://user:pw@x.example/a.ts", "file:///etc/passwd"] {
            let text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\n{bad}\n#EXT-X-ENDLIST\n");
            assert!(parse_media(&text, &base()).is_err(), "{bad}");
        }
    }

    fn master_with(heights: &[(u32, u64)]) -> Master {
        Master {
            variants: heights
                .iter()
                .map(|(h, b)| Variant { uri: Url::parse(&format!("https://x.example/{h}.m3u8")).unwrap(), bandwidth: *b, average_bandwidth: None, resolution: Some((h * 16 / 9, *h)), codecs: Some("avc1.4d401e,mp4a.40.2".into()), audio: Some("aud".into()), subtitles: Some("sub".into()) })
                .collect(),
            renditions: vec![
                Rendition { kind: RenditionKind::Audio, group: "aud".into(), name: "English".into(), language: Some("eng".into()), uri: Some(Url::parse("https://x.example/en.m3u8").unwrap()), default: true, autoselect: true, forced: false, channels: None },
                Rendition { kind: RenditionKind::Audio, group: "aud".into(), name: "Japanese".into(), language: Some("ja".into()), uri: Some(Url::parse("https://x.example/ja.m3u8").unwrap()), default: false, autoselect: true, forced: false, channels: None },
                Rendition { kind: RenditionKind::Subtitles, group: "sub".into(), name: "English".into(), language: Some("en".into()), uri: Some(Url::parse("https://x.example/s-en.m3u8").unwrap()), default: false, autoselect: true, forced: false, channels: None },
                Rendition { kind: RenditionKind::Subtitles, group: "sub".into(), name: "German".into(), language: Some("de".into()), uri: Some(Url::parse("https://x.example/s-de.m3u8").unwrap()), default: false, autoselect: true, forced: false, channels: None },
                Rendition { kind: RenditionKind::Subtitles, group: "sub".into(), name: "Forced".into(), language: Some("fr".into()), uri: Some(Url::parse("https://x.example/s-fr.m3u8").unwrap()), default: true, autoselect: true, forced: true, channels: None },
            ],
        }
    }

    #[test]
    fn variant_choice_follows_the_height_policy_deterministically() {
        let master = master_with(&[(360, 800_000), (1080, 5_000_000), (720, 2_500_000)]);
        let pick = |target| select(&master, &SelectionPolicy { target_height: target, ..Default::default() }).unwrap().variant.resolution.unwrap().1;
        assert_eq!(pick(None), 1080, "best by default");
        assert_eq!(pick(Some(720)), 720);
        assert_eq!(pick(Some(900)), 720, "the tallest that does not exceed the target");
        assert_eq!(pick(Some(100)), 360, "nothing small enough: the closest is the smallest");
        let audio_only = Master { variants: vec![Variant { codecs: Some("mp4a.40.2".into()), resolution: None, bandwidth: 9_999_999, ..master.variants[0].clone() }, master.variants[0].clone()], ..master.clone() };
        assert_eq!(select(&audio_only, &SelectionPolicy::default()).unwrap().variant.bandwidth, 800_000, "audio-only variants are not picked for video");
    }

    #[test]
    fn audio_and_subtitle_choice_uses_languages_with_defaults_as_the_fallback() {
        let master = master_with(&[(720, 2_500_000)]);
        let default = select(&master, &SelectionPolicy::default()).unwrap();
        assert_eq!(default.audio.unwrap().name, "English", "the provider's default");
        assert_eq!(default.subtitles.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Forced"], "only default subtitles without a preference");
        let japanese = select(&master, &SelectionPolicy { audio_languages: vec!["jpn".into()], subtitle_languages: vec!["en-US".into()], ..Default::default() }).unwrap();
        assert_eq!(japanese.audio.unwrap().name, "Japanese", "ja and jpn are the same language");
        assert_eq!(japanese.subtitles.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["English", "Forced"]);
        assert_eq!(language_code("English"), "en");
        assert_eq!(language_code("pt-BR"), "pt");
        assert_eq!(language_code("tlh"), "tlh");
    }

    #[test]
    fn a_rendition_carried_inside_the_video_needs_no_download() {
        let mut master = master_with(&[(720, 2_500_000)]);
        for r in master.renditions.iter_mut().filter(|r| r.kind == RenditionKind::Audio) {
            r.uri = None;
        }
        assert!(select(&master, &SelectionPolicy::default()).unwrap().audio.is_none());
    }

    #[test]
    fn aes_segments_decrypt_and_wrong_keys_or_lengths_are_diagnosed() {
        let key = [7u8; 16];
        let iv = 5u128.to_be_bytes();
        let plain: Vec<u8> = (0..1000u32).map(|n| (n % 251) as u8).collect();
        let encrypted = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(&plain);
        assert_eq!(decrypt_aes128(&key, &iv, &encrypted).unwrap(), plain);
        assert!(decrypt_aes128(&[1u8; 16], &iv, &encrypted).map_or(true, |out| out != plain), "a wrong key never yields the original");
        assert!(decrypt_aes128(&key[..15], &iv, &encrypted).is_err());
        assert!(decrypt_aes128(&key, &iv, &encrypted[..encrypted.len() - 1]).is_err(), "a truncated segment is an error");
    }

    #[test]
    fn content_is_recognised_by_bytes_not_by_the_remote_file_name() {
        let mut ts = vec![0u8; 400];
        ts[0] = 0x47;
        ts[188] = 0x47;
        assert_eq!(sniff(&ts), Some(Container::MpegTs));
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypiso5"), Some(Container::Fmp4));
        assert_eq!(sniff(b"\x00\x00\x00\x18moof\0\0\0\0"), Some(Container::Fmp4));
        assert_eq!(sniff(b"\xEF\xBB\xBFWEBVTT\n\n"), Some(Container::WebVtt));
        assert_eq!(sniff(&[0xFF, 0xF1, 0x50, 0x80]), Some(Container::Aac));
        assert_eq!(sniff(&[0x0B, 0x77, 0x00]), Some(Container::Ac3));
        assert_eq!(sniff(b"<!DOCTYPE html><html>"), None);
        assert_eq!(sniff(b"{\"error\":\"expired\"}"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(Container::Fmp4.extension(true), "mp4");
        assert_eq!(Container::Fmp4.extension(false), "m4s");
    }

    #[test]
    fn the_structure_hash_ignores_renewed_addresses_but_not_a_changed_list() {
        let make = |host: &str, extra: bool| {
            let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nhttps://{host}/a.ts?sig=1\n#EXTINF:4,\nhttps://{host}/b.ts?sig=2\n");
            if extra {
                text.push_str("#EXTINF:3,\nc.ts\n");
            }
            text.push_str("#EXT-X-ENDLIST\n");
            parse_media(&text, &base()).unwrap()
        };
        assert_eq!(structure_hash("v", &make("a.example", false)), structure_hash("v", &make("b.example", false)));
        assert_ne!(structure_hash("v", &make("a.example", false)), structure_hash("v", &make("a.example", true)));
        assert_ne!(structure_hash("v", &make("a.example", false)), structure_hash("a", &make("a.example", false)));
    }

    #[test]
    fn resume_identity_detects_changed_paths_ranges_and_encryption() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:3\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"20@0\"\n#EXTINF:4,\n#EXT-X-BYTERANGE:100@20\nvideo.mp4\n#EXT-X-ENDLIST\n";
        let original = parse_media(text, &base()).unwrap();
        let original_hash = structure_hash("video", &original);
        let mut changed = original.clone();
        changed.segments[0].uri.set_path("/different.mp4");
        assert_ne!(original_hash, structure_hash("video", &changed));
        changed = original.clone();
        changed.segments[0].range = Some((100, 120));
        assert_ne!(original_hash, structure_hash("video", &changed));
        changed = original.clone();
        changed.segments[0].map.as_mut().unwrap().range = Some((20, 1));
        assert_ne!(original_hash, structure_hash("video", &changed));
        changed = original.clone();
        changed.segments[0].key.as_mut().unwrap().iv = Some([1; 16]);
        assert_ne!(original_hash, structure_hash("video", &changed));
        changed = original.clone();
        changed.segments[0].sequence += 1;
        assert_ne!(original_hash, structure_hash("video", &changed));
        changed = original;
        changed.segments[0].uri.set_query(Some("token=renewed"));
        assert_eq!(original_hash, structure_hash("video", &changed));
    }

    #[test]
    fn renewed_worker_sessions_keep_the_upstream_resource_identity() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nhttps://cdn.example/show/a.ts?token=old\n#EXT-X-ENDLIST\n";
        let mut first = parse_media(text, &base()).unwrap();
        first.segments[0].uri = Url::parse("http://127.0.0.1:123/media/session-one/resource-one?subpath=https%3A%2F%2Fcdn.example%2Fshow%2Fa.ts%3Ftoken%3Dold").unwrap();
        let mut renewed = first.clone();
        renewed.segments[0].uri = Url::parse("http://127.0.0.1:456/media/session-two/resource-two?subpath=https%3A%2F%2Fcdn.example%2Fshow%2Fa.ts%3Ftoken%3Dnew").unwrap();
        assert_eq!(structure_hash("video", &first), structure_hash("video", &renewed));
        renewed.segments[0].uri = Url::parse("http://127.0.0.1:456/media/session-two/resource-two?subpath=https%3A%2F%2Fcdn.example%2Fshow%2Fb.ts").unwrap();
        assert_ne!(structure_hash("video", &first), structure_hash("video", &renewed));
    }

    #[test]
    fn the_local_playlists_refer_only_to_local_files() {
        let media = local_media_playlist(
            &[
                LocalSegment { file: "seg-000000.m4s".into(), duration: 2.0, discontinuity: false, map: Some("init-0.mp4".into()) },
                LocalSegment { file: "seg-000001.m4s".into(), duration: 2.0, discontinuity: false, map: Some("init-0.mp4".into()) },
                LocalSegment { file: "seg-000002.m4s".into(), duration: 1.5, discontinuity: true, map: Some("init-1.mp4".into()) },
            ],
            2.0,
        );
        assert_eq!(media.matches("#EXT-X-MAP").count(), 2, "one map per run of segments that share it");
        assert!(media.contains("#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"init-1.mp4\""));
        assert!(media.ends_with("#EXT-X-ENDLIST\n") && !media.contains("http") && !media.contains("EXT-X-KEY") && !media.contains("BYTERANGE"));
        let variant = master_with(&[(720, 2_500_000)]).variants.remove(0);
        let master = local_master_playlist(&LocalMaster {
            variant: &variant,
            video_playlist: "video/index.m3u8".into(),
            audio: Some(LocalRendition { kind: RenditionKind::Audio, group: "aud".into(), name: "Eng\"lish".into(), language: Some("en".into()), default: true, forced: false, playlist: "audio/index.m3u8".into() }),
            subtitles: vec![LocalRendition { kind: RenditionKind::Subtitles, group: "sub".into(), name: "German".into(), language: Some("de".into()), default: false, forced: false, playlist: "subs-0/index.m3u8".into() }],
        });
        assert!(master.contains("NAME=\"English\"") && master.contains("AUDIO=\"aud\"") && master.contains("SUBTITLES=\"sub\""));
        assert!(master.contains("RESOLUTION=1280x720") && master.ends_with("video/index.m3u8\n"));
        assert!(!master.contains("x.example"));
    }
}
