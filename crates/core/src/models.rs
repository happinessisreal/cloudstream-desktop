use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn digest_string(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DubStatus {
    #[default]
    None,
    Dubbed,
    Subbed,
    Both,
}

impl DubStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Dubbed => "dubbed",
            Self::Subbed => "subbed",
            Self::Both => "both",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "dubbed" | "dub" => Self::Dubbed,
            "subbed" | "sub" => Self::Subbed,
            "both" | "all" => Self::Both,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaRef {
    pub media_id: String,
    pub provider_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_ids: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRef {
    pub episode_id: String,
    pub media_id: String,
    pub dub_status: DubStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i32>,
    pub data: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WatchProgress {
    pub profile_id: String,
    pub media_id: String,
    pub episode_id: String,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub is_watched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watched_override: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_reason: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub fn compute_media_id(provider_id: &str, url: &str, unique_url: Option<&str>) -> String {
    let key = unique_url.unwrap_or(url);
    format!("{provider_id}::{}", digest_string(key))
}

pub fn compute_episode_id(
    media_id: &str,
    episode: &Episode,
    dub: Option<DubStatus>,
    index: Option<usize>,
) -> String {
    if let Some(id) = &episode.id {
        if !id.is_empty() {
            return format!("{media_id}::ep::{id}");
        }
    }
    let dub_str = dub
        .or(episode.dub_status)
        .unwrap_or(DubStatus::None)
        .as_str();

    if let (Some(s), Some(e)) = (episode.season, episode.episode) {
        return format!("{media_id}::{dub_str}::s{s}e{e}");
    }
    if let Some(e) = episode.episode {
        return format!("{media_id}::{dub_str}::e{e}");
    }
    if let Some(s) = episode.season {
        let idx = index.unwrap_or(0);
        return format!("{media_id}::{dub_str}::s{s}_idx{idx}");
    }

    if episode.season.is_none()
        && episode.episode.is_none()
        && (index == Some(0) || index.is_none())
        && (episode.name.is_empty()
            || episode.name.contains("Movie")
            || episode.name.contains("Trailer")
            || episode.data.starts_with("http")
            || episode.data.contains("movie"))
    {
        return format!("{media_id}::movie");
    }

    let idx = index.unwrap_or(0);
    let clean_name: String = episode
        .name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect();
    if clean_name.is_empty() {
        format!("{media_id}::{dub_str}::idx{idx}")
    } else {
        format!("{media_id}::{dub_str}::idx{idx}_{clean_name}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryManifest {
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub manifest_version: u32,
    pub plugin_lists: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    pub url: String,
    pub name: String,
    pub internal_name: String,
    pub version: i64,
    #[serde(default = "default_status")]
    pub status: i32,
    #[serde(default = "default_api")]
    pub api_version: u32,
    #[serde(default)]
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub icon_url: Option<String>,
    pub repository_url: Option<String>,
    pub tv_types: Option<Vec<String>>,
    pub file_size: Option<u64>,
    pub file_hash: Option<String>,
    pub jar_url: Option<String>,
    pub jar_hash: Option<String>,
    pub jar_file_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
fn default_status() -> i32 {
    1
}
fn default_api() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub url: String,
    pub manifest: RepositoryManifest,
    pub plugins: Vec<PluginEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Jvm,
    Android,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub id: String,
    pub repository_url: String,
    pub entry: PluginEntry,
    pub runtime: RuntimeKind,
    pub path: String,
    pub sha256: String,
    pub verified: bool,
    pub plugin_class: Option<String>,
    pub installed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUpdate {
    pub plugin_id: String,
    pub name: String,
    pub repository_url: String,
    pub installed_version: i64,
    pub available_version: i64,
    pub changelog: Option<String>,
    pub has_update: bool,
    pub entry: PluginEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkUpdateResult {
    pub plugin_id: String,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub previous_version: i64,
    pub updated_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestStageResult {
    pub stage: String,
    pub status: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestReport {
    pub provider_id: String,
    pub provider_name: String,
    pub passed: bool,
    pub total_duration_ms: u64,
    pub stages: Vec<ProviderTestStageResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionState {
    pub plugin_id: String,
    pub loaded: bool,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extractors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_unavailable_apis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actionable_failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityReport {
    pub sha256: String,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_class: Option<String>,
    pub legacy: bool,
    pub requires_resources: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_apis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actionable_failure: Option<String>,
    pub plugin_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub has_main_page: bool,
    #[serde(default)]
    pub has_quick_search: bool,
    #[serde(default)]
    pub has_download_support: bool,
    #[serde(default)]
    pub has_chromecast_support: bool,
    #[serde(default)]
    pub uses_web_view: bool,
    #[serde(default)]
    pub instant_link_loading: bool,
    #[serde(default)]
    pub sequential_main_page: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequential_main_page_delay_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequential_main_page_scroll_delay_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vpn_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_sync_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_types: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTimeouts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_search_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub get_main_page_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_links_timeout_ms: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCloneConfig {
    pub id: String,
    pub parent_provider_id: String,
    pub name: String,
    pub custom_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_handle: Option<String>,
    #[serde(default = "default_clone_enabled")]
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_clone_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub lang: String,
    pub main_url: String,
    pub has_main_page: bool,
    pub uses_web_view: bool,
    #[serde(default)]
    pub supported_types: Vec<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ProviderCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeouts: Option<ProviderTimeouts>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub provider_id: String,
    pub name: String,
    pub url: String,
    pub poster_url: Option<String>,
    #[serde(default)]
    pub poster_headers: BTreeMap<String, String>,
    pub media_type: Option<String>,
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episodes_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dub_status: Option<DubStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dub_episodes: Option<BTreeMap<String, i32>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub name: String,
    pub data: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dub_status: Option<DubStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_time: Option<i32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrailerData {
    pub extractor_url: String,
    #[serde(default)]
    pub referer: String,
    #[serde(default)]
    pub raw: bool,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActorData {
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_actor: Option<Actor>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeasonData {
    pub season: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetails {
    pub item: MediaItem,
    pub plot: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    pub duration: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coming_soon: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sync_data: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trailers: Vec<TrailerData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ActorData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<MediaItem>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dub_groups: BTreeMap<String, Vec<Episode>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seasons: Vec<SeasonData>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_handle: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_us: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DrmData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub key_request_parameters: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub referer: String,
    #[serde(default)]
    pub quality: i32,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub stream_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<AudioTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist: Option<Vec<PlaylistItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drm: Option<DrmData>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Subtitle {
    pub lang: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_handle: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaTransportInfo {
    pub enabled: bool,
    pub port: u16,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaSessionInfo {
    pub session_token: String,
    pub transport_url: String,
    #[serde(default)]
    pub playlist_urls: Vec<String>,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebViewReport {
    pub supported: bool,
    pub reason: String,
    pub message: String,
    #[serde(default)]
    pub missing_features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedLinks {
    pub streams: Vec<Stream>,
    pub subtitles: Vec<Subtitle>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HomeRow {
    pub name: String,
    #[serde(default)]
    pub items: Vec<MediaItem>,
    #[serde(default)]
    pub has_next: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(default)]
    pub horizontal_images: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HomeResponse {
    pub rows: Vec<HomeRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_section: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more_sections: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatchStatus {
    Watching,
    Completed,
    OnHold,
    Dropped,
    PlanToWatch,
    #[default]
    None,
}

impl WatchStatus {
    pub fn to_android_int(&self) -> i32 {
        match self {
            WatchStatus::Watching => 0,
            WatchStatus::Completed => 1,
            WatchStatus::OnHold => 2,
            WatchStatus::Dropped => 3,
            WatchStatus::PlanToWatch => 4,
            WatchStatus::None => 5,
        }
    }

    pub fn from_android_int(val: i32) -> Self {
        match val {
            0 => WatchStatus::Watching,
            1 => WatchStatus::Completed,
            2 => WatchStatus::OnHold,
            3 => WatchStatus::Dropped,
            4 => WatchStatus::PlanToWatch,
            _ => WatchStatus::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: String,
    pub item: MediaItem,
    pub saved_at: u64,
    #[serde(default)]
    pub watch_status: WatchStatus,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub is_subscribed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchEntry {
    pub id: String,
    pub item: MediaItem,
    pub episode: Episode,
    pub position: f64,
    pub duration: f64,
    pub watched_at: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PosterDensity {
    #[default]
    Comfortable,
    Compact,
}

impl PosterDensity {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "comfortable" => Some(Self::Comfortable),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// The approved desktop design.
    #[default]
    Dark,
    /// Pure black surfaces with stronger borders and text, for OLED screens and low vision.
    Black,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Indigo,
    Teal,
    Rose,
    Amber,
    Green,
    Violet,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TitlePlacement {
    #[default]
    Below,
    Overlay,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreference {
    /// Follow the system's reduced-motion setting.
    #[default]
    System,
    Reduced,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NavigationMode {
    #[default]
    Pointer,
    /// Arrow keys move between cards and controls (keyboard or remote-style use).
    Spatial,
}

pub const POSTER_SCALE_MIN: u32 = 80;
pub const POSTER_SCALE_MAX: u32 = 140;

fn default_poster_scale() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default)]
    pub poster_density: PosterDensity,
    #[serde(default = "default_true")]
    pub search_suggestions: bool,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub accent: Accent,
    /// Poster width as a percentage of the density's size (80 to 140).
    #[serde(default = "default_poster_scale")]
    pub poster_scale: u32,
    #[serde(default)]
    pub title_placement: TitlePlacement,
    /// Show the media type badge on posters.
    #[serde(default = "default_true")]
    pub show_badges: bool,
    #[serde(default)]
    pub motion: MotionPreference,
    #[serde(default)]
    pub navigation: NavigationMode,
    /// `system` or a language tag such as `de` or `pt-BR`.
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_locale() -> String {
    "system".into()
}

/// `system`, or a plain language tag (`ar`, `pt-BR`, `zh-TW`). Which locales exist is the
/// interface's business; an unknown one falls back to English there.
pub fn valid_locale_tag(tag: &str) -> bool {
    if tag == "system" {
        return true;
    }
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or("");
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_lowercase())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
        && tag.len() <= 20
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            poster_density: PosterDensity::Comfortable,
            search_suggestions: true,
            theme: Theme::Dark,
            accent: Accent::Indigo,
            poster_scale: default_poster_scale(),
            title_placement: TitlePlacement::Below,
            show_badges: true,
            motion: MotionPreference::System,
            navigation: NavigationMode::Pointer,
            locale: default_locale(),
            extra: BTreeMap::new(),
        }
    }
}

impl AppearanceSettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(POSTER_SCALE_MIN..=POSTER_SCALE_MAX).contains(&self.poster_scale) {
            anyhow::bail!("Poster size must be between {POSTER_SCALE_MIN}% and {POSTER_SCALE_MAX}%");
        }
        if !valid_locale_tag(&self.locale) {
            anyhow::bail!("'{}' is not a valid language tag. Use system, or a tag such as de or pt-BR", self.locale.chars().take(24).collect::<String>());
        }
        Ok(())
    }
}

pub fn default_completion_threshold() -> f64 {
    0.90
}

pub fn default_resume_threshold_seconds() -> f64 {
    5.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPolicy {
    #[serde(default = "default_true")]
    pub auto_next: bool,
    #[serde(default)]
    pub start_paused: bool,
    #[serde(default = "default_completion_threshold")]
    pub completion_threshold: f64,
    #[serde(default = "default_resume_threshold_seconds")]
    pub resume_threshold_seconds: f64,
    #[serde(default = "default_true")]
    pub prefetch_next: bool,
    /// How many alternate mirrors may be tried automatically after a stream fails to load or
    /// drops. `0` disables automatic failover; manual switching is always available.
    #[serde(default = "default_failover_attempts")]
    pub failover_attempts: u32,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub fn default_failover_attempts() -> u32 {
    2
}

pub const MAX_FAILOVER_ATTEMPTS: u32 = 5;

impl Default for PlaybackPolicy {
    fn default() -> Self {
        Self {
            auto_next: true,
            start_paused: false,
            completion_threshold: 0.90,
            resume_threshold_seconds: 5.0,
            prefetch_next: true,
            failover_attempts: default_failover_attempts(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackQueue {
    pub profile_id: String,
    pub media: MediaItem,
    pub episodes: Vec<Episode>,
    pub current_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dub_status: Option<DubStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl PlaybackQueue {
    pub fn current_episode(&self) -> Option<&Episode> {
        self.episodes.get(self.current_index)
    }

    pub fn has_next(&self) -> bool {
        self.current_index + 1 < self.episodes.len()
    }

    pub fn has_prev(&self) -> bool {
        self.current_index > 0 && !self.episodes.is_empty()
    }

    pub fn next_episode(&self) -> Option<&Episode> {
        if self.has_next() {
            self.episodes.get(self.current_index + 1)
        } else {
            None
        }
    }

    pub fn prev_episode(&self) -> Option<&Episode> {
        if self.has_prev() {
            self.episodes.get(self.current_index - 1)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSettings {
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile_id: Option<String>,
    #[serde(default)]
    pub auto_select_profile: bool,
    /// Send desktop alerts for newly observed episodes only when explicitly enabled.
    #[serde(default)]
    pub subscription_notifications: bool,
    #[serde(default)]
    pub playback: PlaybackPolicy,
    #[serde(default)]
    pub player: PlayerPreferences,
    #[serde(default)]
    pub backup: BackupSettings,
    #[serde(default)]
    pub external_player: crate::external_player::ExternalPlayerSettings,
    #[serde(default)]
    pub updates: crate::updates::UpdateSettings,
    /// First-run setup. Completing, skipping or redoing it never removes data.
    #[serde(default)]
    pub setup: SetupState,
    /// Languages the extension catalog should show first (language tags such as `en`, `ja`).
    #[serde(default)]
    pub provider_languages: Vec<String>,
    /// Optional lookups of posters, cast, recommendations and filler flags from public services.
    #[serde(default)]
    pub metadata: crate::metadata::MetadataSettings,
    /// Opt-in external episode skip timing services.
    #[serde(default)]
    pub skip: crate::skip::SkipSettings,
    /// Opt-in external online subtitle search services.
    #[serde(default)]
    pub subtitle_services: crate::subtitle_providers::ServiceSettings,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceSettings::default(),
            default_profile_id: None,
            auto_select_profile: true,
            subscription_notifications: false,
            playback: PlaybackPolicy::default(),
            player: PlayerPreferences::default(),
            backup: BackupSettings::default(),
            external_player: Default::default(),
            updates: Default::default(),
            setup: SetupState::default(),
            provider_languages: Vec::new(),
            metadata: Default::default(),
            skip: Default::default(),
            subtitle_services: Default::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub avatar: String,
    #[serde(default)]
    pub has_pin: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileInput {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_pin: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettings {
    #[serde(default)]
    pub appearance: Option<AppearanceSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackPolicy>,
    /// Quality/source priority profiles. Scoped to the local profile like Android's
    /// per-account `video_*_priority` keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_priority: Option<SourcePriorityConfig>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub fn default_settings_schema_version() -> u32 {
    1
}

pub fn default_settings_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_settings_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_settings_revision")]
    pub revision: u64,
    #[serde(default)]
    pub global: GlobalSettings,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileSettings>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: default_settings_schema_version(),
            revision: default_settings_revision(),
            global: GlobalSettings::default(),
            profiles: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Range checks for values that reach mpv or drive playback. Called after every patch so a
    /// bad value is rejected with a message instead of being saved.
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        let playback = &self.global.playback;
        if playback.failover_attempts > MAX_FAILOVER_ATTEMPTS {
            bail!("Automatic failover can try at most {MAX_FAILOVER_ATTEMPTS} other sources");
        }
        self.global.appearance.validate()?;
        for (profile_id, profile) in &self.profiles {
            if let Some(appearance) = &profile.appearance {
                appearance.validate().map_err(|e| anyhow::anyhow!("Appearance for profile '{profile_id}': {e}"))?;
            }
        }
        self.global.player.validate()?;
        self.global.backup.validate()?;
        self.global.external_player.validate()?;
        self.global.updates.validate()?;
        self.global.subtitle_services.validate()?;
        if self.global.provider_languages.len() > 40 || self.global.provider_languages.iter().any(|l| l == "system" || !valid_locale_tag(l)) {
            bail!("Source languages must be language tags such as en or ja (at most 40)");
        }
        for (profile_id, profile) in &self.profiles {
            if let Some(config) = &profile.source_priority {
                config
                    .validate()
                    .map_err(|e| anyhow::anyhow!("Source priorities for profile '{profile_id}': {e}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHistoryEntry {
    pub id: String,
    pub profile_id: String,
    pub query: String,
    pub searched_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub items: Vec<MediaItem>,
    pub has_next: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub jvm_available: bool,
    pub worker_available: bool,
    pub mpv_available: bool,
    pub android_available: bool,
    pub java_path: String,
    pub worker_path: String,
    pub notes: Vec<String>,
}

pub fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn to_epoch_millis(timestamp_or_seconds: u64) -> u64 {
    if timestamp_or_seconds > 0 && timestamp_or_seconds < 10_000_000_000 {
        timestamp_or_seconds * 1000
    } else {
        timestamp_or_seconds
    }
}

/// One audio or subtitle track reported by mpv for the active file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTrack {
    pub id: i64,
    pub kind: String,
    pub selected: bool,
    pub external: bool,
    pub forced: bool,
    pub hearing_impaired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub session_id: Option<String>,
    pub active: bool,
    pub playing: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub speed: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    #[serde(default)]
    pub has_next: bool,
    #[serde(default)]
    pub has_prev: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_count: Option<usize>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub fullscreen: bool,
    /// Audio delay in seconds as reported by mpv (positive delays audio).
    #[serde(default)]
    pub audio_delay: f64,
    #[serde(default)]
    pub subtitle_delay: f64,
    /// Clockwise video rotation in degrees as reported by mpv.
    #[serde(default)]
    pub rotation: i32,
    #[serde(default)]
    pub aspect: AspectMode,
    /// The hardware decoder mpv is actually using (`None` for software decoding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hwdec_current: Option<String>,
    /// The stream that is currently open, so the UI can mark the active mirror.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<ActiveStream>,
    #[serde(default)]
    pub tracks: Vec<PlayerTrack>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            session_id: None,
            active: false,
            playing: false,
            paused: false,
            position: 0.0,
            duration: 0.0,
            volume: 100.0,
            speed: 1.0,
            item_name: None,
            episode_name: None,
            media_id: None,
            episode_id: None,
            profile_id: None,
            end_reason: None,
            has_next: false,
            has_prev: false,
            queue_index: None,
            queue_count: None,
            muted: false,
            fullscreen: false,
            audio_delay: 0.0,
            subtitle_delay: 0.0,
            rotation: 0,
            aspect: AspectMode::Fit,
            hwdec_current: None,
            stream: None,
            tracks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerEvent {
    StateChanged {
        state: PlayerState,
    },
    FileLoaded {
        session_id: String,
    },
    EndFile {
        session_id: String,
        reason: String,
        /// mpv's own explanation when the file failed to play (`reason == "error"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Shutdown {
        session_id: String,
    },
    /// Fatal for the current playback: the UI should drop the player bar.
    Error {
        session_id: String,
        message: String,
    },
    /// Non-fatal information (an unsupported preference was skipped, failover happened).
    Notice {
        session_id: String,
        message: String,
    },
    /// The open stream changed without the user leaving the episode.
    SourceChanged {
        session_id: String,
        name: String,
        quality: i32,
        quality_label: String,
        /// `manual` or `failover`.
        reason: String,
        attempt: u32,
    },
    QueueUpdated {
        queue: Option<PlaybackQueue>,
    },
}

// ---------------------------------------------------------------------------------------------
// T16: player controls, quality policy and decoding preferences
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupFrequency {
    #[default]
    Daily,
    Weekly,
}

impl BackupFrequency {
    pub fn interval_ms(self) -> u64 {
        match self {
            Self::Daily => 24 * 3_600_000,
            Self::Weekly => 7 * 24 * 3_600_000,
        }
    }
}

pub const MAX_BACKUP_RETENTION: u32 = 100;

fn default_backup_retention() -> u32 {
    7
}

/// Scheduled backup preferences. Backups only run while the app is open; a backup that came due
/// while it was closed runs when the app starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Absolute folder path. Only files the app named itself are ever pruned from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default)]
    pub frequency: BackupFrequency,
    /// How many scheduled backups to keep (1 to 100).
    #[serde(default = "default_backup_retention")]
    pub retention: u32,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self { enabled: false, directory: None, frequency: BackupFrequency::Daily, retention: default_backup_retention(), extra: BTreeMap::new() }
    }
}

impl BackupSettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if self.retention == 0 || self.retention > MAX_BACKUP_RETENTION {
            bail!("Keep between 1 and {MAX_BACKUP_RETENTION} scheduled backups");
        }
        if let Some(directory) = &self.directory {
            let path = std::path::Path::new(directory);
            if directory.trim().is_empty() || !path.is_absolute() || directory.contains('\0') {
                bail!("Choose a full folder path for scheduled backups");
            }
        }
        if self.enabled && self.directory.is_none() {
            bail!("Choose a folder before turning scheduled backups on");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    #[serde(default)]
    pub completed: bool,
    /// `true` when the user chose Skip instead of finishing.
    #[serde(default)]
    pub skipped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    /// Version of the setup flow that was completed, so a future flow can offer new steps.
    #[serde(default)]
    pub version: u32,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub const VOLUME_MAX: f64 = 130.0;
pub const SPEED_MIN: f64 = 0.25;
pub const SPEED_MAX: f64 = 4.0;
pub const AUDIO_DELAY_LIMIT_SECONDS: f64 = 30.0;
pub const SEEK_RELATIVE_LIMIT_SECONDS: f64 = 86_400.0;
pub const BUFFER_RAM_LIMIT_MB: u32 = 4096;
pub const BUFFER_SECONDS_LIMIT: u32 = 3600;

/// How the video is fitted into the mpv window.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AspectMode {
    /// Keep the video's own aspect ratio, letterboxing as needed.
    #[default]
    #[serde(rename = "fit")]
    Fit,
    /// Keep the aspect ratio but crop to fill the window.
    #[serde(rename = "fill")]
    Fill,
    /// Distort the picture to fill the window.
    #[serde(rename = "stretch")]
    Stretch,
    #[serde(rename = "16:9")]
    Ratio16x9,
    #[serde(rename = "4:3")]
    Ratio4x3,
    #[serde(rename = "21:9")]
    Ratio21x9,
    /// mpv is in a combination this app does not offer (for example set from mpv's own keys).
    /// Reported in state, but never accepted as a command.
    #[serde(rename = "custom")]
    Custom,
}

impl AspectMode {
    /// Derive the mode from the three mpv properties that define it, so that changes made inside
    /// the mpv window are reflected in the app instead of being overwritten by a cached value.
    pub fn from_mpv(keepaspect: bool, panscan: f64, aspect_override: f64) -> Self {
        let override_active = aspect_override > 0.0;
        if !keepaspect {
            return Self::Stretch;
        }
        if panscan >= 0.999 && !override_active {
            return Self::Fill;
        }
        if panscan.abs() > 0.001 {
            return Self::Custom;
        }
        if !override_active {
            return Self::Fit;
        }
        let near = |ratio: f64| (aspect_override - ratio).abs() < 0.01;
        if near(16.0 / 9.0) {
            Self::Ratio16x9
        } else if near(4.0 / 3.0) {
            Self::Ratio4x3
        } else if near(21.0 / 9.0) {
            Self::Ratio21x9
        } else {
            Self::Custom
        }
    }

    /// `(keepaspect, panscan, video-aspect-override)` that realises this mode in mpv.
    pub fn mpv_values(self) -> Option<(bool, f64, Value)> {
        let none = Value::from(-1);
        Some(match self {
            Self::Fit => (true, 0.0, none),
            Self::Fill => (true, 1.0, none),
            Self::Stretch => (false, 0.0, none),
            Self::Ratio16x9 => (true, 0.0, Value::from("16:9")),
            Self::Ratio4x3 => (true, 0.0, Value::from("4:3")),
            Self::Ratio21x9 => (true, 0.0, Value::from("21:9")),
            Self::Custom => return None,
        })
    }
}

/// One acknowledged, validated action on the running mpv session. Pause/relative seek/stop
/// keep working through the older string `player_action` command as well.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum PlayerCommand {
    Play,
    Pause,
    TogglePause,
    SeekRelative { seconds: f64 },
    SeekAbsolute { seconds: f64 },
    SetVolume { volume: f64 },
    SetMute { muted: bool },
    SetSpeed { speed: f64 },
    SetFullscreen { fullscreen: bool },
    SetAspect { mode: AspectMode },
    SetAudioDelay { seconds: f64 },
    SetSubtitleDelay { seconds: f64 },
    SetAudioTrack { id: Option<i64> },
    SetSubtitleTrack { id: Option<i64> },
    SetRotation { degrees: i32 },
}

impl PlayerCommand {
    /// Reject out-of-range values before anything is sent to mpv. `duration` is the currently
    /// known duration (`0` when unknown or live).
    pub fn validate(&self, duration: f64) -> anyhow::Result<()> {
        use anyhow::bail;
        let finite = |name: &str, v: f64| -> anyhow::Result<()> {
            if !v.is_finite() {
                bail!("{name} must be a finite number");
            }
            Ok(())
        };
        match self {
            Self::Play | Self::Pause | Self::TogglePause => {}
            Self::SeekRelative { seconds } => {
                finite("Seek offset", *seconds)?;
                if seconds.abs() > SEEK_RELATIVE_LIMIT_SECONDS {
                    bail!("Seek offset must be within ±{SEEK_RELATIVE_LIMIT_SECONDS:.0} seconds");
                }
            }
            Self::SeekAbsolute { seconds } => {
                finite("Seek position", *seconds)?;
                if *seconds < 0.0 {
                    bail!("Seek position cannot be negative");
                }
                if duration > 0.0 && *seconds > duration {
                    bail!(
                        "Cannot seek to {seconds:.1}s: this video is only {duration:.1}s long"
                    );
                }
            }
            Self::SetVolume { volume } => {
                finite("Volume", *volume)?;
                if !(0.0..=VOLUME_MAX).contains(volume) {
                    bail!("Volume must be between 0 and {VOLUME_MAX:.0} (got {volume})");
                }
            }
            Self::SetMute { .. } | Self::SetFullscreen { .. } => {}
            Self::SetSpeed { speed } => {
                finite("Speed", *speed)?;
                if !(SPEED_MIN..=SPEED_MAX).contains(speed) {
                    bail!("Speed must be between {SPEED_MIN}x and {SPEED_MAX}x (got {speed}x)");
                }
            }
            Self::SetAspect { mode } => {
                if *mode == AspectMode::Custom {
                    bail!("Choose Fit, Fill, Stretch, 16:9, 4:3 or 21:9");
                }
            }
            Self::SetAudioDelay { seconds } => {
                finite("Audio delay", *seconds)?;
                if seconds.abs() > AUDIO_DELAY_LIMIT_SECONDS {
                    bail!(
                        "Audio delay must be within ±{AUDIO_DELAY_LIMIT_SECONDS:.0} seconds (got {seconds})"
                    );
                }
            }
            Self::SetSubtitleDelay { seconds } => {
                finite("Subtitle delay", *seconds)?;
                if seconds.abs() > AUDIO_DELAY_LIMIT_SECONDS { bail!("Subtitle delay must be within ±{AUDIO_DELAY_LIMIT_SECONDS:.0} seconds"); }
            }
            Self::SetAudioTrack { id } | Self::SetSubtitleTrack { id } => {
                if id.is_some_and(|id| id <= 0) { bail!("Track ID must be positive"); }
            }
            Self::SetRotation { degrees } => {
                if !matches!(degrees, 0 | 90 | 180 | 270) {
                    bail!("Rotation must be 0, 90, 180 or 270 degrees (got {degrees})");
                }
            }
        }
        Ok(())
    }

    /// The ordered mpv JSON IPC commands that realise this action. Call `validate` first.
    pub fn to_mpv(&self) -> Vec<Vec<Value>> {
        let set = |name: &str, value: Value| vec![Value::from("set_property"), Value::from(name), value];
        match self {
            Self::Play => vec![set("pause", Value::from(false))],
            Self::Pause => vec![set("pause", Value::from(true))],
            Self::TogglePause => vec![vec![Value::from("cycle"), Value::from("pause")]],
            Self::SeekRelative { seconds } => {
                vec![vec![Value::from("seek"), Value::from(*seconds), Value::from("relative")]]
            }
            Self::SeekAbsolute { seconds } => {
                vec![vec![Value::from("seek"), Value::from(*seconds), Value::from("absolute")]]
            }
            Self::SetVolume { volume } => vec![set("volume", Value::from(*volume))],
            Self::SetMute { muted } => vec![set("mute", Value::from(*muted))],
            Self::SetSpeed { speed } => vec![set("speed", Value::from(*speed))],
            Self::SetFullscreen { fullscreen } => vec![set("fullscreen", Value::from(*fullscreen))],
            Self::SetAspect { mode } => match mode.mpv_values() {
                Some((keep, pan, aspect)) => vec![
                    set("keepaspect", Value::from(keep)),
                    set("panscan", Value::from(pan)),
                    set("video-aspect-override", aspect),
                ],
                None => Vec::new(),
            },
            Self::SetAudioDelay { seconds } => vec![set("audio-delay", Value::from(*seconds))],
            Self::SetSubtitleDelay { seconds } => vec![set("sub-delay", Value::from(*seconds))],
            Self::SetAudioTrack { id } => vec![set("aid", id.map(Value::from).unwrap_or_else(|| Value::from("no")))],
            Self::SetSubtitleTrack { id } => vec![set("sid", id.map(Value::from).unwrap_or_else(|| Value::from("no")))],
            Self::SetRotation { degrees } => vec![set("video-rotate", Value::from(*degrees))],
        }
    }

    /// Short human description used in error messages.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Play => "resume playback",
            Self::Pause => "pause playback",
            Self::TogglePause => "toggle pause",
            Self::SeekRelative { .. } | Self::SeekAbsolute { .. } => "seek",
            Self::SetVolume { .. } => "change the volume",
            Self::SetMute { .. } => "change mute",
            Self::SetSpeed { .. } => "change the speed",
            Self::SetFullscreen { .. } => "change fullscreen",
            Self::SetAspect { .. } => "change the aspect ratio",
            Self::SetAudioDelay { .. } => "change the audio delay",
            Self::SetSubtitleDelay { .. } => "change the subtitle delay",
            Self::SetAudioTrack { .. } => "select the audio track",
            Self::SetSubtitleTrack { .. } => "select the subtitle track",
            Self::SetRotation { .. } => "rotate the video",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecodingMode {
    /// mpv's safe hardware decoders, falling back to software (`--hwdec=auto-safe`).
    #[default]
    Auto,
    /// Try every hardware decoder mpv knows (`--hwdec=auto`).
    Hardware,
    /// Never use hardware decoding (`--hwdec=no`).
    Software,
}

impl DecodingMode {
    pub fn mpv_value(self) -> &'static str {
        match self {
            Self::Auto => "auto-safe",
            Self::Hardware => "auto",
            Self::Software => "no",
        }
    }
}

fn default_player_volume() -> f64 {
    100.0
}

fn default_player_speed() -> f64 {
    1.0
}

/// Appearance for text subtitles. ASS styling is retained unless explicitly overridden.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SubtitleStyle {
    pub font: String,
    pub font_id: Option<String>,
    pub size: f64,
    pub color: String,
    pub bold: bool,
    pub italic: bool,
    pub outline_color: String,
    pub outline_size: f64,
    pub shadow_offset: f64,
    pub background_color: String,
    pub background_opacity: u8,
    pub background_box: bool,
    pub align_x: String,
    pub align_y: String,
    pub vertical_margin: u32,
    pub override_ass: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl Default for SubtitleStyle {
    fn default() -> Self {
        Self { font: "sans-serif".into(), font_id: None, size: 38.0, color: "#ffffff".into(), bold: false, italic: false,
            outline_color: "#000000".into(), outline_size: 1.65, shadow_offset: 0.0,
            background_color: "#000000".into(), background_opacity: 69, background_box: false,
            align_x: "center".into(), align_y: "bottom".into(), vertical_margin: 34,
            override_ass: false, extra: BTreeMap::new() }
    }
}
impl SubtitleStyle {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if self.font.trim().is_empty() || self.font.chars().count() > 120 || self.font.chars().any(|c| c.is_control() || matches!(c, ';' | '{' | '}' | '\\' | '"')) {
            bail!("Subtitle font name must be 1 to 120 plain characters");
        }
        if self.font_id.as_deref().is_some_and(|id| !crate::fonts::valid_font_id(id)) { bail!("Invalid imported font identifier"); }
        if !self.size.is_finite() || !(10.0..=120.0).contains(&self.size) { bail!("Subtitle font size must be between 10 and 120"); }
        for color in [&self.color, &self.outline_color, &self.background_color] {
            if color.len() != 7 || !color.starts_with('#') || !color[1..].bytes().all(|b| b.is_ascii_hexdigit()) { bail!("Subtitle colors must use #RRGGBB"); }
        }
        if !self.outline_size.is_finite() || !(0.0..=10.0).contains(&self.outline_size) { bail!("Subtitle outline must be between 0 and 10"); }
        if !self.shadow_offset.is_finite() || !(0.0..=10.0).contains(&self.shadow_offset) { bail!("Subtitle shadow must be between 0 and 10"); }
        if self.background_opacity > 100 { bail!("Subtitle background opacity must be between 0 and 100"); }
        if !matches!(self.align_x.as_str(), "left" | "center" | "right") || !matches!(self.align_y.as_str(), "top" | "center" | "bottom") { bail!("Invalid subtitle alignment"); }
        if self.vertical_margin > 200 { bail!("Subtitle vertical margin must be between 0 and 200"); }
        Ok(())
    }
}

/// Persisted player settings that are applied to every mpv session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPreferences {
    #[serde(default = "default_player_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "default_player_speed")]
    pub speed: f64,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub aspect: AspectMode,
    #[serde(default)]
    pub audio_delay: f64,
    #[serde(default)]
    pub subtitle_delay: f64,
    #[serde(default)]
    pub subtitle_style: SubtitleStyle,
    #[serde(default)]
    pub subtitle_transforms: crate::subtitles::TextTransforms,
    #[serde(default)]
    pub subtitles_enabled: Option<bool>,
    #[serde(default)]
    pub subtitle_language: Option<String>,
    #[serde(default)]
    pub audio_enabled: Option<bool>,
    #[serde(default)]
    pub audio_language: Option<String>,
    #[serde(default)]
    pub decoding: DecodingMode,
    /// Demuxer cache size in MiB. `0` keeps mpv's default.
    #[serde(default)]
    pub buffer_ram_mb: u32,
    /// Seconds of media to read ahead. `0` keeps mpv's default.
    #[serde(default)]
    pub buffer_seconds: u32,
    /// Dynamic range compression to even out loud and quiet scenes.
    #[serde(default)]
    pub audio_compressor: bool,
    #[serde(default = "default_seek_step")]
    pub seek_step_seconds: u32,
    #[serde(default = "default_hold_speed")]
    pub hold_speed: f64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_seek_step() -> u32 { 10 }
fn default_hold_speed() -> f64 { 2.0 }

impl Default for PlayerPreferences {
    fn default() -> Self {
        Self {
            volume: default_player_volume(),
            muted: false,
            speed: default_player_speed(),
            fullscreen: false,
            aspect: AspectMode::Fit,
            audio_delay: 0.0,
            subtitle_delay: 0.0,
            subtitle_style: SubtitleStyle::default(),
            subtitle_transforms: crate::subtitles::TextTransforms::default(),
            subtitles_enabled: None,
            subtitle_language: None,
            audio_enabled: None,
            audio_language: None,
            decoding: DecodingMode::Auto,
            buffer_ram_mb: 0,
            buffer_seconds: 0,
            audio_compressor: false,
            seek_step_seconds: 10,
            hold_speed: 2.0,
            extra: BTreeMap::new(),
        }
    }
}

impl PlayerPreferences {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if !self.volume.is_finite() || !(0.0..=VOLUME_MAX).contains(&self.volume) {
            bail!("Default volume must be between 0 and {VOLUME_MAX:.0}");
        }
        if !self.speed.is_finite() || !(SPEED_MIN..=SPEED_MAX).contains(&self.speed) {
            bail!("Default speed must be between {SPEED_MIN}x and {SPEED_MAX}x");
        }
        if !self.audio_delay.is_finite() || self.audio_delay.abs() > AUDIO_DELAY_LIMIT_SECONDS {
            bail!("Audio delay must be within ±{AUDIO_DELAY_LIMIT_SECONDS:.0} seconds");
        }
        if !self.subtitle_delay.is_finite() || self.subtitle_delay.abs() > AUDIO_DELAY_LIMIT_SECONDS {
            bail!("Subtitle delay must be within ±{AUDIO_DELAY_LIMIT_SECONDS:.0} seconds");
        }
        self.subtitle_style.validate()?;
        for language in [&self.subtitle_language, &self.audio_language].into_iter().flatten() {
            if language.is_empty() || language.len() > 12 || !language.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                bail!("Language must be a short language tag, such as en or pt-BR");
            }
        }
        if self.aspect == AspectMode::Custom {
            bail!("Default aspect must be Fit, Fill, Stretch, 16:9, 4:3 or 21:9");
        }
        if self.buffer_ram_mb > BUFFER_RAM_LIMIT_MB {
            bail!("Buffer size must be at most {BUFFER_RAM_LIMIT_MB} MiB");
        }
        if self.buffer_seconds > BUFFER_SECONDS_LIMIT {
            bail!("Buffer length must be at most {BUFFER_SECONDS_LIMIT} seconds");
        }
        if self.seek_step_seconds == 0 || self.seek_step_seconds > 120 {
            bail!("Seek step must be between 1 and 120 seconds");
        }
        if !self.hold_speed.is_finite() || !(0.5..=4.0).contains(&self.hold_speed) {
            bail!("Hold speed must be between 0.5x and 4.0x");
        }
        Ok(())
    }
}

/// Tracks to try to keep when the stream behind the same episode changes. Languages, not track
/// ids: ids differ between mirrors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_language: Option<String>,
    /// `Some(true)` when the user had subtitles on, `Some(false)` when they were switched off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_language: Option<String>,
}

/// Summary of the stream mpv currently has open.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveStream {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub quality: i32,
    pub quality_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Stable identity used to recognise the same mirror again (see `stream_key`).
    pub key: String,
}

fn default_source_priority() -> i32 {
    1
}

pub const PRIORITY_LIMIT: i32 = 100;
pub const MAX_SOURCE_PROFILES: usize = 12;

/// A named quality/source ranking, modelled on Android's `QualityProfile`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    pub id: String,
    pub name: String,
    /// Priority per quality bucket (`"unknown"`, `"144"`, ..., `"2160"`). Missing entries use
    /// the Android defaults.
    #[serde(default)]
    pub quality_priorities: BTreeMap<String, i32>,
    /// Priority per source/extractor name. Missing entries use `1`, like Android.
    #[serde(default)]
    pub source_priorities: BTreeMap<String, i32>,
    /// Hide links whose combined priority is below zero.
    #[serde(default)]
    pub hide_negative_sources: bool,
    /// Hide mirrors that already failed during this playback.
    #[serde(default)]
    pub hide_error_sources: bool,
    /// Tie-break for equal priorities: IETF tag or English name (`en`, `English`, `ja`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_language: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl SourceProfile {
    pub fn default_profile() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            quality_priorities: BTreeMap::new(),
            source_priorities: BTreeMap::new(),
            hide_negative_sources: false,
            hide_error_sources: false,
            preferred_language: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn source_priority(&self, source: &str) -> i32 {
        self.source_priorities
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(source.trim()))
            .map(|(_, p)| *p)
            .unwrap_or_else(default_source_priority)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if self.id.trim().is_empty() {
            bail!("A source profile needs an id");
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 60 {
            bail!("A source profile name must be 1 to 60 characters");
        }
        for (key, p) in &self.quality_priorities {
            if key != "unknown" && !matches!(key.parse::<i32>(), Ok(144 | 240 | 360 | 480 | 720 | 1080 | 1440 | 2160)) {
                bail!("Unknown quality '{key}'. Use unknown, 144, 240, 360, 480, 720, 1080, 1440 or 2160");
            }
            if p.abs() > PRIORITY_LIMIT {
                bail!("Priorities must be within ±{PRIORITY_LIMIT}");
            }
        }
        for (name, p) in &self.source_priorities {
            if name.trim().is_empty() {
                bail!("Source names cannot be empty");
            }
            if p.abs() > PRIORITY_LIMIT {
                bail!("Priorities must be within ±{PRIORITY_LIMIT}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePriorityConfig {
    pub active_profile_id: String,
    pub profiles: Vec<SourceProfile>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for SourcePriorityConfig {
    fn default() -> Self {
        Self {
            active_profile_id: "default".into(),
            profiles: vec![SourceProfile::default_profile()],
            extra: BTreeMap::new(),
        }
    }
}

impl SourcePriorityConfig {
    pub fn active(&self) -> SourceProfile {
        self.profiles
            .iter()
            .find(|p| p.id == self.active_profile_id)
            .or_else(|| self.profiles.first())
            .cloned()
            .unwrap_or_else(SourceProfile::default_profile)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::bail;
        if self.profiles.is_empty() {
            bail!("Keep at least one source profile");
        }
        if self.profiles.len() > MAX_SOURCE_PROFILES {
            bail!("At most {MAX_SOURCE_PROFILES} source profiles are supported");
        }
        let mut seen = std::collections::BTreeSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !seen.insert(profile.id.clone()) {
                bail!("Source profile ids must be unique ('{}')", profile.id);
            }
        }
        if !self.profiles.iter().any(|p| p.id == self.active_profile_id) {
            bail!("The active source profile does not exist");
        }
        Ok(())
    }
}

/// One mirror in ranked order. `index` points into the slice that was ranked, so the caller keeps
/// ownership of the original stream (headers, referer and opaque extractor data untouched).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RankedStream {
    pub index: usize,
    pub key: String,
    pub priority: i32,
    /// `false` when the profile hides this link (negative priority or already failed).
    pub usable: bool,
    pub quality_label: String,
    /// `false` for Auto/Unknown. No resolution is implied.
    pub quality_known: bool,
    pub language_match: bool,
    pub errored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatus {
    pub id: String,
    pub label: String,
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What the installed mpv can actually do, probed once per app run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCapabilities {
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub capabilities: Vec<CapabilityStatus>,
}

impl PlayerCapabilities {
    pub fn supports(&self, id: &str) -> bool {
        self.capabilities.iter().any(|c| c.id == id && c.supported)
    }

    pub fn reason(&self, id: &str) -> Option<String> {
        self.capabilities
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.reason.clone())
    }
}
