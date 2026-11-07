//! Optional, ID-matched episode skip markers. No title search or automatic seeking happens here.
//! AniSkip API contract: https://github.com/aniskip/aniskip-extension/tree/main/src/api/aniskip-http-client
//! AnimeSkip API contract: https://api.animeskip.com
//! TheIntroDB API contract: https://theintrodb.com
use crate::{metadata::{BoxFuture, MetadataLink}, models::Episode};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc, time::Duration};

const MAX_RESPONSE: usize = 256 * 1024;
const MAX_MARKERS: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SkipSettings {
    /// Sends the linked MAL ID, episode number and duration to AniSkip when enabled.
    pub aniskip: bool,
    /// Sends the linked ID, episode and duration to AnimeSkip when enabled.
    pub animeskip: bool,
    /// Sends the linked ID, season and episode to TheIntroDB when enabled.
    pub theintrodb: bool,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkipMarker {
    pub kind: String,
    pub start: f64,
    pub end: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub title: String,
    pub time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeekThumbnail {
    pub url: String,
    pub time: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A link supplied by the provider or explicitly selected by the person, for this exact media.
/// A title or a search suggestion alone is never sufficient.
pub fn matching_mal_id(media_id: &str, links: &[MetadataLink]) -> Option<u32> {
    let id = links.iter().find(|link| link.media_id == media_id && link.service == "mal")?.external_id.as_str();
    if id.is_empty() || id.len() > 10 || !id.bytes().all(|b| b.is_ascii_digit()) { return None; }
    id.parse::<u32>().ok().filter(|id| *id > 0)
}

/// General link extraction for a specific service.
pub fn matching_external_id(media_id: &str, links: &[MetadataLink], service: &str) -> Option<String> {
    let link = links.iter().find(|l| l.media_id == media_id && l.service == service)?;
    let id = link.external_id.trim();
    if id.is_empty() || id.len() > 64 || id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
        return None;
    }
    Some(id.to_string())
}

/// AniSkip indexes MAL episodes, not a provider's local per-season episode numbers. Refuse
/// multi-season numbering unless an explicit mapping is implemented, rather than guessing.
pub fn query_episode(episode: &Episode) -> Option<u32> {
    if episode.season.is_some_and(|season| season != 1) { return None; }
    episode.episode.and_then(|number| u32::try_from(number).ok()).filter(|n| (1..=100_000).contains(n))
}

pub trait Transport: Send + Sync {
    fn get(&self, url: String) -> BoxFuture<Value>;
}

pub struct Reqwest { client: reqwest::Client }
impl Reqwest {
    pub fn new() -> Result<Self> {
        Ok(Self { client: reqwest::Client::builder().timeout(Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none()).user_agent("CloudstreamDesktop").build()? })
    }
}
impl Transport for Reqwest {
    fn get(&self, address: String) -> BoxFuture<Value> {
        let client = self.client.clone();
        Box::pin(async move {
            let url = url::Url::parse(&address).context("Invalid skip service address")?;
            let allowed_hosts = ["api.aniskip.com", "api.animeskip.com", "api.theintrodb.com", "theintrodb.com"];
            if url.scheme() != "https" || !url.host_str().is_some_and(|h| allowed_hosts.contains(&h)) || url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
                bail!("Invalid skip service address");
            }
            let mut response = client.get(url).send().await.context("Could not reach the skip service")?;
            if response.status() == reqwest::StatusCode::NOT_FOUND { return Ok(serde_json::json!({"found":false})); }
            if !response.status().is_success() { bail!("The skip service answered with an error ({})", response.status().as_u16()); }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE { bail!("The skip service sent too much data"); }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).context("The skip service sent unreadable data")
        })
    }
}

pub struct Client { transport: Arc<dyn Transport> }
impl Client {
    pub fn new(transport: Arc<dyn Transport>) -> Self { Self { transport } }

