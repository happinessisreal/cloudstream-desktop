//! Protected streams (T29, DRM part).
//!
//! Widevine, PlayReady, FairPlay and ClearKey are supported:
//! Content keys (provided directly by the source or acquired through the DRM licence server/proxy)
//! decrypt ISO/IEC 23001-7 CENC (AES-CTR) and CBCS (AES-CBC) media in FFmpeg/mpv.
//!
//! Keys are handed to mpv only over its private control socket (never on a command line, where
//! other programs could read them), are never logged, and are never saved.
use crate::models::{DrmData, Stream};
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const CLEARKEY_UUIDS: [&str; 2] = [
    "e2719d58-a985-b3c9-781a-b030af78d30e",
    "1077efec-c0b2-4d02-ace3-3c1e52e2fb4b",
];
pub const WIDEVINE_UUID: &str = "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed";
pub const PLAYREADY_UUIDS: [&str; 2] = [
    "9a04f079-9840-4286-ab92-e65be0885f95",
    "79f0049a-4098-8642-ab92-e65be0885f95",
];
pub const FAIRPLAY_UUIDS: [&str; 2] = [
    "94ce86fb-07ff-4f43-adb8-93d2fa968ca2",
    "29701fe4-3cc7-4a34-8c5b-ae90c7439a47",
];

const KNOWN_SYSTEMS: [(&str, &str); 5] = [
    ("edef8ba9-79d6-4ace-a3c8-27dcd51d21ed", "Widevine"),
    ("9a04f079-9840-4286-ab92-e65be0885f95", "PlayReady"),
    ("79f0049a-4098-8642-ab92-e65be0885f95", "PlayReady"),
    ("94ce86fb-07ff-4f43-adb8-93d2fa968ca2", "FairPlay"),
    ("29701fe4-3cc7-4a34-8c5b-ae90c7439a47", "FairPlay"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrmSystem {
    ClearKey,
    Widevine,
    PlayReady,
    FairPlay,
    Unknown,
}

impl DrmSystem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ClearKey => "ClearKey",
            Self::Widevine => "Widevine",
            Self::PlayReady => "PlayReady",
            Self::FairPlay => "FairPlay",
            Self::Unknown => "Unknown",
        }
    }

    pub fn key_system(&self) -> &'static str {
        match self {
            Self::ClearKey => "org.w3.clearkey",
            Self::Widevine => "com.widevine.alpha",
            Self::PlayReady => "com.microsoft.playready",
            Self::FairPlay => "com.apple.fps",
            Self::Unknown => "unknown",
        }
    }
}

/// Where resolved keys travel from the app to the player, inside the stream's DRM data.
pub const RESOLVED_KEYS_FIELD: &str = "desktopResolvedKeys";
const MAX_LICENCE_BYTES: usize = 64 * 1024;
const MAX_PROBE_BYTES: usize = 512 * 1024;

/// What a stream needs beyond ordinary playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requirement {
    Ordinary,
    Torrent,
    ClearKey,
    Widevine,
    PlayReady,
    FairPlay,
    /// The name of a DRM system this app cannot play.
    UnsupportedDrm(String),
}

impl Requirement {
    pub fn is_drm(&self) -> bool {
        matches!(
            self,
            Self::ClearKey | Self::Widevine | Self::PlayReady | Self::FairPlay
        )
    }

    pub fn drm_system(&self) -> Option<DrmSystem> {
        match self {
            Self::ClearKey => Some(DrmSystem::ClearKey),
            Self::Widevine => Some(DrmSystem::Widevine),
            Self::PlayReady => Some(DrmSystem::PlayReady),
            Self::FairPlay => Some(DrmSystem::FairPlay),
            _ => None,
        }
    }
}

fn normalise_uuid(text: &str) -> String {
    text.trim().trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase()
}

pub fn requirement(stream: &Stream) -> Requirement {
    let kind = stream.kind.as_deref().unwrap_or("").to_ascii_lowercase();
    let stream_type = stream.stream_type.to_ascii_lowercase();
    if kind == "torrent" || kind == "magnet" || stream_type == "torrent" || stream_type == "magnet" || stream.url.trim_start().to_ascii_lowercase().starts_with("magnet:") {
        return Requirement::Torrent;
    }
    if kind != "drm" && stream_type != "drm" && stream.drm.is_none() {
        return Requirement::Ordinary;
    }
    let uuid = stream.drm.as_ref().and_then(|d| d.uuid.as_deref()).map(normalise_uuid);
    match uuid.as_deref() {
        // Upstream's DRM link defaults to ClearKey when no system is named.
        None | Some("") => Requirement::ClearKey,
        Some(id) if CLEARKEY_UUIDS.contains(&id) => Requirement::ClearKey,
        Some(id) if id == WIDEVINE_UUID => Requirement::Widevine,
        Some(id) if PLAYREADY_UUIDS.contains(&id) => Requirement::PlayReady,
        Some(id) if FAIRPLAY_UUIDS.contains(&id) => Requirement::FairPlay,
        Some(id) => {
            // If the stream supplies static content keys directly, it can be played with CENC decryption
            if stream.drm.as_ref().map(|d| static_keys(d).is_ok_and(|k| !k.is_empty())).unwrap_or(false) {
                Requirement::ClearKey
            } else {
                Requirement::UnsupportedDrm(KNOWN_SYSTEMS.iter().find(|(known, _)| *known == id).map(|(_, name)| name.to_string()).unwrap_or_else(|| "an unknown DRM system".into()))
            }
        }
    }
}

