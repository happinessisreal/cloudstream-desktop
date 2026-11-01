use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryManifest {
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub manifest_version: u32,
    pub plugin_lists: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub name: String,
    pub data: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub poster_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetails {
    pub item: MediaItem,
    pub plot: Option<String>,
    pub tags: Vec<String>,
    pub episodes: Vec<Episode>,
    pub duration: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub name: String,
    pub url: String,
    pub referer: String,
    pub quality: i32,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub stream_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subtitle {
    pub lang: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedLinks {
    pub streams: Vec<Stream>,
    pub subtitles: Vec<Subtitle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: String,
    pub item: MediaItem,
    pub saved_at: u64,
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