    /// Returns no markers and makes no request when disabled, unlinked or ambiguous.
    pub async fn lookup(&self, settings: &SkipSettings, media_id: &str, links: &[MetadataLink], episode: &Episode, duration: f64) -> Result<Vec<SkipMarker>> {
        if !duration.is_finite() || !(10.0..=86_400.0).contains(&duration) { return Ok(Vec::new()); }
        let mut all_markers = Vec::new();

        // 1. AniSkip
        if settings.aniskip {
            if let (Some(mal), Some(number)) = (matching_mal_id(media_id, links), query_episode(episode)) {
                let mut url = url::Url::parse(&format!("https://api.aniskip.com/v2/skip-times/{mal}/{number}"))?;
                {
                    let mut query = url.query_pairs_mut();
                    for kind in ["op", "ed", "mixed-op", "mixed-ed", "recap"] { query.append_pair("types", kind); }
                    query.append_pair("episodeLength", &format!("{duration:.3}"));
                }
                if let Ok(response) = self.transport.get(url.into()).await {
                    if let Ok(markers) = parse_response(&response, duration) {
                        all_markers.extend(markers);
                    }
                }
            }
        }

        // 2. AnimeSkip
        if settings.animeskip {
            let id = matching_external_id(media_id, links, "mal")
                .or_else(|| matching_external_id(media_id, links, "anilist"))
                .or_else(|| matching_external_id(media_id, links, "kitsu"));
            if let (Some(ext_id), Some(number)) = (id, query_episode(episode)) {
                let mut url = url::Url::parse(&format!("https://api.animeskip.com/v1/skip-times/{ext_id}/{number}"))?;
                url.query_pairs_mut().append_pair("duration", &format!("{duration:.3}"));
                if let Ok(response) = self.transport.get(url.into()).await {
                    if let Ok(markers) = parse_animeskip_response(&response, duration) {
                        all_markers.extend(markers);
                    }
                }
            }
        }

        // 3. TheIntroDB
        if settings.theintrodb {
            let id = matching_external_id(media_id, links, "imdb")
                .or_else(|| matching_external_id(media_id, links, "tmdb"))
                .or_else(|| matching_external_id(media_id, links, "tvdb"));
            let season = episode.season.unwrap_or(1);
            let ep = episode.episode.unwrap_or(1);
            if let Some(ext_id) = id {
                let url = format!("https://api.theintrodb.com/v1/timestamps/{ext_id}/{season}/{ep}");
                if let Ok(response) = self.transport.get(url).await {
                    if let Ok(markers) = parse_theintrodb_response(&response, duration) {
                        all_markers.extend(markers);
                    }
                }
            }
        }

        all_markers.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.end.total_cmp(&b.end)).then_with(|| a.kind.cmp(&b.kind)));
        all_markers.dedup_by(|a, b| (a.start - b.start).abs() < 1.0 && a.kind == b.kind);
        Ok(all_markers)
    }
}

/// Validate the remote result against the actual file duration before exposing a skip action.
pub fn parse_response(value: &Value, duration: f64) -> Result<Vec<SkipMarker>> {
    if !duration.is_finite() || !(10.0..=86_400.0).contains(&duration) { return Ok(Vec::new()); }
    match value.get("found").and_then(Value::as_bool) {
        Some(false) => return Ok(Vec::new()),
        Some(true) => {},
        None => bail!("The skip service sent unreadable data"),
    }
    let entries = value.get("results").and_then(Value::as_array).context("The skip service sent unreadable data")?;
    if entries.len() > MAX_MARKERS { bail!("The skip service sent too many markers"); }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let Some(kind) = entry.get("skipType").and_then(Value::as_str).filter(|kind| matches!(*kind, "op" | "ed" | "mixed-op" | "mixed-ed" | "recap")) else { continue };
        let Some(start) = entry.pointer("/interval/startTime").and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        let Some(end) = entry.pointer("/interval/endTime").and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        let Some(length) = entry.get("episodeLength").and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0) else { continue };
        // Different releases and edits can have different openings. Reject rather than guess.
        if (length - duration).abs() > (duration * 0.02).max(5.0) || start < 0.0 || start >= duration || end <= start || end > duration + 1.0 { continue; }
        let end = end.min(duration);
        let key = format!("{kind}:{start:.3}:{end:.3}");
        if seen.insert(key) { result.push(SkipMarker { kind: kind.into(), start, end, source: "AniSkip".into() }); }
    }
    result.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.end.total_cmp(&b.end)).then_with(|| a.kind.cmp(&b.kind)));
    Ok(result)
}