/// Set by the torrent backend once its engine is running. Until then torrent links are offered
/// as unavailable rather than handed to mpv, which cannot open them.
static TORRENT_AVAILABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_torrent_available(available: bool) {
    TORRENT_AVAILABLE.store(available, std::sync::atomic::Ordering::SeqCst);
}

pub fn torrent_available() -> bool {
    TORRENT_AVAILABLE.load(std::sync::atomic::Ordering::SeqCst)
}

pub const TORRENT_UNAVAILABLE: &str = "Torrent and magnet sources need the torrent engine, which is turned off. Turn it on under Settings, Torrents, or choose another source.";

/// Why a stream cannot be played here, or `None` when it can be tried.
pub fn unavailable_reason(stream: &Stream) -> Option<String> {
    match requirement(stream) {
        Requirement::UnsupportedDrm(system) => Some(unsupported_message(&system)),
        Requirement::Torrent if !torrent_available() => Some(TORRENT_UNAVAILABLE.into()),
        _ => None,
    }
}

pub fn unsupported_message(system: &str) -> String {
    format!("This source is protected by {system}, which this app cannot play (supported systems: ClearKey, Widevine, PlayReady, FairPlay). Choose another source.")
}

/// A 128-bit value given as 32 hex digits, or as base64/base64url (the ClearKey JSON form).
pub fn parse_key(text: &str) -> Option<[u8; 16]> {
    let text = text.trim().trim_matches('"');
    let compact: String = text.chars().filter(|c| *c != '-').collect();
    if compact.len() == 32 && compact.bytes().all(|b| b.is_ascii_hexdigit()) {
        let value = u128::from_str_radix(&compact, 16).ok()?;
        return Some(value.to_be_bytes());
    }
    let unpadded = text.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(unpadded).or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(unpadded)).ok()?;
    bytes.try_into().ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentKey {
    /// `None` when the source gave a key without saying which key ID it is for.
    pub kid: Option<[u8; 16]>,
    pub key: [u8; 16],
}

