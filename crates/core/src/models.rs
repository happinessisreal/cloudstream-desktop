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
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default)]
    pub poster_density: PosterDensity,
    #[serde(default = "default_true")]
    pub search_suggestions: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            poster_density: PosterDensity::Comfortable,
            search_suggestions: true,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSettings {
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceSettings::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettings {
    #[serde(default)]
    pub appearance: Option<AppearanceSettings>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub fn default_settings_schema_version() -> u32 {
    1
}

pub fn default_settings_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