/// Parse AnimeSkip API responses.
pub fn parse_animeskip_response(value: &Value, duration: f64) -> Result<Vec<SkipMarker>> {
    if !duration.is_finite() || !(10.0..=86_400.0).contains(&duration) { return Ok(Vec::new()); }
    let entries = if let Some(arr) = value.as_array() {
        arr
    } else if let Some(arr) = value.get("timestamps").and_then(Value::as_array) {
        arr
    } else if value.get("found").and_then(Value::as_bool) == Some(false) {
        return Ok(Vec::new());
    } else {
        bail!("Invalid AnimeSkip response format");
    };

    if entries.len() > MAX_MARKERS { bail!("Too many markers returned"); }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let raw_type = entry.get("type").or_else(|| entry.get("skipType")).and_then(Value::as_str).unwrap_or("");
        let kind = match raw_type {
            "op" | "intro" | "opening" => "op",
            "ed" | "outro" | "ending" => "ed",
            "recap" => "recap",
            "mixed-op" => "mixed-op",
            "mixed-ed" => "mixed-ed",
            _ => continue,
        };
        let Some(start) = entry.get("startTime").or_else(|| entry.get("start")).and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        let Some(end) = entry.get("endTime").or_else(|| entry.get("end")).and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        if start < 0.0 || start >= duration || end <= start || end > duration + 1.0 { continue; }
        let end = end.min(duration);
        let key = format!("{kind}:{start:.3}:{end:.3}");
        if seen.insert(key) {
            result.push(SkipMarker { kind: kind.into(), start, end, source: "AnimeSkip".into() });
        }
    }
    result.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.end.total_cmp(&b.end)));
    Ok(result)
}

/// Parse TheIntroDB API responses.
pub fn parse_theintrodb_response(value: &Value, duration: f64) -> Result<Vec<SkipMarker>> {
    if !duration.is_finite() || !(10.0..=86_400.0).contains(&duration) { return Ok(Vec::new()); }
    let entries = if let Some(arr) = value.as_array() {
        arr
    } else if let Some(arr) = value.get("timestamps").or_else(|| value.get("results")).and_then(Value::as_array) {
        arr
    } else if value.get("found").and_then(Value::as_bool) == Some(false) {
        return Ok(Vec::new());
    } else {
        bail!("Invalid TheIntroDB response format");
    };

    if entries.len() > MAX_MARKERS { bail!("Too many markers returned"); }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let raw_type = entry.get("type").or_else(|| entry.get("tag")).and_then(Value::as_str).unwrap_or("");
        let kind = match raw_type {
            "intro" | "opening" | "op" => "op",
            "outro" | "credits" | "ending" | "ed" => "ed",
            "recap" => "recap",
            _ => continue,
        };
        let Some(start) = entry.get("start").or_else(|| entry.get("startTime")).and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        let Some(end) = entry.get("end").or_else(|| entry.get("endTime")).and_then(Value::as_f64).filter(|n| n.is_finite()) else { continue };
        if start < 0.0 || start >= duration || end <= start || end > duration + 1.0 { continue; }
        let end = end.min(duration);
        let key = format!("{kind}:{start:.3}:{end:.3}");
        if seen.insert(key) {
            result.push(SkipMarker { kind: kind.into(), start, end, source: "TheIntroDB".into() });
        }
    }
    result.sort_by(|a, b| a.start.total_cmp(&b.start).then_with(|| a.end.total_cmp(&b.end)));
    Ok(result)
}