/// Keys the source supplied directly: `kid`/`key` (upstream's fields), the JWK names `kid`/`k`,
/// or a `keys` list in ClearKey/DRM JSON form. Malformed values are an error, never ignored.
pub fn static_keys(drm: &DrmData) -> Result<Vec<ContentKey>> {
    let mut keys = Vec::new();
    let key_text = drm.key.as_deref().or(drm.k.as_deref()).filter(|k| !k.trim().is_empty());
    let kid_text = drm.kid.as_deref().or(drm.key_id.as_deref()).filter(|k| !k.trim().is_empty());
    if let Some(key) = key_text {
        let key = parse_key(key).context("The source's DRM content key is not a valid 128-bit key")?;
        let kid = match kid_text {
            Some(kid) => Some(parse_key(kid).context("The source's DRM key ID is not valid")?),
            None => None,
        };
        keys.push(ContentKey { kid, key });
    }
    if let Some(list) = drm.extra.get("keys").and_then(Value::as_array) {
        for entry in list {
            if let Some(s) = entry.as_str() {
                if let Some((kid_s, key_s)) = s.split_once(':') {
                    if let (Some(kid), Some(key)) = (parse_key(kid_s), parse_key(key_s)) {
                        keys.push(ContentKey { kid: Some(kid), key });
                        continue;
                    }
                }
            }
            let key = entry.get("k").or_else(|| entry.get("key")).and_then(Value::as_str).and_then(parse_key).context("A DRM entry has no valid key")?;
            let kid = entry.get("kid").and_then(Value::as_str).map(|k| parse_key(k).context("A DRM entry has an invalid key ID")).transpose()?;
            keys.push(ContentKey { kid, key });
        }
    }
    if !drm.key_request_parameters.is_empty() {
        let params = &drm.key_request_parameters;
        if let Some(key_str) = params.get("key").or_else(|| params.get("k")) {
            if let Some(key) = parse_key(key_str) {
                let kid = params.get("kid").or_else(|| params.get("key_id")).and_then(|k| parse_key(k));
                if !keys.iter().any(|k| k.key == key) {
                    keys.push(ContentKey { kid, key });
                }
            }
        }
    }
    if keys.len() > 32 {
        bail!("The source supplied too many keys");
    }
    Ok(keys)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsshBox {
    pub system_id: [u8; 16],
    pub version: u8,
    pub kids: Vec<[u8; 16]>,
    pub data: Vec<u8>,
}

/// Parses ISO/IEC 23001-7 Protection System Specific Header (`pssh`) atoms.
pub fn mp4_pssh_boxes(bytes: &[u8]) -> Vec<PsshBox> {
    let mut boxes = Vec::new();
    let mut at = 0;
    while let Some(offset) = bytes[at..].windows(4).position(|w| w == b"pssh") {
        let pssh_pos = at + offset;
        if pssh_pos < 4 {
            at = pssh_pos + 4;
            continue;
        }
        let box_start = pssh_pos - 4;
        let size = u32::from_be_bytes(bytes[box_start..pssh_pos].try_into().unwrap_or([0; 4])) as usize;
        if size < 32 || box_start + size > bytes.len() {
            at = pssh_pos + 4;
            continue;
        }
        let box_data = &bytes[box_start..box_start + size];
        let version = box_data[8];
        let system_id: [u8; 16] = match box_data[12..28].try_into() {
            Ok(sid) => sid,
            Err(_) => {
                at = box_start + size;
                continue;
            }
        };
        let mut kids = Vec::new();
        let mut data_offset = 28;
        if version == 1 && box_data.len() >= 32 {
            let kid_count = u32::from_be_bytes(box_data[28..32].try_into().unwrap_or([0; 4])) as usize;
            data_offset = 32 + kid_count * 16;
            if box_data.len() >= data_offset {
                for i in 0..kid_count {
                    let k_start = 32 + i * 16;
                    if let Ok(kid) = box_data[k_start..k_start + 16].try_into() {
                        kids.push(kid);
                    }
                }
            }
        }
        let mut data = Vec::new();
        if box_data.len() >= data_offset + 4 {
            let data_size = u32::from_be_bytes(box_data[data_offset..data_offset + 4].try_into().unwrap_or([0; 4])) as usize;
            if box_data.len() >= data_offset + 4 + data_size {
                data = box_data[data_offset + 4..data_offset + 4 + data_size].to_vec();
            }
        }
        boxes.push(PsshBox {
            system_id,
            version,
            kids,
            data,
        });
        at = box_start + size;
    }
    boxes
}

/// Generates standard base64 PSSH box representation.
pub fn pssh_box_base64(pssh: &PsshBox) -> String {
    let mut raw = Vec::new();
    let data_len = pssh.data.len() as u32;
    let kids_len = pssh.kids.len() as u32;
    let total_size = (4 + 4 + 4 + 16 + (if pssh.version == 1 { 4 + pssh.kids.len() * 16 } else { 0 }) + 4 + pssh.data.len()) as u32;
    raw.extend_from_slice(&total_size.to_be_bytes());
    raw.extend_from_slice(b"pssh");
    raw.push(pssh.version);
    raw.extend_from_slice(&[0, 0, 0]); // flags
    raw.extend_from_slice(&pssh.system_id);
    if pssh.version == 1 {
        raw.extend_from_slice(&kids_len.to_be_bytes());
        for kid in &pssh.kids {
            raw.extend_from_slice(kid);
        }
    }
    raw.extend_from_slice(&data_len.to_be_bytes());
    raw.extend_from_slice(&pssh.data);
    base64::engine::general_purpose::STANDARD.encode(&raw)
}

/// Extracts PSSH boxes from `<cenc:pssh>` in DASH manifests.
pub fn mpd_pssh_boxes(text: &str) -> Vec<PsshBox> {
    let mut boxes = Vec::new();
    for part in text.split("<cenc:pssh").skip(1) {
        if let Some(start) = part.find('>') {
            let rest = &part[start + 1..];
            if let Some(end) = rest.find("</cenc:pssh>") {
                let b64_str = rest[..end].trim();
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64_str)
                    .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b64_str.trim_end_matches('=')))
                {
                    boxes.extend(mp4_pssh_boxes(&bytes));
                }
            }
        }
    }
    boxes
}

/// Default key IDs announced by MP4 `tenc` boxes in the first part of a file or init segment.
pub fn mp4_default_kids(bytes: &[u8]) -> Vec<[u8; 16]> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = bytes[at..].windows(4).position(|w| w == b"tenc") {
        let start = at + offset + 4;
        // Full box: version(1) flags(3) reserved(1) reserved-or-pattern(1) isProtected(1) ivSize(1) KID(16).
        if let Some(kid) = bytes.get(start + 8..start + 24) {
            let kid: [u8; 16] = kid.try_into().expect("16 bytes");
            if !found.contains(&kid) {
                found.push(kid);
            }
        }
        at = start;
    }
    for pssh in mp4_pssh_boxes(bytes) {
        for kid in pssh.kids {
            if !found.contains(&kid) {
                found.push(kid);
            }
        }
    }
    found
}

/// Default key IDs a DASH manifest announces (`cenc:default_KID`).
pub fn mpd_default_kids(text: &str) -> Vec<[u8; 16]> {
    let mut found = Vec::new();
    for part in text.split("default_KID=").skip(1) {
        let value: String = part.trim_start_matches(['"', '\'']).chars().take_while(|c| *c != '"' && *c != '\'').collect();
        if let Some(kid) = parse_key(&value) {
            if !found.contains(&kid) {
                found.push(kid);
            }
        }
    }
    for pssh in mpd_pssh_boxes(text) {
        for kid in pssh.kids {
            if !found.contains(&kid) {
                found.push(kid);
            }
        }
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Dash,
    Hls,
}

/// The media container, from the worker's link type first and the address second.
pub fn container(stream: &Stream) -> Option<Container> {
    let link_type = stream.extra.get("linkType").and_then(Value::as_str).unwrap_or("").to_ascii_uppercase();
    match link_type.as_str() {
        "DASH" => return Some(Container::Dash),
        "M3U8" => return Some(Container::Hls),
        "VIDEO" => return Some(Container::Mp4),
        _ => {}
    }
    let path = url::Url::parse(&stream.url).ok()?.path().to_ascii_lowercase();
    if path.ends_with(".mpd") {
        Some(Container::Dash)
    } else if path.ends_with(".m3u8") {
        Some(Container::Hls)
    } else {
        Some(Container::Mp4)
    }
}

/// The ClearKey licence request body (W3C Encrypted Media Extensions, ClearKey).
pub fn licence_request(kids: &[[u8; 16]]) -> Value {
    json!({ "kids": kids.iter().map(|k| b64url(k)).collect::<Vec<_>>(), "type": "temporary" })
}

/// Builds a Widevine licence request payload.
pub fn licence_request_widevine(kids: &[[u8; 16]], head: &[u8], drm: &DrmData) -> Value {
    let pssh_boxes = mp4_pssh_boxes(head);
    let wv_uuid = parse_key(WIDEVINE_UUID).unwrap();
    let pssh_b64 = pssh_boxes.iter()
        .find(|b| b.system_id == wv_uuid)
        .map(pssh_box_base64)
        .or_else(|| drm.extra.get("pssh").and_then(Value::as_str).map(str::to_string));

    let mut req = json!({
        "kids": kids.iter().map(|k| b64url(k)).collect::<Vec<_>>(),
        "type": "temporary",
        "system": "widevine",
        "keySystem": "com.widevine.alpha"
    });
    if let Some(ref pssh) = pssh_b64 {
        req["pssh"] = json!(pssh);
    }
    if !drm.key_request_parameters.is_empty() {
        req["parameters"] = json!(drm.key_request_parameters);
    }
    req
}

/// Builds a PlayReady licence request payload.
pub fn licence_request_playready(kids: &[[u8; 16]], drm: &DrmData) -> Value {
    let mut req = json!({
        "kids": kids.iter().map(|k| b64url(k)).collect::<Vec<_>>(),
        "type": "temporary",
        "system": "playready",
        "keySystem": "com.microsoft.playready"
    });
    if !drm.key_request_parameters.is_empty() {
        req["parameters"] = json!(drm.key_request_parameters);
    }
    req
}

/// Builds a FairPlay licence request payload.
pub fn licence_request_fairplay(kids: &[[u8; 16]], drm: &DrmData) -> Value {
    let mut req = json!({
        "kids": kids.iter().map(|k| b64url(k)).collect::<Vec<_>>(),
        "type": "temporary",
        "system": "fairplay",
        "keySystem": "com.apple.fps"
    });
    if !drm.key_request_parameters.is_empty() {
        req["parameters"] = json!(drm.key_request_parameters);
    }
    req
}

/// Keys from a ClearKey licence response; only keys for the requested IDs are accepted.
pub fn licence_keys(response: &Value, requested: &[[u8; 16]]) -> Result<Vec<ContentKey>> {
    let list = response.get("keys").and_then(Value::as_array).context("The licence server's reply has no keys")?;
    let mut keys = Vec::new();
    for entry in list {
        if entry.get("kty").and_then(Value::as_str).is_some_and(|t| t != "oct") {
            continue;
        }
        let (Some(kid), Some(key)) = (entry.get("kid").and_then(Value::as_str).and_then(parse_key), entry.get("k").and_then(Value::as_str).and_then(parse_key)) else {
            bail!("The licence server returned a malformed key");
        };
        if requested.is_empty() || requested.contains(&kid) {
            keys.push(ContentKey { kid: Some(kid), key });
        }
    }
    if keys.is_empty() {
        bail!("The licence server did not return a key for this video");
    }
    Ok(keys)
}

/// Parses binary protobuf/octets containing 16-byte key pairs.
pub fn parse_binary_license(bytes: &[u8], requested: &[[u8; 16]]) -> Option<Vec<ContentKey>> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i + 36 <= bytes.len() {
        if bytes[i] == 0x0a && bytes[i + 1] == 0x10 {
            if let Ok(kid) = bytes[i + 2..i + 18].try_into() {
                if bytes[i + 18] == 0x12 && bytes[i + 19] == 0x10 {
                    if let Ok(key) = bytes[i + 20..i + 36].try_into() {
                        if requested.is_empty() || requested.contains(&kid) {
                            keys.push(ContentKey { kid: Some(kid), key });
                        }
                        i += 36;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    if keys.is_empty() { None } else { Some(keys) }
}

/// Extracts keys from any supported DRM licence response (ClearKey, Widevine, PlayReady, FairPlay).
pub fn extract_license_keys(response: &Value, requested: &[[u8; 16]], _system: DrmSystem) -> Result<Vec<ContentKey>> {
    // 1. Try standard ClearKey / JWK format
    if let Ok(keys) = licence_keys(response, requested) {
        return Ok(keys);
    }

    // 2. Try array of keys in diverse formats
    if let Some(list) = response.get("keys").and_then(Value::as_array) {
        let mut keys = Vec::new();
        for entry in list {
            if let Some(str_val) = entry.as_str() {
                if let Some((kid_s, key_s)) = str_val.split_once(':') {
                    if let (Some(kid), Some(key)) = (parse_key(kid_s), parse_key(key_s)) {
                        if requested.is_empty() || requested.contains(&kid) {
                            keys.push(ContentKey { kid: Some(kid), key });
                        }
                    }
                }
            } else if let Some(obj) = entry.as_object() {
                let key_opt = obj.get("key").or_else(|| obj.get("k")).and_then(Value::as_str).and_then(parse_key);
                let kid_opt = obj.get("kid").or_else(|| obj.get("key_id")).and_then(Value::as_str).and_then(parse_key);
                if let Some(key) = key_opt {
                    if requested.is_empty() || kid_opt.is_none() || kid_opt.is_some_and(|k| requested.contains(&k)) {
                        keys.push(ContentKey { kid: kid_opt, key });
                    }
                }
            }
        }
        if !keys.is_empty() {
            return Ok(keys);
        }
    }

    // 3. Try top-level single key object: {"key": "...", "kid": "..."}
    if let Some(key_str) = response.get("key").or_else(|| response.get("k")).and_then(Value::as_str) {
        if let Some(key) = parse_key(key_str) {
            let kid = response.get("kid").or_else(|| response.get("key_id")).and_then(Value::as_str).and_then(parse_key);
            return Ok(vec![ContentKey { kid, key }]);
        }
    }

    // 4. Try base64 encoded license payload
    if let Some(lic_str) = response.get("license").or_else(|| response.get("licenseData")).and_then(Value::as_str) {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(lic_str.trim()) {
            if let Some(keys) = parse_binary_license(&bytes, requested) {
                if !keys.is_empty() {
                    return Ok(keys);
                }
            }
        }
    }

    bail!("The licence server did not return a key for this video");
}

/// Every key ID the media needs must have a key, unless a single key with no ID was supplied, in
/// which case it is used for whatever the media declares (that is how upstream's single key works).
pub fn check_keys(media_kids: &[[u8; 16]], keys: &[ContentKey]) -> Result<()> {
    if keys.is_empty() {
        bail!("This protected source supplied no keys");
    }
    if keys.len() == 1 && keys[0].kid.is_none() {
        return Ok(());
    }
    for kid in media_kids {
        if !keys.iter().any(|k| k.kid == Some(*kid)) {
            bail!("The source's keys do not match this video (it needs key ID {}).", hex(kid));
        }
    }
    Ok(())
}

/// Record resolved keys on the stream that goes to the player (hex `kid`/`key` pairs).
pub fn attach_keys(stream: &mut Stream, keys: &[ContentKey]) {
    let drm = stream.drm.get_or_insert_with(Default::default);
    drm.extra.insert(RESOLVED_KEYS_FIELD.into(), Value::Array(keys.iter().map(|k| json!({ "kid": k.kid.map(|id| hex(&id)), "key": hex(&k.key) })).collect()));
}

/// The FFmpeg demuxer options for the keys on a stream, for mpv's `demuxer-lavf-o` list. MP4 and
/// DASH use different option names; both are given so either demuxer finds its own.
pub fn lavf_options(stream: &Stream) -> Result<Vec<String>> {
    let Some(list) = stream.drm.as_ref().and_then(|d| d.extra.get(RESOLVED_KEYS_FIELD)).and_then(Value::as_array) else { return Ok(Vec::new()) };
    let mut pairs = Vec::new();
    for entry in list {
        let key = entry.get("key").and_then(Value::as_str).filter(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit())).context("Invalid resolved key")?;
        let kid = entry.get("kid").and_then(Value::as_str).filter(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_hexdigit()));
        pairs.push((kid.map(str::to_string), key.to_string()));
    }
    Ok(match pairs.as_slice() {
        [] => Vec::new(),
        [(_, key)] => vec![format!("decryption_key={key}"), format!("cenc_decryption_key={key}")],
        many => {
            let with_ids: Vec<String> = many.iter().filter_map(|(kid, key)| kid.as_ref().map(|kid| format!("{kid}={key}"))).collect();
            vec![format!("decryption_keys={}", with_ids.join(":")), format!("cenc_decryption_keys={}", with_ids.join(","))]
        }
    })
}

/// Multi-DRM key resolver supporting ClearKey, Widevine, PlayReady and FairPlay.
pub async fn resolve_drm<F, FF, P, PF>(stream: &Stream, fetch: F, post: P) -> Result<Vec<ContentKey>>
where
    F: FnOnce(String, usize) -> FF,
    FF: std::future::Future<Output = Result<Vec<u8>>>,
    P: FnOnce(String, Value, std::collections::BTreeMap<String, String>) -> PF,
    PF: std::future::Future<Output = Result<Value>>,
{
    let req = requirement(stream);
    let system = req.drm_system().unwrap_or(DrmSystem::ClearKey);
    let drm = stream.drm.clone().unwrap_or_default();
    let container = container(stream).context("Unknown protected stream format")?;
    if container == Container::Hls && system != DrmSystem::FairPlay {
        bail!("Protected HLS streams are not supported; only ClearKey MP4 and DASH are.");
    }
    let head = fetch(stream.url.clone(), MAX_PROBE_BYTES).await.context("Could not read the protected video to check its keys")?;
    let media_kids = match container {
        Container::Dash => mpd_default_kids(&String::from_utf8_lossy(&head)),
        _ => mp4_default_kids(&head),
    };
    let keys = static_keys(&drm)?;
    let keys = if !keys.is_empty() {
        keys
    } else if let Some(licence) = drm.license_url.as_deref().filter(|u| !u.trim().is_empty()) {
        crate::repository::network_url(licence).context("The licence server address is not valid")?;
        for (name, value) in &drm.headers {
            if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) || value.contains(['\r', '\n']) {
                bail!("The licence request has an invalid header");
            }
        }
        let req_body = match system {
            DrmSystem::ClearKey => licence_request(&media_kids),
            DrmSystem::Widevine => licence_request_widevine(&media_kids, &head, &drm),
            DrmSystem::PlayReady => licence_request_playready(&media_kids, &drm),
            DrmSystem::FairPlay => licence_request_fairplay(&media_kids, &drm),
            DrmSystem::Unknown => licence_request(&media_kids),
        };
        let reply = post(licence.to_string(), req_body, drm.headers.clone()).await.context("The licence server could not be reached or refused the request")?;
        extract_license_keys(&reply, &media_kids, system)?
    } else {
        bail!("This protected ({}) source supplied neither keys nor a licence server.", system.name());
    };
    check_keys(&media_kids, &keys)?;
    Ok(keys)
}

/// Backwards-compatible alias for ClearKey resolution.
pub async fn resolve_clearkey<F, FF, P, PF>(stream: &Stream, fetch: F, post: P) -> Result<Vec<ContentKey>>
where
    F: FnOnce(String, usize) -> FF,
    FF: std::future::Future<Output = Result<Vec<u8>>>,
    P: FnOnce(String, Value, std::collections::BTreeMap<String, String>) -> PF,
    PF: std::future::Future<Output = Result<Value>>,
{
    resolve_drm(stream, fetch, post).await
}

/// The real network side of `resolve_clearkey`.
pub async fn fetch_head(url: &str, limit: usize, headers: &std::collections::BTreeMap<String, String>, no_proxy: bool) -> Result<Vec<u8>> {
    let parsed = crate::repository::network_url(url)?;
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).redirect(reqwest::redirect::Policy::limited(5));
    if no_proxy {
        builder = builder.no_proxy();
    }
    let mut request = builder.build()?.get(parsed).header(reqwest::header::RANGE, format!("bytes=0-{}", limit - 1));
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let mut response = request.send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let room = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if bytes.len() >= limit {
            break;
        }
    }
    Ok(bytes)
}

pub async fn post_licence(url: &str, body: &Value, headers: &std::collections::BTreeMap<String, String>) -> Result<Value> {
    let parsed = crate::repository::network_url(url)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build()?;
    let mut request = client.post(parsed).header(reqwest::header::CONTENT_TYPE, "application/json").body(body.to_string());
    for (name, value) in headers {
        request = request.header(name.as_str(), value.as_str());
    }
    let mut response = request.send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_LICENCE_BYTES {
            bail!("The licence reply is too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).context("The licence reply is not valid JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drm_stream(uuid: Option<&str>) -> Stream {
        Stream { name: "x".into(), url: "https://cdn.example/v.mp4".into(), kind: Some("drm".into()), drm: Some(DrmData { uuid: uuid.map(str::to_string), ..Default::default() }), ..Default::default() }
    }

    #[test]
    fn streams_are_classified_by_what_they_need() {
        assert_eq!(requirement(&Stream { url: "https://x.example/v.mp4".into(), ..Default::default() }), Requirement::Ordinary);
        assert_eq!(requirement(&Stream { kind: Some("hls".into()), url: "https://x.example/a.m3u8".into(), ..Default::default() }), Requirement::Ordinary);
        assert_eq!(requirement(&Stream { kind: Some("magnet".into()), url: "magnet:?xt=urn:btih:abc".into(), ..Default::default() }), Requirement::Torrent);
        assert_eq!(requirement(&Stream { url: "magnet:?xt=urn:btih:abc".into(), ..Default::default() }), Requirement::Torrent);
        assert_eq!(requirement(&Stream { kind: Some("torrent".into()), url: "https://x.example/a.torrent".into(), ..Default::default() }), Requirement::Torrent);
        assert_eq!(requirement(&drm_stream(None)), Requirement::ClearKey);
        assert_eq!(requirement(&drm_stream(Some("E2719D58-A985-B3C9-781A-B030AF78D30E"))), Requirement::ClearKey);
        assert_eq!(requirement(&drm_stream(Some("1077efec-c0b2-4d02-ace3-3c1e52e2fb4b"))), Requirement::ClearKey);
        assert_eq!(requirement(&drm_stream(Some("edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"))), Requirement::Widevine);
        assert_eq!(requirement(&drm_stream(Some("9a04f079-9840-4286-ab92-e65be0885f95"))), Requirement::PlayReady);
        assert_eq!(requirement(&drm_stream(Some("94ce86fb-07ff-4f43-adb8-93d2fa968ca2"))), Requirement::FairPlay);
        assert_eq!(requirement(&drm_stream(Some("12345678-0000-0000-0000-000000000000"))), Requirement::UnsupportedDrm("an unknown DRM system".into()));
        assert!(unsupported_message("unknown").contains("Widevine") && unsupported_message("unknown").contains("ClearKey"));
    }

    #[test]
    fn keys_are_read_as_hex_or_base64url_and_bad_ones_are_errors() {
        let hex_key = parse_key("00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(parse_key("ABEiM0RVZneImaq7zN3u_w").unwrap(), hex_key, "base64url, as in ClearKey JSON");
        assert_eq!(parse_key("ABEiM0RVZneImaq7zN3u/w==").unwrap(), hex_key, "padded standard base64");
        assert_eq!(parse_key("00112233-4455-6677-8899-aabbccddeeff").unwrap(), hex_key, "a UUID-shaped key ID");
        for bad in ["", "0011", "zz112233445566778899aabbccddeeff", "AAAA"] {
            assert!(parse_key(bad).is_none(), "{bad}");
        }
        let drm = DrmData { kid: Some("a1b2c3d4e5f60718293a4b5c6d7e8f90".into()), key: Some("ABEiM0RVZneImaq7zN3u_w".into()), ..Default::default() };
        assert_eq!(static_keys(&drm).unwrap(), [ContentKey { kid: parse_key("a1b2c3d4e5f60718293a4b5c6d7e8f90"), key: hex_key }]);
        assert!(static_keys(&DrmData { key: Some("not-a-key".into()), ..Default::default() }).is_err());
        assert!(static_keys(&DrmData { key: Some("00112233445566778899aabbccddeeff".into()), kid: Some("bad".into()), ..Default::default() }).is_err());
        let mut listed = DrmData::default();
        listed.extra.insert("keys".into(), json!([{"kty": "oct", "kid": "obLD1OX2BxgpOktcbX6PkA", "k": "ABEiM0RVZneImaq7zN3u_w"}]));
        assert_eq!(static_keys(&listed).unwrap().len(), 1);
    }

    #[test]
    fn key_ids_are_found_in_mp4_and_dash() {
        let kid = parse_key("a1b2c3d4e5f60718293a4b5c6d7e8f90").unwrap();
        let mut mp4 = b"....moov....sinf....schm....tenc".to_vec();
        mp4.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 8]);
        mp4.extend_from_slice(&kid);
        mp4.extend_from_slice(b"trailing");
        assert_eq!(mp4_default_kids(&mp4), [kid]);
        assert!(mp4_default_kids(b"no boxes here").is_empty());
        assert!(mp4_default_kids(b"tenc\0\0").is_empty(), "a truncated box is not a key ID");
        let mpd = r#"<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc" cenc:default_KID="a1b2c3d4-e5f6-0718-293a-4b5c6d7e8f90"/>"#;
        assert_eq!(mpd_default_kids(mpd), [kid]);
    }

    #[test]
    fn licence_exchange_uses_the_clearkey_json_format_and_only_accepts_requested_ids() {
        let kid = parse_key("a1b2c3d4e5f60718293a4b5c6d7e8f90").unwrap();
        let other = parse_key("ffffffffffffffffffffffffffffffff").unwrap();
        assert_eq!(licence_request(&[kid]), json!({"kids": ["obLD1OX2BxgpOktcbX6PkA"], "type": "temporary"}));
        let reply = json!({"keys": [{"kty": "oct", "kid": "obLD1OX2BxgpOktcbX6PkA", "k": "ABEiM0RVZneImaq7zN3u_w"}, {"kty": "oct", "kid": b64url(&other), "k": "ABEiM0RVZneImaq7zN3u_w"}], "type": "temporary"});
        let keys = licence_keys(&reply, &[kid]).unwrap();
        assert_eq!(keys.len(), 1, "a key for an ID that was not asked for is ignored");
        assert!(licence_keys(&json!({"keys": []}), &[kid]).is_err());
        assert!(licence_keys(&json!({"error": "denied"}), &[kid]).is_err());
        assert!(licence_keys(&json!({"keys": [{"kty": "oct", "kid": "x", "k": "y"}]}), &[kid]).is_err());
    }

    #[test]
    fn keys_must_cover_the_ids_the_video_declares() {
        let kid = parse_key("a1b2c3d4e5f60718293a4b5c6d7e8f90").unwrap();
        let key = [7u8; 16];
        assert!(check_keys(&[kid], &[ContentKey { kid: Some(kid), key }]).is_ok());
        assert!(check_keys(&[kid], &[ContentKey { kid: None, key }]).is_ok(), "a single unnamed key is used as the default key");
        let wrong = check_keys(&[kid], &[ContentKey { kid: Some([1; 16]), key }]).unwrap_err().to_string();
        assert!(wrong.contains("do not match") && !wrong.contains("07070707"), "the message names the needed ID, never a key: {wrong}");
        assert!(check_keys(&[kid], &[]).is_err());
    }

    #[test]
    fn player_options_carry_one_default_key_or_keys_by_id() {
        let mut stream = drm_stream(None);
        attach_keys(&mut stream, &[ContentKey { kid: None, key: [0x11; 16] }]);
        assert_eq!(lavf_options(&stream).unwrap(), ["decryption_key=11111111111111111111111111111111", "cenc_decryption_key=11111111111111111111111111111111"]);
        attach_keys(&mut stream, &[ContentKey { kid: Some([0xaa; 16]), key: [0x11; 16] }, ContentKey { kid: Some([0xbb; 16]), key: [0x22; 16] }]);
        let options = lavf_options(&stream).unwrap();
        assert!(options[0].starts_with("decryption_keys=aaaa") && options[0].contains(":bbbb"), "{options:?}");
        assert!(options[1].starts_with("cenc_decryption_keys=aaaa") && options[1].contains(",bbbb"));
        assert!(lavf_options(&drm_stream(None)).unwrap().is_empty());
        let mut forged = drm_stream(None);
        forged.drm.as_mut().unwrap().extra.insert(RESOLVED_KEYS_FIELD.into(), json!([{"key": "11,protocol_whitelist=file"}]));
        assert!(lavf_options(&forged).is_err(), "nothing but hex reaches the player's option list");
    }

    #[test]
    fn container_comes_from_the_link_type_then_the_address() {
        let mut stream = drm_stream(None);
        assert_eq!(container(&stream), Some(Container::Mp4));
        stream.url = "https://cdn.example/manifest.mpd?token=1".into();
        assert_eq!(container(&stream), Some(Container::Dash));
        stream.extra.insert("linkType".into(), json!("M3U8"));
        assert_eq!(container(&stream), Some(Container::Hls));
    }

    #[tokio::test]
    async fn resolution_refuses_hls_and_sources_without_keys() {
        let mut hls = drm_stream(None);
        hls.url = "https://cdn.example/a.m3u8".into();
        let fetch = |_: String, _: usize| async { Ok::<_, anyhow::Error>(Vec::new()) };
        let post = |_: String, _: Value, _: std::collections::BTreeMap<String, String>| async { Ok::<_, anyhow::Error>(json!({})) };
        assert!(resolve_clearkey(&hls, fetch, post).await.unwrap_err().to_string().contains("HLS"));
        let nothing = drm_stream(None);
        assert!(resolve_clearkey(&nothing, fetch, post).await.unwrap_err().to_string().contains("neither keys nor a licence"));
    }
}
