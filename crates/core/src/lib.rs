pub mod download_manager;
pub mod download_queue;
pub mod downloads;
pub mod direct_subtitles;
pub mod fonts;
pub mod hls;
pub mod hls_download;
pub mod subtitle_providers;
pub mod subtitle_online;
pub mod backup;
pub mod backup_android;
pub mod backup_schedule;
pub mod cast;
pub mod diagnostics;
pub mod drm;
pub mod entry;
pub mod external_player;
pub mod instance;
pub mod keyring;
pub mod metadata;
pub mod models;
pub mod mpris;
pub mod notify;
pub mod player;
pub mod repository;
pub mod store;
pub mod streams;
pub mod updates;
pub mod subtitles;
pub mod skip;
pub mod subscriptions;
pub mod tracking;
pub mod worker;

use anyhow::{bail, Context, Result};
use models::*;
use repository::{digest, plugin_id, RepositoryManager};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use store::Store;
use tokio::sync::Mutex;
use worker::{Worker, WorkerConfig};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HomeCacheKey {
    pub provider_id: String,
    pub package_hash: String,
    pub section: u32,
    pub page: u32,
    pub lang: String,
}

#[derive(Debug, Clone)]
pub struct HomeCacheEntry {
    pub data: serde_json::Value,
    pub cached_at: std::time::Instant,
}

/// What the interface shows for a title's metadata.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataResult {
    pub links: Vec<metadata::MetadataLink>,
    pub view: metadata::EnrichedView,
    pub errors: Vec<String>,
    /// Some data came from the cache because a service could not be reached.
    pub stale: bool,
}

/// Everything needed to keep an episode playing when its current mirror fails or the user picks
/// another one. Lives only while that episode plays; never persisted.
#[derive(Debug, Clone, Default)]
pub struct PlaybackContext {
    pub mirrors: Vec<Stream>,
    pub subtitles: Vec<Subtitle>,
    pub errored: std::collections::HashSet<String>,
    pub attempts: u32,
    pub current_key: String,
    pub media_id: Option<String>,
    pub episode_id: Option<String>,
}

impl PlaybackContext {
    fn matches_player(&self, state: &PlayerState) -> bool {
        self.media_id == state.media_id && self.episode_id == state.episode_id
    }
}

#[derive(Debug, Clone)]
pub struct ActiveTransport {
    pub tokens: Vec<String>,
    pub player_session_id: String,
}

pub(crate) struct PendingSubtitleArchive {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) created: std::time::Instant,
}

#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubtitleFileSelection {
    Added,
    Archive { id: String, entries: Vec<subtitles::ArchiveSubtitle> },
}

#[derive(Debug)]
struct PlaybackSuperseded;
impl std::fmt::Display for PlaybackSuperseded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Playback request was superseded") }
}
impl std::error::Error for PlaybackSuperseded {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DetailCacheKey { profile_id: String, provider_id: String, url: String }
#[derive(Debug, Clone)]
struct DetailCacheEntry { details: MediaDetails, cached_at: std::time::Instant }
const DETAIL_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(120);
const DETAIL_CACHE_LIMIT: usize = 64;

pub struct App {
    pub store: Store,
    pub repositories: RepositoryManager,
    pub worker: Worker,
    pub player: player::Player,
    pub resources: PathBuf,
    pub data: PathBuf,
    pub mutation: Mutex<()>,
    pub client_requests: Mutex<std::collections::HashMap<String, u64>>,
    pub home_cache: std::sync::Mutex<std::collections::HashMap<HomeCacheKey, HomeCacheEntry>>,
    detail_cache: std::sync::Mutex<std::collections::HashMap<DetailCacheKey, DetailCacheEntry>>,
    detail_cache_epoch: std::sync::atomic::AtomicU64,
    pub active_transport_token: Arc<tokio::sync::Mutex<Option<ActiveTransport>>>,
    pub playback_context: Mutex<Option<PlaybackContext>>,
    pending_subtitle_archive: Mutex<Option<PendingSubtitleArchive>>,
    playback_intent: std::sync::atomic::AtomicU64,
    active_playback_intent: std::sync::atomic::AtomicU64,
    subscription_delivery: Mutex<()>,
    subscription_check: Mutex<()>,
    /// Serialises backup runs (manual "back up now" and scheduled) so two never race.
    pub backup_lock: Mutex<()>,
    /// Links and files that arrived from outside (a clicked link, "open with", a second launch),
    /// waiting for the interface to be ready to show them.
    pub entries: entry::EntryQueue,
    /// Bounded, redacted recent errors and request timings for the diagnostics view.
    pub diagnostics: diagnostics::Diagnostics,
    /// The release-notes address from the last signed feed check (the only address the app opens).
    pub last_release_notes: std::sync::Mutex<Option<String>>,
    metadata_transport: std::sync::Mutex<Option<Arc<dyn metadata::Transport>>>,
    skip_transport: std::sync::Mutex<Option<Arc<dyn skip::Transport>>>,
    pub(crate) subtitle_http: std::sync::Mutex<Option<Arc<dyn subtitle_providers::Transport>>>,
    pub(crate) subtitle_secrets: std::sync::Mutex<Option<Arc<dyn subtitle_providers::secrets::SecretStore>>>,
    pub(crate) online_subtitles: Mutex<Option<subtitle_online::OnlineSearch>>,
    entry_errors: std::sync::Mutex<Vec<String>>,
}
impl App {
    pub fn open(data: PathBuf, resources: PathBuf) -> Result<Arc<Self>> {
        Self::open_with_player(data, resources, player::Player::new(std::env::temp_dir()))
    }

    /// Like `open`, with a caller supplied player (for example one launched headless in tests).
    pub fn open_with_player(
        data: PathBuf,
        resources: PathBuf,
        player: player::Player,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data)?;
        let ext_dir = data.join("extensions");
        clean_interrupted_updates(&ext_dir);
        let bundled_java = resources.join("jre/bin/java");
        let java = if bundled_java.is_file() {
            bundled_java
        } else {
            PathBuf::from("java")
        };
        let store = Store::open(&data.join("cloudstream.db"))?;
        // Settings are applied before the player is shared, so the first playback already uses them.
        let player = player.with_fonts_directory(data.join("subtitle-fonts"));
        let player = match store.get_settings() {
            Ok(settings) => player.with_defaults(settings.global.playback, settings.global.player),
            Err(_) => player,
        };
        let app = Arc::new(Self {
            store,
            repositories: RepositoryManager::new(ext_dir)?,
            worker: Worker::new(WorkerConfig {
                java,
                jar: resources.join("worker/worker.jar"),
            }),
            player,
            resources,
            data,
            mutation: Mutex::new(()),
            client_requests: Mutex::new(std::collections::HashMap::new()),
            home_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            detail_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            detail_cache_epoch: std::sync::atomic::AtomicU64::new(0),
            active_transport_token: Arc::new(tokio::sync::Mutex::new(None)),
            playback_context: Mutex::new(None),
            pending_subtitle_archive: Mutex::new(None),
            playback_intent: std::sync::atomic::AtomicU64::new(0),
            active_playback_intent: std::sync::atomic::AtomicU64::new(0),
            subscription_delivery: Mutex::new(()),
            subscription_check: Mutex::new(()),
            backup_lock: Mutex::new(()),
            entries: entry::EntryQueue::default(),
            diagnostics: diagnostics::Diagnostics::default(),
            last_release_notes: std::sync::Mutex::new(None),
            metadata_transport: std::sync::Mutex::new(None),
            skip_transport: std::sync::Mutex::new(None),
            subtitle_http: std::sync::Mutex::new(None),
            subtitle_secrets: std::sync::Mutex::new(None),
            online_subtitles: Mutex::new(None),
            entry_errors: std::sync::Mutex::new(Vec::new()),
        });

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let flusher = app.clone();
            // Subscribe before spawning so an immediately failing player cannot emit its
            // terminal event before the background task begins running.
            let mut rx = flusher.player.subscribe();
            handle.spawn(async move {
                let mut last_saved_pos: f64 = 0.0;
                let mut last_saved_time = std::time::Instant::now();
                let mut current_session: Option<String> = None;

                while let Ok(event) = rx.recv().await {
                    match event {
                        PlayerEvent::StateChanged { ref state } => {
                            if state.session_id != current_session {
                                current_session = state.session_id.clone();
                                last_saved_pos = state.position;
                                last_saved_time = std::time::Instant::now();
                            }
                            let pos_delta = (state.position - last_saved_pos).abs();
                            let time_elapsed = last_saved_time.elapsed().as_secs();
                            if state.paused || (pos_delta >= 5.0 && time_elapsed >= 5) {
                                let _ = flusher.flush_player_progress().await;
                                last_saved_pos = state.position;
                                last_saved_time = std::time::Instant::now();
                            }
                        }
                        PlayerEvent::Error { ref message, .. } => flusher.diagnostics.record("player", message),
                        PlayerEvent::Notice { ref message, .. } => flusher.diagnostics.record("player-notice", message),
                        PlayerEvent::EndFile { ref session_id, ref reason, ref detail } => {
                            let _ = flusher.flush_player_progress().await;
                            if reason == "eof" {
                                let state = flusher.player.state().await;
                                let is_live = state.duration <= 0.0 || !state.duration.is_finite();
                                if !is_live {
                                    if let Some((watch, profile_id)) = flusher.player.current_watch().await {
                                        let media_id = watch.item.media_id.as_deref().unwrap_or("");
                                        let _ = flusher.store.mark_episode_watched(&profile_id, media_id, &watch.id, true);
                                    }
                                    let policy = flusher.player.policy().await;
                                    if policy.auto_next {
                                        let _ = flusher.advance_queue_and_play(true).await;
                                    }
                                }
                                // A completed final episode has no later shutdown while mpv is
                                // kept open. Ownership protects a newly started queue item here.
                                flusher.close_transport_for(session_id).await;
                            } else if reason == "error" {
                                if !flusher
                                    .failover_playback(session_id, detail.as_deref())
                                    .await
                                    .unwrap_or(false)
                                {
                                    flusher.close_transport_for(session_id).await;
                                }
                            } else {
                                flusher.close_transport_for(session_id).await;
                            }
                            current_session = None;
                        }
                        PlayerEvent::Shutdown { ref session_id } => {
                            let _ = flusher.flush_player_progress().await;
                            flusher.close_transport_for(session_id).await;
                            current_session = None;
                        }
                        _ => {}
                    }
                }
            });
        }

        Ok(app)
    }
    pub async fn status(&self) -> RuntimeStatus {
        let bundled_java = self.resources.join("jre/bin/java");
        let java = if bundled_java.is_file() {
            bundled_java
        } else {
            PathBuf::from("java")
        };
        let available = |path: PathBuf, argument: &'static str| async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::process::Command::new(path).arg(argument).output(),
            )
            .await
            .is_ok_and(|r| r.is_ok_and(|o| o.status.success()))
        };
        let jvm = available(java.clone(), "-version").await;
        let mpv = available(PathBuf::from("mpv"), "--version").await;
        let worker_path = self.resources.join("worker/worker.jar");
        let mut notes = Vec::new();
        if !worker_path.is_file() {
            notes.push("Build the extension worker with npm run worker:build.".into());
        }
        if !jvm {
            notes.push("Java 21+ is needed until the bundled runtime is built.".into());
        }
        if !mpv {
            notes.push("Install mpv to play resolved streams.".into());
        }
        notes.push(
            "Data-only .cs3 packages can use the desktop DEX bridge. Full Android APIs and WebView remain unsupported."
                .into(),
        );
        RuntimeStatus {
            jvm_available: jvm,
            worker_available: worker_path.is_file(),
            mpv_available: mpv,
            android_available: false,
            java_path: java.to_string_lossy().into(),
            worker_path: worker_path.to_string_lossy().into(),
            notes,
        }
    }
    pub async fn providers(&self) -> Result<Vec<Provider>> {
        let installed = self.store.installed()?;
        if installed.is_empty() {
            return Ok(vec![]);
        }
        let mut list: Vec<Provider> = serde_json::from_value(
            self.worker
                .request(&installed, "listProviders", json!({}))
                .await?,
        )?;
        let disabled_set = self.store.list_disabled_providers()?;
        for p in &mut list {
            p.disabled = disabled_set.contains(&p.id);
        }

        let clones = self.store.list_provider_clones()?;
        let mut cloned = Vec::new();
        for clone in clones {
            if let Some(parent) = list.iter().find(|p| p.id == clone.parent_provider_id) {
                let mut cp = parent.clone();
                cp.id = clone.id.clone();
                cp.name = clone.name.clone();
                cp.main_url = clone.custom_url.clone();
                cp.disabled = !clone.enabled || parent.disabled;
                cp.clone_of = Some(parent.id.clone());
                cloned.push(cp);
            }
        }
        list.extend(cloned);
        Ok(list)
    }
    pub fn set_provider_enabled(&self, provider_id: &str, enabled: bool) -> Result<()> {
        self.invalidate_home_cache(Some(provider_id));
        self.store.set_provider_disabled(provider_id, !enabled)
    }
    pub fn configure_provider_clone(&self, clone: ProviderCloneConfig) -> Result<()> {
        self.invalidate_home_cache(Some(&clone.id));
        self.store.save_provider_clone(&clone)
    }
    pub fn delete_provider_clone(&self, clone_id: &str) -> Result<()> {
        self.invalidate_home_cache(Some(clone_id));
        self.store.delete_provider_clone(clone_id)
    }
    pub async fn provider_home(
        &self,
        provider_id: &str,
        page: u32,
        section: u32,
        force_refresh: bool,
        request_id: Option<&str>,
    ) -> Result<Value> {
        let providers = self.providers().await.unwrap_or_default();
        let (package_hash, lang) = if let Some(p) = providers.iter().find(|p| p.id == provider_id) {
            let installed = self.store.installed().unwrap_or_default();
            let hash = installed
                .iter()
                .find(|pi| pi.id == p.plugin_id)
                .map(|pi| pi.sha256.clone())
                .unwrap_or_default();
            (hash, p.lang.clone())
        } else {
            (String::new(), "en".to_string())
        };

        let key = HomeCacheKey {
            provider_id: provider_id.to_string(),
            package_hash,
            section,
            page,
            lang,
        };

        const TTL: std::time::Duration = std::time::Duration::from_secs(600);

        if !force_refresh {
            let cache = self.home_cache.lock().unwrap();
            if let Some(entry) = cache.get(&key) {
                if entry.cached_at.elapsed() < TTL {
                    let mut data = entry.data.clone();
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("cached".to_string(), Value::Bool(true));
                    }
                    return Ok(data);
                }
            }
        }

        let res = self
            .call_cancellable(
                request_id,
                "getMainPage",
                json!({"providerId": provider_id, "page": page, "section": section}),
            )
            .await?;

        // Avoid caching unresolved errors as empty success
        if res.get("error").is_none() && res.get("rows").is_some() {
            let mut cache = self.home_cache.lock().unwrap();
            if cache.len() >= 50 {
                let now = std::time::Instant::now();
                cache.retain(|_, v| now.duration_since(v.cached_at) < TTL);
                if cache.len() >= 50 {
                    if let Some(oldest_key) = cache.keys().next().cloned() {
                        cache.remove(&oldest_key);
                    }
                }
            }
            cache.insert(
                key,
                HomeCacheEntry {
                    data: res.clone(),
                    cached_at: std::time::Instant::now(),
                },
            );
        }

        let mut data = res;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("cached".to_string(), Value::Bool(false));
        }
        Ok(data)
    }
    pub fn invalidate_home_cache(&self, provider_id: Option<&str>) {
        // Provider clones may depend on their parent's configuration, so clear all detail
        // entries on any source mutation. An epoch stops older in-flight requests repopulating it.
        self.detail_cache_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.detail_cache.lock().unwrap().clear();
        let mut cache = self.home_cache.lock().unwrap();
        if let Some(pid) = provider_id {
            cache.retain(|k, _| k.provider_id != pid);
        } else {
            cache.clear();
        }
    }
    pub async fn add_repository(&self, input: &str) -> Result<Repository> {
        let _guard = self.mutation.lock().await;
        let resolved_url = repository::resolve_repository_url(&self.repositories.client, input).await?;
        let normalized = repository::normalize_repository(&resolved_url)?;

        let existing = self.store.repositories()?.into_iter().find(|r| r.url == normalized);
        let mut fetched = repository::fetch_repository(&self.repositories.client, &normalized).await?;
        fetched.updated_at = Some(timestamp_ms());

        if let Some(prev) = existing {
            fetched.custom_name = prev.custom_name;
        }

        self.store.put("repository", &fetched.url, &fetched)?;
        Ok(fetched)
    }
    pub async fn refresh_repository(&self, url: &str) -> Result<Repository> {
        let _guard = self.mutation.lock().await;
        let normalized = repository::normalize_repository(url)?;
        let existing = self.store.repositories()?.into_iter().find(|r| r.url == normalized)
            .context("Repository is not added")?;
        let mut fetched = repository::fetch_repository(&self.repositories.client, &normalized).await?;
        fetched.custom_name = existing.custom_name;
        fetched.updated_at = Some(timestamp_ms());
        self.store.put("repository", &fetched.url, &fetched)?;
        Ok(fetched)
    }
    pub fn rename_repository(&self, url: &str, name: Option<String>) -> Result<Repository> {
        let normalized = repository::normalize_repository(url)?;
        let mut existing = self.store.repositories()?.into_iter().find(|r| r.url == normalized)
            .context("Repository is not added")?;
        let clean_name = name.and_then(|n| {
            let t = n.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });
        existing.custom_name = clean_name;
        self.store.put("repository", &existing.url, &existing)?;
        Ok(existing)
    }
    pub async fn install_local(&self, file_path: &std::path::Path) -> Result<InstalledPlugin> {
        let _guard = self.mutation.lock().await;
        let plugin = self.repositories.prepare_local(file_path)?;
        self.activate(&plugin).await?;
        Ok(plugin)
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_cancellable(None, method, params).await
    }
    pub async fn call_cancellable(
        &self,
        request_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let mut effective_params = params;
        if let Some(pid) = effective_params.get("providerId").and_then(|v| v.as_str()) {
            if let Ok(clones) = self.store.list_provider_clones() {
                if let Some(clone) = clones.into_iter().find(|c| c.id == pid) {
                    effective_params["providerId"] = json!(clone.parent_provider_id);
                }
            }
        }

        let installed = self.store.installed()?;
        let id = self.worker.next_id();
        if let Some(req_id) = request_id {
            self.client_requests
                .lock()
                .await
                .insert(req_id.to_string(), id);
        }
        let started = std::time::Instant::now();
        let res = self
            .worker
            .request_with_id(&installed, id, method, effective_params, None)
            .await;
        if let Some(req_id) = request_id {
            self.client_requests.lock().await.remove(req_id);
        }
        // Only the method name, how long it took and a coarse outcome are kept; never parameters
        // or provider messages, which can hold addresses and tokens.
        let outcome = match &res {
            Ok(_) => "ok",
            Err(error) => diagnostics::classify_error(error),
        };
        self.diagnostics.record_timing(method, started.elapsed().as_millis() as u64, outcome);
        if let Err(error) = &res {
            self.diagnostics.record("worker", &format!("{method} failed: {error:#}"));
        }
        res
    }
    /// Load fresh details for the UI and retain usable episodes briefly for background checks.
    pub async fn load_details(&self, profile_id: &str, provider_id: &str, url: &str, request_id: Option<&str>) -> Result<MediaDetails> {
        let epoch = self.detail_cache_epoch.load(std::sync::atomic::Ordering::SeqCst);
        let started_at = std::time::Instant::now();
        let value = self.call_cancellable(request_id, "load", json!({"providerId":provider_id,"url":url})).await?;
        let details: MediaDetails = serde_json::from_value(value).context("Provider returned invalid details")?;
        if subscriptions::episode_snapshot(&details).is_ok() {
            let mut cache = self.detail_cache.lock().unwrap();
            if self.detail_cache_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch {
                cache.retain(|_, entry| entry.cached_at.elapsed() < DETAIL_CACHE_TTL);
                let key = DetailCacheKey { profile_id: profile_id.into(), provider_id: provider_id.into(), url: url.into() };
                if started_at.elapsed() < DETAIL_CACHE_TTL && cache.get(&key).is_none_or(|entry| entry.cached_at <= started_at) {
                    if cache.len() >= DETAIL_CACHE_LIMIT && !cache.contains_key(&key) {
                        if let Some(oldest) = cache.iter().min_by_key(|(_, entry)| entry.cached_at).map(|(key, _)| key.clone()) { cache.remove(&oldest); }
                    }
                    cache.insert(key, DetailCacheEntry { details: details.clone(), cached_at: started_at });
                }
            }
        }
        Ok(details)
    }

    fn cached_subscription_details(&self, profile_id: &str, provider_id: &str, url: &str) -> Option<MediaDetails> {
        let key = DetailCacheKey { profile_id: profile_id.into(), provider_id: provider_id.into(), url: url.into() };
        let mut cache = self.detail_cache.lock().unwrap();
        cache.retain(|_, entry| entry.cached_at.elapsed() < DETAIL_CACHE_TTL);
        cache.get(&key).map(|entry| entry.details.clone())
    }

    pub async fn cancel_request(&self, request_id: &str) -> Result<bool> {
        let id_opt = {
            let mut map = self.client_requests.lock().await;
            map.remove(request_id)
        };
        if let Some(internal_id) = id_opt {
            self.worker.cancel(internal_id).await;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub async fn extension_states(&self) -> Result<Value> {
        if self.store.installed()?.is_empty() {
            return Ok(json!([]));
        }
        self.call("getPluginStates", json!({})).await
    }
    pub async fn check_for_updates(&self) -> Result<Vec<ExtensionUpdate>> {
        let installed = self.store.installed()?;
        let repositories = self.store.repositories()?;
        let mut updates = Vec::new();

        for plugin in installed {
            if let Some(repo) = repositories.iter().find(|r| r.url == plugin.repository_url) {
                if let Some(entry) = repo
                    .plugins
                    .iter()
                    .find(|e| e.internal_name == plugin.entry.internal_name)
                {
                    let has_update = entry.version > plugin.entry.version;
                    let changelog = entry
                        .extra
                        .get("changelog")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                    updates.push(ExtensionUpdate {
                        plugin_id: plugin.id.clone(),
                        name: plugin.entry.name.clone(),
                        repository_url: plugin.repository_url.clone(),
                        installed_version: plugin.entry.version,
                        available_version: entry.version,
                        changelog,
                        has_update,
                        entry: entry.clone(),
                    });
                }
            }
        }
        Ok(updates)
    }

    pub async fn update_extension(&self, plugin_id: &str) -> Result<InstalledPlugin> {
        let installed = self.store.installed()?;
        let target = installed
            .into_iter()
            .find(|p| p.id == plugin_id)
            .context("Extension is not currently installed")?;

        self.install(&target.repository_url, &target.entry.internal_name)
            .await
    }

    pub async fn update_all_extensions(&self) -> Result<Vec<BulkUpdateResult>> {
        let candidates = self.check_for_updates().await?;
        let mut results = Vec::new();

        for candidate in candidates.into_iter().filter(|c| c.has_update) {
            let prev_ver = candidate.installed_version;
            match self.update_extension(&candidate.plugin_id).await {
                Ok(updated) => {
                    results.push(BulkUpdateResult {
                        plugin_id: candidate.plugin_id,
                        name: candidate.name,
                        success: true,
                        error: None,
                        previous_version: prev_ver,
                        updated_version: Some(updated.entry.version),
                    });
                }
                Err(e) => {
                    results.push(BulkUpdateResult {
                        plugin_id: candidate.plugin_id,
                        name: candidate.name,
                        success: false,
                        error: Some(e.to_string()),
                        previous_version: prev_ver,
                        updated_version: None,
                    });
                }
            }
        }
        Ok(results)
    }

    pub async fn test_provider(
        &self,
        provider_id: &str,
        request_id: Option<&str>,
    ) -> Result<ProviderTestReport> {
        let providers = self.providers().await?;
        let provider = providers
            .into_iter()
            .find(|p| p.id == provider_id)
            .context("Provider not found or disabled")?;

        let start_total = std::time::Instant::now();
        let mut stages = Vec::new();
        let mut candidate_url: Option<String> = None;
        let mut candidate_data: Option<String> = None;
        let caps = provider.capabilities.clone().unwrap_or_default();

        // Stage 1: Home
        if caps.has_main_page {
            let stage_start = std::time::Instant::now();
            let home_res = self
                .call_cancellable(
                    request_id,
                    "getMainPage",
                    json!({ "providerId": provider.id, "page": 1, "section": 0 }),
                )
                .await;
            let elapsed = stage_start.elapsed().as_millis() as u64;

            match home_res {
                Ok(val) => {
                    let home: Result<HomeResponse, _> = serde_json::from_value(val);
                    match home {
                        Ok(resp) => {
                            let total_items: usize = resp.rows.iter().map(|r| r.items.len()).sum();
                            if let Some(first_item) = resp.rows.iter().flat_map(|r| &r.items).next() {
                                candidate_url = Some(first_item.url.clone());
                            }
                            stages.push(ProviderTestStageResult {
                                stage: "home".into(),
                                status: "passed".into(),
                                duration_ms: elapsed,
                                message: Some(format!(
                                    "Loaded {} rows with {} items",
                                    resp.rows.len(),
                                    total_items
                                )),
                                items_count: Some(total_items),
                            });
                        }
                        Err(e) => {
                            stages.push(ProviderTestStageResult {
                                stage: "home".into(),
                                status: "failed".into(),
                                duration_ms: elapsed,
                                message: Some(format!("Invalid home response schema: {e}")),
                                items_count: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    stages.push(ProviderTestStageResult {
                        stage: "home".into(),
                        status: "failed".into(),
                        duration_ms: elapsed,
                        message: Some(e.to_string()),
                        items_count: None,
                    });
                }
            }
        } else {
            stages.push(ProviderTestStageResult {
                stage: "home".into(),
                status: "skipped".into(),
                duration_ms: 0,
                message: Some("Provider lacks main page capability".into()),
                items_count: None,
            });
        }

        // Stage 2: Search
        let test_query = "bunny";
        let stage_start = std::time::Instant::now();
        let search_res = self
            .call_cancellable(
                request_id,
                "search",
                json!({ "providerId": provider.id, "query": test_query, "page": 1 }),
            )
            .await;
        let elapsed = stage_start.elapsed().as_millis() as u64;

        match search_res {
            Ok(val) => {
                let items: Result<Vec<MediaItem>, _> = serde_json::from_value(
                    val.get("items")
                        .cloned()
                        .unwrap_or(serde_json::Value::Array(vec![])),
                );
                match items {
                    Ok(item_list) => {
                        if candidate_url.is_none() {
                            if let Some(first) = item_list.first() {
                                candidate_url = Some(first.url.clone());
                            }
                        }
                        stages.push(ProviderTestStageResult {
                            stage: "search".into(),
                            status: "passed".into(),
                            duration_ms: elapsed,
                            message: Some(format!("Returned {} search items", item_list.len())),
                            items_count: Some(item_list.len()),
                        });
                    }
                    Err(e) => {
                        stages.push(ProviderTestStageResult {
                            stage: "search".into(),
                            status: "failed".into(),
                            duration_ms: elapsed,
                            message: Some(format!("Invalid search response schema: {e}")),
                            items_count: None,
                        });
                    }
                }
            }
            Err(e) => {
                stages.push(ProviderTestStageResult {
                    stage: "search".into(),
                    status: "failed".into(),
                    duration_ms: elapsed,
                    message: Some(e.to_string()),
                    items_count: None,
                });
            }
        }

        // Stage 3: Details (load)
        if let Some(url) = candidate_url {
            let stage_start = std::time::Instant::now();
            let load_res = self
                .call_cancellable(
                    request_id,
                    "load",
                    json!({ "providerId": provider.id, "url": url }),
                )
                .await;
            let elapsed = stage_start.elapsed().as_millis() as u64;

            match load_res {
                Ok(val) => {
                    let details: Result<MediaDetails, _> = serde_json::from_value(val);
                    match details {
                        Ok(det) => {
                            let ep_count = det.episodes.len();
                            if let Some(first_ep) = det.episodes.first() {
                                candidate_data = Some(first_ep.data.clone());
                            }
                            stages.push(ProviderTestStageResult {
                                stage: "details".into(),
                                status: "passed".into(),
                                duration_ms: elapsed,
                                message: Some(format!("Loaded details with {} episodes", ep_count)),
                                items_count: Some(ep_count),
                            });
                        }
                        Err(e) => {
                            stages.push(ProviderTestStageResult {
                                stage: "details".into(),
                                status: "failed".into(),
                                duration_ms: elapsed,
                                message: Some(format!("Invalid details schema: {e}")),
                                items_count: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    stages.push(ProviderTestStageResult {
                        stage: "details".into(),
                        status: "failed".into(),
                        duration_ms: elapsed,
                        message: Some(e.to_string()),
                        items_count: None,
                    });
                }
            }
        } else {
            stages.push(ProviderTestStageResult {
                stage: "details".into(),
                status: "skipped".into(),
                duration_ms: 0,
                message: Some("No candidate media item available to load".into()),
                items_count: None,
            });
        }

        // Stage 4: Links (loadLinks)
        if let Some(data) = candidate_data {
            let stage_start = std::time::Instant::now();
            let links_res = self
                .call_cancellable(
                    request_id,
                    "loadLinks",
                    json!({ "providerId": provider.id, "data": data }),
                )
                .await;
            let elapsed = stage_start.elapsed().as_millis() as u64;

            match links_res {
                Ok(val) => {
                    let links: Result<ResolvedLinks, _> = serde_json::from_value(val);
                    match links {
                        Ok(l) => {
                            stages.push(ProviderTestStageResult {
                                stage: "links".into(),
                                status: "passed".into(),
                                duration_ms: elapsed,
                                message: Some(format!(
                                    "Resolved {} streams, {} subtitles",
                                    l.streams.len(),
                                    l.subtitles.len()
                                )),
                                items_count: Some(l.streams.len()),
                            });
                        }
                        Err(e) => {
                            stages.push(ProviderTestStageResult {
                                stage: "links".into(),
                                status: "failed".into(),
                                duration_ms: elapsed,
                                message: Some(format!("Invalid links schema: {e}")),
                                items_count: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    stages.push(ProviderTestStageResult {
                        stage: "links".into(),
                        status: "failed".into(),
                        duration_ms: elapsed,
                        message: Some(e.to_string()),
                        items_count: None,
                    });
                }
            }
        } else {
            stages.push(ProviderTestStageResult {
                stage: "links".into(),
                status: "skipped".into(),
                duration_ms: 0,
                message: Some("No candidate episode data available to test links".into()),
                items_count: None,
            });
        }

        let all_passed = stages
            .iter()
            .all(|s| s.status == "passed" || s.status == "skipped");
        let total_elapsed = start_total.elapsed().as_millis() as u64;

        Ok(ProviderTestReport {
            provider_id: provider.id,
            provider_name: provider.name,
            passed: all_passed,
            total_duration_ms: total_elapsed,
            stages,
            error: None,
        })
    }

    pub async fn install(
        &self,
        repository_url: &str,
        internal_name: &str,
    ) -> Result<InstalledPlugin> {
        let _guard = self.mutation.lock().await;
        let repo = self
            .store
            .repositories()?
            .into_iter()
            .find(|r| r.url == repository_url)
            .context("Repository is missing. Add or refresh it first")?;
        let entry = repo
            .plugins
            .into_iter()
            .find(|e| e.internal_name == internal_name)
            .context("Extension is missing from this repository")?;
        let plugin = self.repositories.prepare(repository_url, &entry).await?;
        self.activate(&plugin).await?;
        Ok(plugin)
    }
    async fn activate(&self, plugin: &InstalledPlugin) -> Result<()> {
        let previous = self.store.installed()?;
        if let Err(error) = self
            .worker
            .request(
                &previous,
                "loadPlugin",
                json!({"pluginId":plugin.id,"path":plugin.path}),
            )
            .await
        {
            if let Some(old) = previous.iter().find(|old| old.id == plugin.id) {
                let _ = self
                    .worker
                    .request(
                        &previous,
                        "loadPlugin",
                        json!({"pluginId":old.id,"path":old.path}),
                    )
                    .await;
            }
            if plugin.runtime == RuntimeKind::Jvm || previous.iter().any(|old| old.id == plugin.id)
            {
                return Err(error.context(
                    "Extension could not be loaded; the previous installed version was kept",
                ));
            }
        }
        // Preserve unsupported first-time CS3 downloads with their actual load error.
        if let Err(error) = self.store.put("plugin", &plugin.id, plugin) {
            self.worker.stop().await;
            return Err(error);
        }
        self.invalidate_home_cache(None);
        Ok(())
    }
    pub async fn install_sample(&self) -> Result<InstalledPlugin> {
        let _guard = self.mutation.lock().await;
        let bytes = std::fs::read(self.resources.join("worker/desktop-sample.jar"))
            .context("Sample extension is missing. Run npm run worker:build")?;
        let hash = format!("sha256-{}", digest(&bytes));
        let entry = PluginEntry {
            url: "https://www.blender.org/about/projects/".into(),
            name: "Blender Open Movies".into(),
            internal_name: "desktop-sample".into(),
            version: 1,
            status: 1,
            api_version: 1,
            authors: vec!["Cloudstream Desktop contributors".into()],
            description: Some(
                "A bundled compatibility sample using official Blender open movie downloads."
                    .into(),
            ),
            language: Some("en".into()),
            icon_url: None,
            repository_url: None,
            tv_types: Some(vec!["Movie".into()]),
            file_size: None,
            file_hash: None,
            jar_url: Some("bundled://desktop-sample.jar".into()),
            jar_hash: Some(hash.clone()),
            jar_file_size: Some(bytes.len() as u64),
            source_url: None,
            extra: std::collections::BTreeMap::new(),
        };
        let plugin = self.repositories.prepare_bytes(
            "bundled://samples",
            entry,
            &bytes,
            Some(&hash),
            RuntimeKind::Jvm,
        )?;
        self.activate(&plugin).await?;
        Ok(plugin)
    }
    pub async fn install_torrentio(&self) -> Result<InstalledPlugin> {
        let _guard = self.mutation.lock().await;
        let bytes = std::fs::read(self.resources.join("worker/torrentio.jar"))
            .context("Torrentio extension is missing. Run npm run worker:build")?;
        let hash = format!("sha256-{}", digest(&bytes));
        let entry = PluginEntry {
            url: "https://torrentio.strem.fun".into(),
            name: "Torrentio".into(),
            internal_name: "torrentio".into(),
            version: 1,
            status: 1,
            api_version: 1,
            authors: vec!["Torrentio".into()],
            description: Some(
                "Torrent and magnet stream resolver for Cloudstream Desktop."
                    .into(),
            ),
            language: Some("en".into()),
            icon_url: None,
            repository_url: None,
            tv_types: Some(vec!["Movie".into(), "TvSeries".into()]),
            file_size: None,
            file_hash: None,
            jar_url: Some("bundled://torrentio.jar".into()),
            jar_hash: Some(hash.clone()),
            jar_file_size: Some(bytes.len() as u64),
            source_url: None,
            extra: std::collections::BTreeMap::new(),
        };
        let plugin = self.repositories.prepare_bytes(
            "bundled://samples",
            entry,
            &bytes,
            Some(&hash),
            RuntimeKind::Jvm,
        )?;
        self.activate(&plugin).await?;
        Ok(plugin)
    }
    pub fn get_custom_trackers(&self) -> Result<Vec<String>> {
        let path = self.data.join("trackers.txt");
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(path)?;
        let trackers = content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        Ok(trackers)
    }
    pub fn save_custom_trackers(&self, trackers: &[String]) -> Result<()> {
        let path = self.data.join("trackers.txt");
        let content = trackers
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, content)?;
        Ok(())
    }
    pub async fn remove_plugin(&self, id: &str) -> Result<()> {
        let _guard = self.mutation.lock().await;
        if !self.store.installed()?.iter().any(|p| p.id == id) {
            bail!("Extension is not installed")
        }
        self.invalidate_home_cache(None);
        self.worker.stop().await;
        self.store.remove("plugin", id)?;
        // Retain content-addressed packages as a local recovery cache.
        Ok(())
    }
    pub async fn remove_repository(&self, url: &str) -> Result<()> {
        let _guard = self.mutation.lock().await;
        self.invalidate_home_cache(None);
        self.worker.stop().await;
        for plugin in self
            .store
            .installed()?
            .iter()
            .filter(|p| p.repository_url == url)
        {
            self.store.remove("plugin", &plugin.id)?;
        }
        self.store.remove("repository", url)
    }
    pub fn bookmark(&self, mut item: MediaItem) -> Result<LibraryItem> {
        let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
        item.media_id = Some(media_id.clone());
        let legacy_id = plugin_id(&item.provider_id, &item.url);
        let now_ms = timestamp_ms();

        let mut entry = self.store.get_library_item(&media_id, None)?.unwrap_or_else(|| LibraryItem {
            id: media_id.clone(),
            item: item.clone(),
            saved_at: now_ms,
            watch_status: WatchStatus::None,
            is_favorite: false,
            is_subscribed: false,
            profile_id: Some("default".into()),
            updated_at: Some(now_ms),
            extra: std::collections::BTreeMap::new(),
        });
        entry.item = item;
        entry.updated_at = Some(now_ms);
        self.store.put("library", &format!("default::{}", entry.id), &entry)?;
        self.store.put_alias(&legacy_id, "media", &media_id)?;
        Ok(entry)
    }

    pub fn set_library_status(&self, item: MediaItem, status: WatchStatus, profile_id: Option<&str>) -> Result<LibraryItem> {
        self.store.set_library_status(item, status, profile_id)
    }

    pub fn set_library_favorite(&self, item: MediaItem, is_favorite: bool, profile_id: Option<&str>) -> Result<LibraryItem> {
        self.store.set_library_favorite(item, is_favorite, profile_id)
    }

    pub fn set_library_subscribed(&self, item: MediaItem, is_subscribed: bool, profile_id: Option<&str>) -> Result<LibraryItem> {
        self.store.set_library_subscribed(item, is_subscribed, profile_id)
    }

    pub fn remove_library_item(&self, id: &str, profile_id: Option<&str>) -> Result<()> {
        self.store.remove_library_item(id, profile_id)
    }

    pub fn clear_library(&self, status: Option<WatchStatus>, profile_id: Option<&str>) -> Result<()> {
        self.store.clear_library(status, profile_id)
    }

    pub fn remove_history_item(&self, id: &str, profile_id: Option<&str>) -> Result<()> {
        self.store.remove_history_item(id, profile_id)
    }

    pub fn clear_history(&self, profile_id: Option<&str>) -> Result<()> {
        self.store.clear_history(profile_id)
    }

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        self.store.list_profiles()
    }

    pub fn get_profile(&self, id: &str) -> Result<Option<Profile>> {
        self.store.get_profile(id)
    }

    pub fn create_profile(&self, input: CreateProfileInput) -> Result<Profile> {
        self.store.create_profile(input)
    }

    pub fn update_profile(&self, id: &str, input: UpdateProfileInput) -> Result<Profile> {
        self.store.update_profile(id, input)
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        self.store.delete_profile(id)
    }

    pub fn verify_profile_pin(&self, id: &str, pin: &str) -> Result<bool> {
        self.store.verify_profile_pin(id, pin)
    }

    pub async fn switch_profile(&self, id: &str, pin: Option<&str>) -> Result<()> {
        let profile = self.store.get_profile(id)?.context("Profile not found")?;
        if profile.has_pin {
            let Some(p) = pin else {
                bail!("PIN required to unlock this profile");
            };
            if !self.store.verify_profile_pin(id, p)? {
                bail!("Incorrect PIN");
            }
        }

        let _ = self.flush_player_progress().await;
        self.player.set_queue(None).await;
        self.invalidate_home_cache(None);
        Ok(())
    }

    pub fn get_media_watch_progress(&self, media_id: &str, profile_id: Option<&str>) -> Result<Vec<WatchProgress>> {
        self.store.get_media_watch_progress(profile_id.unwrap_or("default"), media_id)
    }

    pub fn mark_episode_watched(
        &self,
        media_id: &str,
        episode_id: &str,
        is_watched: bool,
        profile_id: Option<&str>,
    ) -> Result<WatchProgress> {
        self.store.mark_episode_watched(profile_id.unwrap_or("default"), media_id, episode_id, is_watched)
    }

    pub fn mark_episodes_up_to(
        &self,
        media_id: &str,
        target_episode_id: &str,
        episodes: &[Episode],
        profile_id: Option<&str>,
    ) -> Result<Vec<WatchProgress>> {
        self.store.mark_episodes_up_to(profile_id.unwrap_or("default"), media_id, target_episode_id, episodes)
    }
    pub async fn play(
        &self,
        stream: Stream,
        subtitles: Vec<Subtitle>,
        item: Option<MediaItem>,
        episode: Option<Episode>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
    ) -> Result<String> {
        self.play_with_mirrors(stream, Vec::new(), subtitles, item, episode, resume, profile_id, queue)
            .await
    }

    /// Start `stream`, remembering the other mirrors of the same episode so a manual switch or a
    /// bounded automatic failover can use them without re-resolving links.
    #[allow(clippy::too_many_arguments)]
    pub async fn play_with_mirrors(
        &self,
        stream: Stream,
        mirrors: Vec<Stream>,
        subtitles: Vec<Subtitle>,
        item: Option<MediaItem>,
        episode: Option<Episode>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
    ) -> Result<String> {
        let intent = self.playback_intent.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let key = streams::stream_key(&stream);
        let (media_id, episode_id) = match (&item, &episode) {
            (Some(item), Some(episode)) => {
                let media = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
                let episode_id = compute_episode_id(&media, episode, episode.dub_status, None);
                (Some(media), Some(episode_id))
            }
            _ => (None, None),
        };
        let mut all = mirrors;
        if !all.iter().any(|m| streams::stream_key(m) == key) {
            all.insert(0, stream.clone());
        }
        let mut context_guard = self.playback_context.lock().await;
        if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent { return Err(PlaybackSuperseded.into()); }
        *context_guard = Some(PlaybackContext {
            mirrors: all,
            subtitles: subtitles.clone(),
            errored: Default::default(),
            attempts: 0,
            current_key: key,
            media_id,
            episode_id,
        });
        drop(context_guard);
        let result = self
            .start_stream(stream, subtitles, item, episode, resume, profile_id, queue, &TrackChoice::default(), intent)
            .await;
        if result.is_err() {
            let mut guard = self.playback_context.lock().await;
            if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) == intent { *guard = None; }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_stream(
        &self,
        stream: Stream,
        subtitles: Vec<Subtitle>,
        item: Option<MediaItem>,
        episode: Option<Episode>,
        resume: f64,
        profile_id: Option<&str>,
        queue: Option<PlaybackQueue>,
        carry: &TrackChoice,
        intent: u64,
    ) -> Result<String> {
        // Refuse what cannot be played before anything is saved or any session is opened.
        if let Some(reason) = drm::unavailable_reason(&stream) { bail!("{reason}"); }
        let _ = self.save_progress().await;
        let profile_id_str = profile_id.unwrap_or("default");
        let watch = match (item, episode) {
            (Some(mut item), Some(mut episode)) => {
                let media_id =
                    compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
                item.media_id = Some(media_id.clone());
                let episode_id = compute_episode_id(&media_id, &episode, episode.dub_status, None);
                episode.id = Some(episode_id.clone());

                self.store
                    .save_episode_payload(&episode_id, &media_id, &episode.data)?;
                let legacy_data_alias = plugin_id(&item.provider_id, &episode.data);
                self.store
                    .put_alias(&legacy_data_alias, "episode", &episode_id)?;

                let effective_resume = if resume > 0.0 {
                    resume
                } else if let Ok(Some(existing_progress)) =
                    self.store.get_watch_progress(profile_id_str, &episode_id)
                {
                    existing_progress.position_seconds
                } else {
                    0.0
                };

                let watch = WatchEntry {
                    id: episode_id,
                    item,
                    episode,
                    position: effective_resume,
                    duration: 0.0,
                    watched_at: timestamp_ms(),
                };
                Some(watch)
            }
            _ => None,
        };
        let mut final_stream = stream.clone();
        let mut final_subtitles: Vec<Subtitle> = subtitles.iter().filter(|sub| !subtitles::is_archive_subtitle(sub)).cloned().collect();
        let has_handles = stream.resource_handle.is_some()
            || stream.audio_tracks.iter().any(|track| track.resource_handle.is_some())
            || final_subtitles.iter().any(|sub| sub.resource_handle.is_some());
        let installed = if has_handles { Some(self.store.installed()?) } else { None };
        let mut new_tokens = Vec::new();
        let prepare: Result<()> = async {
            if let Some(handle) = stream.resource_handle.as_ref() {
                let session = self.worker.create_media_session(installed.as_ref().unwrap(), handle).await
                    .context("Could not start the provider's media session; reload the sources and try again")?;
                new_tokens.push(session.session_token.clone());
                if streams::is_playlist(&stream) {
                    streams::apply_playlist_transport(&mut final_stream, &session.playlist_urls)?;
                }
                final_stream.url = session.transport_url.clone();
                final_stream.transport_url = Some(session.transport_url);
                // The worker applies the main resource headers. Never pass them globally to mpv,
                // where they would otherwise leak onto subtitle and audio requests.
                final_stream.headers.clear();
                final_stream.referer.clear();
            }
            for audio in &mut final_stream.audio_tracks {
                if let Some(handle) = audio.resource_handle.as_ref() {
                    let session = self.worker.create_media_session(installed.as_ref().unwrap(), handle).await
                        .context("Could not start a separate audio session")?;
                    new_tokens.push(session.session_token);
                    audio.url = session.transport_url;
                    audio.headers.clear();
                }
            }
            for sub in &mut final_subtitles {
                if let Some(handle) = sub.resource_handle.as_ref() {
                    let session = self.worker.create_media_session(installed.as_ref().unwrap(), handle).await
                        .context("Could not start a subtitle session")?;
                    new_tokens.push(session.session_token);
                    sub.url = session.transport_url;
                    sub.headers.clear();
                }
            }
            if drm::requirement(&final_stream).is_drm() {
                // Read the start of the media through the same session the player will use, to
                // learn which keys it needs, then take keys from the source or its licence server.
                let mut media_headers = final_stream.headers.clone();
                if !final_stream.referer.is_empty() && !media_headers.keys().any(|k| k.eq_ignore_ascii_case("referer")) {
                    media_headers.insert("Referer".into(), final_stream.referer.clone());
                }
                let proxied = stream.resource_handle.is_some();
                let keys = drm::resolve_drm(
                    &final_stream,
                    |url, limit| async move { drm::fetch_head(&url, limit, &media_headers, proxied).await },
                    |url, body, headers| async move { drm::post_licence(&url, &body, &headers).await },
                ).await?;
                drm::attach_keys(&mut final_stream, &keys);
            }
            Ok(())
        }.await;
        if let Err(error) = prepare {
            if let Some(installed) = installed.as_ref() {
                for token in &new_tokens { let _ = self.worker.close_media_session(installed, token).await; }
            }
            return Err(error);
        }

        // Playback starts immediately; the player prepares copies asynchronously after load.
        let transforms = self.player.preferences().await.subtitle_transforms;
        let originals: Vec<_> = subtitles.iter().filter(|sub| !subtitles::is_archive_subtitle(sub)).collect();
        let mut cleanup_notices = Vec::new();
        let prepared: Vec<player::PreparedSubtitle> = originals.iter().zip(final_subtitles.iter()).map(|(source, resolved)| {
            let cleanup = if transforms.is_empty() { None } else if let Some(format) = subtitles::text_format(source) {
                Some((format.to_string(), resolved.resource_handle.is_some(), transforms.clone()))
            } else {
                cleanup_notices.push(format!("Subtitle cleanup is unavailable for {}. Using the original.", source.lang));
                None
            };
            player::PreparedSubtitle { subtitle: resolved.clone(), cleanup }
        }).collect();

        // Serialize replacement with terminal-event cleanup. The lock stays held until the new
        // player session owns its transport, so a delayed shutdown from the previous player can
        // never close the replacement's token.
        let mut active_transport = self.active_transport_token.lock().await;
        if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent {
            if let Some(installed) = installed.as_ref() {
                for token in &new_tokens { let _ = self.worker.close_media_session(installed, token).await; }
            }
            return Err(PlaybackSuperseded.into());
        }
        if let Some(old) = active_transport.take() {
            if let Ok(installed) = self.store.installed() {
                for token in &old.tokens { let _ = self.worker.close_media_session(&installed, token).await; }
            }
        }
        let start_pos = watch.as_ref().map(|w| w.position).unwrap_or(resume);
        let session_id = match self.player.play_with_prepared(
            &final_stream, &prepared, watch.clone(), start_pos, Some(profile_id_str), queue, carry,
        ).await {
            Ok(session_id) => session_id,
            Err(error) => {
                if let Some(installed) = installed.as_ref() {
                    for token in &new_tokens { let _ = self.worker.close_media_session(installed, token).await; }
                }
                return Err(error);
            }
        };
        self.active_playback_intent.store(intent, std::sync::atomic::Ordering::SeqCst);
        if !new_tokens.is_empty() {
            *active_transport = Some(ActiveTransport { tokens: new_tokens, player_session_id: session_id.clone() });
        }
        drop(active_transport);
        for notice in cleanup_notices { self.player.notice(&session_id, notice); }
        if let Some(watch) = watch {
            self.store.save_watch_entry(&watch, profile_id_str)?;
        }
        Ok(session_id)
    }

    /// Check subscribed items while the app is running. A failed or stale request never
    /// overwrites a successful snapshot; at most four provider requests run concurrently.
    pub async fn check_subscriptions(self: &Arc<Self>, profile_id: &str, force: bool) -> Result<subscriptions::SubscriptionCheckReport> {
        let _check = self.subscription_check.lock().await;
        if self.store.get_profile(profile_id)?.is_none() { bail!("Profile not found"); }
        let notify = self.store.get_settings()?.global.subscription_notifications;
        let now = timestamp_ms();
        let mut report = subscriptions::SubscriptionCheckReport::default();
        let mut jobs = Vec::new();
        for item in self.store.library(Some(profile_id))?.into_iter().filter(|item| item.is_subscribed) {
            if !self.store.provider_is_enabled(&item.item.provider_id)? { report.skipped += 1; continue; }
            let Some(generation) = self.store.ensure_subscription_generation(profile_id, &item.id)? else { continue };
            if !force {
                let key = format!("{profile_id}::{}", item.id);
                let old = self.store.get::<subscriptions::SubscriptionSnapshot>("subscription_snapshot", &key)?;
                if !subscriptions::check_is_due(old.as_ref(), &generation, now) {
                    report.skipped += 1;
                    continue;
                }
            }
            jobs.push((item, generation));
        }
        let mut queue = jobs.into_iter();
        let mut running = tokio::task::JoinSet::new();
        loop {
            while running.len() < 4 {
                let Some((item, generation)) = queue.next() else { break };
                let app = self.clone();
                let profile = profile_id.to_owned();
                running.spawn(async move {
                    let outcome: Result<Option<subscriptions::SubscriptionChanges>> = async {
                    let cached = if force { None } else { app.cached_subscription_details(&profile, &item.item.provider_id, &item.item.url) };
                    let details = if let Some(details) = cached { details } else {
                    let request_id = format!("subscription-{}", uuid::Uuid::new_v4());
                    let request = app.load_details(&profile, &item.item.provider_id, &item.item.url, Some(&request_id));
                    tokio::pin!(request);
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
                    let mut poll = tokio::time::interval(std::time::Duration::from_millis(250));
                    let details = loop {
                        tokio::select! {
                            result = &mut request => break result?,
                            _ = tokio::time::sleep_until(deadline) => {
                                let _ = app.cancel_request(&request_id).await;
                                bail!("Provider check timed out");
                            },
                            _ = poll.tick() => {
                                if !app.store.subscription_is_current(&profile, &item.id, &generation)? {
                                    let _ = app.cancel_request(&request_id).await;
                                    return Ok(None);
                                }
                            },
                        }
                    };
                    details
                    };
                    let observed = subscriptions::episode_snapshot(&details)?;
                    let changes = app.store.apply_subscription_snapshot(&profile, &item.id, &generation, observed, timestamp_ms(), notify)?;
                    Ok(changes)
                    }.await;
                    (item, generation, outcome)
                });
            }
            let Some(result) = running.join_next().await else { break };
            match result {
                Ok((item, generation, outcome)) => {
                    let changes = match outcome {
                        Ok(Some(changes)) => { report.checked += 1; Some(changes) },
                        Ok(None) => { report.skipped += 1; None },
                        Err(_) => { report.failed += 1; None },
                    };
                    let episodes = if force { self.store.subscription_unread(profile_id, &item.id, &generation)? }
                        else { changes.map(|c| c.new_episodes.into_iter().map(|(group, items)| (group, items.into_iter().collect())).collect()).unwrap_or_default() };
                    let count = episodes.values().map(std::collections::BTreeSet::len).sum();
                    if count > 0 { report.updates.push(subscriptions::SubscriptionUpdate { media_id: item.id, title: item.item.name, episode_count: count, generation, episodes }); }
                }
                Err(_) => report.failed += 1,
            }
        }
        report.updates.sort_by(|a, b| a.media_id.cmp(&b.media_id));
        Ok(report)
    }

    /// Deliver claimed batches serially. The sink is injected so notification behavior can be
    /// tested without a desktop session. Failure restores the batch for a later attempt.
    pub async fn deliver_subscription_alerts<F, Fut>(&self, profile_id: &str, mut sink: F) -> Result<subscriptions::AlertDeliveryReport>
    where F: FnMut(subscriptions::SubscriptionAlert) -> Fut, Fut: std::future::Future<Output = Result<()>> {
        let _delivery = self.subscription_delivery.lock().await;
        let mut report = subscriptions::AlertDeliveryReport::default();
        for item in self.store.library(Some(profile_id))?.into_iter().filter(|item| item.is_subscribed) {
            let Some(alert) = self.store.claim_subscription_alert(profile_id, &item.id)? else { continue };
            if sink(alert.clone()).await.is_ok() { report.delivered += 1; }
            else { self.store.restore_subscription_alert(&alert)?; report.failed += 1; }
        }
        Ok(report)
    }

    /// Close the worker media session behind the current transport, unless a newer playback has
    /// already replaced `session_id` (a mirror switch or failover must keep its new transport).
    pub async fn close_transport_for(&self, session_id: &str) {
        let mut guard = self.active_transport_token.lock().await;
        if guard.as_ref().is_none_or(|active| active.player_session_id != session_id) {
            return;
        }
        if let Some(active) = guard.take() {
            if let Ok(installed) = self.store.installed() {
                for token in &active.tokens { let _ = self.worker.close_media_session(&installed, token).await; }
            }
        }
    }

    /// Accept only a path obtained from the native file chooser. ZIP contents remain in memory
    /// until the user chooses an entry, and never expose a filesystem path to the web view.
    pub async fn prepare_local_subtitle(&self, path: &std::path::Path, session_id: &str) -> Result<SubtitleFileSelection> {
        *self.pending_subtitle_archive.lock().await = None;
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id) {
            bail!("Playback changed while choosing the subtitle. Choose the file again");
        }
        let is_zip = path.extension().and_then(|e| e.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if !is_zip {
            self.player.add_local_subtitle(path, session_id).await?;
            return Ok(SubtitleFileSelection::Added);
        }
        use std::io::Read;
        let file = std::fs::File::open(path).context("Could not open the subtitle archive")?;
        if !file.metadata()?.is_file() { bail!("Choose a subtitle archive file"); }
        let mut bytes = Vec::new();
        file.take(subtitles::MAX_SUBTITLE_BYTES + 1).read_to_end(&mut bytes).context("Could not read the subtitle archive")?;
        let entries = subtitles::list_archive(&bytes)?;
        let id = uuid::Uuid::new_v4().to_string();
        *self.pending_subtitle_archive.lock().await = Some(PendingSubtitleArchive { id: id.clone(), session_id: session_id.into(), bytes, created: std::time::Instant::now() });
        Ok(SubtitleFileSelection::Archive { id, entries })
    }

    /// Download a provider archive through its resource session, with the same size limits as
    /// a local archive. Only subtitles belonging to the current playback context are accepted.
    pub async fn prepare_provider_subtitle_archive(&self, session_id: &str, index: usize) -> Result<SubtitleFileSelection> {
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id) { bail!("Playback changed. Choose the subtitle again"); }
        let subtitle = self.playback_context.lock().await.as_ref()
            .filter(|context| context.matches_player(&state))
            .and_then(|context| context.subtitles.get(index).cloned())
            .context("That provider subtitle is no longer available")?;
        if !subtitles::is_archive_subtitle(&subtitle) { bail!("That provider subtitle is not a ZIP archive"); }
        let installed = if subtitle.resource_handle.is_some() { Some(self.store.installed()?) } else { None };
        let session = match subtitle.resource_handle.as_deref() {
            Some(handle) => Some(self.worker.create_media_session(installed.as_ref().unwrap(), handle).await.context("Could not start the subtitle's provider session")?),
            None if !subtitle.headers.is_empty() => bail!("This protected subtitle requires a provider session. Reload the sources"),
            None => None,
        };
        let url = session.as_ref().map(|session| session.transport_url.as_str()).unwrap_or(&subtitle.url);
        let fetched: Result<Vec<u8>> = async {
            repository::network_url(url)?;
            let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).redirect(reqwest::redirect::Policy::limited(5));
            if session.is_some() { builder = builder.no_proxy(); }
            let mut response = builder.build()?.get(url).send().await.context("Could not download the subtitle archive")?
                .error_for_status().context("The provider rejected the subtitle archive request")?;
            if response.content_length().is_some_and(|size| size > subtitles::MAX_SUBTITLE_BYTES) { bail!("Subtitle archives must be 20 MiB or smaller"); }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.context("Could not read the subtitle archive")? {
                if bytes.len().saturating_add(chunk.len()) > subtitles::MAX_SUBTITLE_BYTES as usize { bail!("Subtitle archives must be 20 MiB or smaller"); }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        }.await;
        if let (Some(session), Some(installed)) = (&session, &installed) { let _ = self.worker.close_media_session(installed, &session.session_token).await; }
        let bytes = fetched?;
        let entries = subtitles::list_archive(&bytes)?;
        let current = self.player.state().await;
        if !current.active || current.session_id.as_deref() != Some(session_id) { bail!("Playback changed while downloading subtitles. Choose the archive again"); }
        let id = uuid::Uuid::new_v4().to_string();
        *self.pending_subtitle_archive.lock().await = Some(PendingSubtitleArchive { id: id.clone(), session_id: session_id.into(), bytes, created: std::time::Instant::now() });
        Ok(SubtitleFileSelection::Archive { id, entries })
    }

    pub async fn select_archived_subtitle(&self, id: &str, index: usize) -> Result<()> {
        let pending = {
            let mut guard = self.pending_subtitle_archive.lock().await;
            if guard.as_ref().is_none_or(|entry| entry.id != id) { bail!("That subtitle archive is no longer available. Choose it again"); }
            guard.take().unwrap()
        };
        if pending.created.elapsed() > std::time::Duration::from_secs(600) { bail!("Subtitle selection expired. Choose the archive again"); }
        self.player.add_archived_subtitle(&pending.bytes, index, &pending.session_id).await
    }

    pub async fn cancel_archived_subtitle(&self, id: &str) {
        let mut guard = self.pending_subtitle_archive.lock().await;
        if guard.as_ref().is_some_and(|entry| entry.id == id) { guard.take(); }
    }

    pub fn import_subtitle_font(&self, path: &std::path::Path) -> Result<fonts::ImportedFont> {
        fonts::import_font(&self.data.join("subtitle-fonts"), path)
    }

    pub fn subtitle_font_preview(&self, id: &str) -> Result<String> {
        use base64::Engine;
        let bytes = fonts::read_font(&self.data.join("subtitle-fonts"), id)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// The active quality/source profile for a local profile (Android's default priorities when
    /// the user has not customised anything).
    pub fn source_profile(&self, profile_id: &str) -> SourceProfile {
        self.store
            .get_settings()
            .ok()
            .and_then(|s| {
                s.profiles
                    .get(profile_id)
                    .and_then(|p| p.source_priority.as_ref().map(SourcePriorityConfig::active))
            })
            .unwrap_or_else(SourceProfile::default_profile)
    }

    /// Rank a freshly resolved episode without inheriting failures from earlier playback.
    pub async fn rank_streams(&self, streams: &[Stream], profile_id: Option<&str>) -> Vec<RankedStream> {
        let profile = self.source_profile(profile_id.unwrap_or("default"));
        streams::rank_streams(streams, &profile, &Default::default())
    }

    /// Include failed mirrors only when the caller explicitly identifies the active session.
    pub async fn rank_streams_for_session(&self, streams: &[Stream], profile_id: Option<&str>, session_id: Option<&str>) -> Vec<RankedStream> {
        let profile_id = profile_id.unwrap_or("default");
        let profile = self.source_profile(profile_id);
        let state = self.player.state().await;
        let same_session = state.active && session_id.is_some() && state.session_id.as_deref() == session_id
            && state.profile_id.as_deref().unwrap_or("default") == profile_id;
        let errored = if same_session {
            self.playback_context.lock().await.as_ref().filter(|c| c.matches_player(&state)).map(|c| c.errored.clone()).unwrap_or_default()
        } else { Default::default() };
        streams::rank_streams(streams, &profile, &errored)
    }

    /// Cancel pending starts before stopping the active player. The transport lock serializes
    /// the final stop against a start that has already passed its intent check.
    pub async fn stop_playback(&self) {
        let intent = self.playback_intent.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let mut active = self.active_transport_token.lock().await;
        if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent { return; }
        let _ = self.save_progress().await;
        self.player.stop().await;
        self.active_playback_intent.store(0, std::sync::atomic::Ordering::SeqCst);
        let mut context = self.playback_context.lock().await;
        if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) == intent { *context = None; }
        drop(context);
        if let Some(old) = active.take() {
            if let Ok(installed) = self.store.installed() {
                for token in old.tokens { let _ = self.worker.close_media_session(&installed, &token).await; }
            }
        }
    }

    /// Run a typed player command, persisting preferences it changed.
    pub async fn player_command(&self, command: PlayerCommand) -> Result<PlayerState> {
        if let Some(prefs) = self.player.execute(command).await? {
            self.store.update_settings(json!({ "player": prefs }), Some("global"), None)?;
        }
        Ok(self.player.state().await)
    }

    /// Fetch markers only for the current episode's explicitly linked MAL ID. A response from
    /// an old session or after the person disables the service is discarded.
    pub async fn player_skip_markers(&self, session_id: &str) -> Result<Vec<skip::SkipMarker>> {
        let settings = self.store.get_settings()?.global.skip;
        if !settings.aniskip { return Ok(Vec::new()); }
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id) { bail!("Playback changed; skip times were not applied"); }
        let Some((watch, profile)) = self.player.current_watch().await else { return Ok(Vec::new()) };
        let Some(media_id) = watch.item.media_id.as_deref() else { return Ok(Vec::new()) };
        if state.media_id.as_deref() != Some(media_id) || state.episode_id.as_deref() != Some(watch.id.as_str()) || state.profile_id.as_deref() != Some(profile.as_str()) { return Ok(Vec::new()); }
        let supplied = self.cached_subscription_details(&profile, &watch.item.provider_id, &watch.item.url).map(|details| details.sync_data).unwrap_or_default();
        let links = self.metadata_links(media_id, &supplied)?;
        let Some(expected_mal) = skip::matching_mal_id(media_id, &links) else { return Ok(Vec::new()) };
        let transport = self.skip_transport.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let client = skip::Client::new(match transport { Some(transport) => transport, None => Arc::new(skip::Reqwest::new()?) });
        let markers = client.lookup(&settings, media_id, &links, &watch.episode, state.duration).await?;
        let current = self.player.state().await;
        if !self.store.get_settings()?.global.skip.aniskip || !current.active || current.session_id.as_deref() != Some(session_id)
            || current.episode_id != state.episode_id || (current.duration - state.duration).abs() > 1.0 { return Ok(Vec::new()); }
        let latest = self.cached_subscription_details(&profile, &watch.item.provider_id, &watch.item.url).map(|details| details.sync_data).unwrap_or_default();
        if skip::matching_mal_id(media_id, &self.metadata_links(media_id, &latest)?) != Some(expected_mal) { return Ok(Vec::new()); }
        Ok(markers)
    }

    pub fn set_skip_transport(&self, transport: Arc<dyn skip::Transport>) {
        *self.skip_transport.lock().unwrap_or_else(|e| e.into_inner()) = Some(transport);
    }

    /// Re-apply saved settings to the player after the user changed them.
    pub async fn apply_player_settings(&self, settings: &Settings) {
        self.player.set_policy(settings.global.playback.clone()).await;
        self.player.set_preferences(settings.global.player.clone()).await;
    }

    /// Switch the episode that is playing to another mirror at the same position, keeping the
    /// chosen audio/subtitle languages where the new stream offers them.
    pub async fn switch_stream(&self, stream: Stream, subtitles: Option<Vec<Subtitle>>) -> Result<String> {
        let intent = self.playback_intent.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let state = self.player.state().await;
        if !state.active {
            bail!("Nothing is playing, so there is no source to switch");
        }
        let (item, episode, profile_id) = match self.player.current_watch().await {
            Some((watch, profile)) => (Some(watch.item), Some(watch.episode), profile),
            None => (None, None, state.profile_id.clone().unwrap_or_else(|| "default".into())),
        };
        let carry = self.player.track_choice().await;
        let queue = self.player.queue().await;
        let key = streams::stream_key(&stream);
        let subtitles = {
            let mut guard = self.playback_context.lock().await;
            if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent { return Err(PlaybackSuperseded.into()); }
            let context = guard.get_or_insert_with(PlaybackContext::default);
            if !context.mirrors.iter().any(|m| streams::stream_key(m) == key) {
                context.mirrors.push(stream.clone());
            }
            if let Some(given) = subtitles {
                context.subtitles = given;
            }
            context.current_key = key;
            context.attempts = 0;
            context.subtitles.clone()
        };
        let quality_label = streams::quality_label(stream.quality);
        let name = stream.name.clone();
        let quality = stream.quality;
        let session_id = self
            .start_stream(stream, subtitles, item, episode, state.position, Some(&profile_id), queue, &carry, intent)
            .await?;
        self.player.broadcast(PlayerEvent::SourceChanged {
            session_id: session_id.clone(),
            name,
            quality,
            quality_label,
            reason: "manual".into(),
            attempt: 0,
        });
        Ok(session_id)
    }

    /// After a load failure or a dropped stream, try the next best mirror at the position where
    /// playback stopped. Bounded by the `failoverAttempts` playback setting. Returns `true` when
    /// another stream was started; otherwise the user has already been told why playback ended.
    pub async fn failover_playback(&self, failed_session: &str, detail: Option<&str>) -> Result<bool> {
        // Wait for any in-flight player start to finish establishing session ownership before
        // deciding whether an immediately emitted failure still belongs to this intent.
        let transport_guard = self.active_transport_token.lock().await;
        let intent = self.playback_intent.load(std::sync::atomic::Ordering::SeqCst);
        if self.active_playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent { return Ok(false); }
        let state = self.player.state().await;
        if state.session_id.as_deref() != Some(failed_session) {
            return Ok(false);
        }
        drop(transport_guard);
        if self.playback_context.lock().await.as_ref().is_none_or(|context| !context.matches_player(&state)) {
            return Ok(false);
        }
        let policy = self.player.policy().await;
        let reason = detail.unwrap_or("it could not be loaded");
        let profile_id = state.profile_id.clone().unwrap_or_else(|| "default".into());
        let profile = self.source_profile(&profile_id);

        let next = {
            let mut guard = self.playback_context.lock().await;
            if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent { return Ok(false); }
            match guard.as_mut() {
                None => None,
                Some(context) => {
                    context.errored.insert(context.current_key.clone());
                    if context.attempts >= policy.failover_attempts {
                        None
                    } else {
                        let ranked = streams::rank_streams(&context.mirrors, &profile, &context.errored);
                        ranked.iter().find(|r| r.usable && !r.errored).map(|r| {
                            context.attempts += 1;
                            context.current_key = r.key.clone();
                            (context.mirrors[r.index].clone(), context.subtitles.clone(), context.attempts)
                        })
                    }
                }
            }
        };

        let Some((stream, subtitles, attempt)) = next else {
            let name = state.stream.as_ref().map(|s| s.name.as_str()).unwrap_or("This source");
            let tried = self
                .playback_context
                .lock()
                .await
                .as_ref()
                .map(|c| c.errored.len())
                .unwrap_or(1);
            let message = if policy.failover_attempts == 0 || tried <= 1 {
                format!("{name} failed to play: {reason}. Choose another source.")
            } else {
                format!("{tried} sources failed to play and no other source is available. Last error: {reason}.")
            };
            let _transport_guard = self.active_transport_token.lock().await;
            if self.playback_intent.load(std::sync::atomic::Ordering::SeqCst) != intent
                || self.player.state().await.session_id.as_deref() != Some(failed_session) {
                return Ok(false);
            }
            self.player.broadcast_error(failed_session.to_string(), message);
            self.player.stop().await;
            self.active_playback_intent.store(0, std::sync::atomic::Ordering::SeqCst);
            return Ok(false);
        };

        let (item, episode) = match self.player.current_watch().await {
            Some((watch, _)) => (Some(watch.item), Some(watch.episode)),
            None => (None, None),
        };
        let carry = self.player.track_choice().await;
        let queue = self.player.queue().await;
        let name = stream.name.clone();
        let quality = stream.quality;
        let quality_label = streams::quality_label(quality);
        match self
            .start_stream(stream, subtitles, item, episode, state.position, Some(&profile_id), queue, &carry, intent)
            .await
        {
            Ok(session_id) => {
                self.player.broadcast(PlayerEvent::SourceChanged {
                    session_id,
                    name,
                    quality,
                    quality_label,
                    reason: "failover".into(),
                    attempt,
                });
                Ok(true)
            }
            Err(error) => {
                if error.downcast_ref::<PlaybackSuperseded>().is_some() { return Ok(false); }
                self.player.broadcast_error(failed_session.to_string(), format!("Could not switch to another source: {error}"));
                Ok(false)
            }
        }
    }

    pub async fn set_playback_queue(&self, queue: Option<PlaybackQueue>) -> Result<()> {
        self.player.set_queue(queue).await;
        Ok(())
    }

    pub async fn get_webview_status(&self) -> Result<WebViewReport> {
        let installed = self.store.installed()?;
        self.worker.get_webview_status(&installed).await
    }

    pub async fn advance_queue_and_play(&self, forward: bool) -> Result<bool> {
        let queue_opt = self.player.queue().await;
        let Some(mut queue) = queue_opt else {
            return Ok(false);
        };

        let target_index = if forward {
            if queue.current_index + 1 >= queue.episodes.len() {
                return Ok(false);
            }
            queue.current_index + 1
        } else {
            if queue.current_index == 0 {
                return Ok(false);
            }
            queue.current_index - 1
        };

        if let Ok(providers) = self.providers().await {
            if let Some(p) = providers.iter().find(|p| p.id == queue.media.provider_id) {
                if p.disabled {
                    self.player.broadcast_error("".into(), format!("Provider '{}' is disabled", p.name));
                    return Ok(false);
                }
            }
        }

        let next_episode = queue.episodes[target_index].clone();
        queue.current_index = target_index;

        let links_res = self.call("loadLinks", serde_json::json!({
            "providerId": queue.media.provider_id,
            "data": next_episode.data,
        })).await;

        let links: ResolvedLinks = match links_res {
            Ok(val) => match serde_json::from_value(val) {
                Ok(l) => l,
                Err(e) => {
                    self.player.broadcast_error("".into(), format!("Could not parse links: {e}"));
                    return Ok(false);
                }
            },
            Err(e) => {
                self.player.broadcast_error("".into(), format!("Failed to resolve stream for next episode: {e}"));
                return Ok(false);
            }
        };

        if links.streams.is_empty() {
            self.player.broadcast_error("".into(), "No playable streams returned for next episode. Choose a mirror or retry.".into());
            return Ok(false);
        }

        let profile_id = queue.profile_id.clone();
        let ranked = self.rank_streams(&links.streams, Some(&profile_id)).await;
        let Some(best_index) = streams::best_stream_index(&ranked) else {
            self.player.broadcast_error("".into(), "No streams match your source preferences. Adjust the profile or choose another episode.".into());
            return Ok(false);
        };
        let best_stream = links.streams[best_index].clone();

        let _ = self.play_with_mirrors(
            best_stream,
            links.streams.clone(),
            links.subtitles,
            Some(queue.media.clone()),
            Some(next_episode),
            0.0,
            Some(&profile_id),
            Some(queue),
        ).await?;

        Ok(true)
    }

    pub async fn flush_player_progress(&self) -> Result<()> {
        if let Some((watch, profile_id)) = self.player.current_watch().await {
            self.store.save_watch_entry(&watch, &profile_id)?;
        }
        Ok(())
    }

    pub async fn save_progress(&self) -> Result<()> {
        self.flush_player_progress().await
    }

    // ---- Metadata matching and enrichment (T24) -----------------------------------------

    /// Replace the network layer (tests use recorded responses).
    pub fn set_metadata_transport(&self, transport: Arc<dyn metadata::Transport>) {
        *self.metadata_transport.lock().unwrap_or_else(|e| e.into_inner()) = Some(transport);
    }

    fn metadata_client(&self) -> Result<metadata::Client> {
        let settings = self.store.get_settings()?.global.metadata;
        let transport = {
            let mut guard = self.metadata_transport.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(t) => t.clone(),
                None => {
                    let t: Arc<dyn metadata::Transport> = Arc::new(metadata::Reqwest::new()?);
                    *guard = Some(t.clone());
                    t
                }
            }
        };
        Ok(metadata::Client::new(transport, settings))
    }

    fn stored_links(&self, media_id: &str) -> Result<Vec<metadata::MetadataLink>> {
        Ok(self.store.list::<metadata::MetadataLink>("metadata_link")?.into_iter().filter(|l| l.media_id == media_id).collect())
    }

    fn blocked_links(&self, media_id: &str) -> Result<Vec<(String, String)>> {
        Ok(self.store.list::<Value>("metadata_blocked")?.into_iter().filter(|v| v["mediaId"] == media_id).filter_map(|v| Some((v["service"].as_str()?.to_string(), v["externalId"].as_str()?.to_string()))).collect())
    }

    /// The metadata links that apply to a title: what its source supplied, overridden by the user.
    pub fn metadata_links(&self, media_id: &str, sync_data: &std::collections::BTreeMap<String, String>) -> Result<Vec<metadata::MetadataLink>> {
        Ok(metadata::effective_links(media_id, &metadata::external_ids(sync_data), &self.stored_links(media_id)?, &self.blocked_links(media_id)?, timestamp_ms()))
    }

    /// Search the enabled services for suggestions. Nothing is linked by this call.
    pub async fn metadata_search(&self, title: &str, year: Option<i32>) -> Result<(Vec<metadata::Candidate>, Vec<String>)> {
        self.metadata_client()?.search(title, year).await
    }

    /// Record the person's choice of a service match for a title.
    pub fn set_metadata_link(&self, media_id: &str, service: &str, external_id: &str, label: Option<String>, year: Option<i32>) -> Result<metadata::MetadataLink> {
        if !metadata::SERVICES.contains(&service) {
            bail!("Unknown metadata service");
        }
        let id = external_id.trim();
        // Reuse the validators through the public id parser.
        let probe = std::collections::BTreeMap::from([(service.to_string(), id.to_string())]);
        if metadata::external_ids(&probe).is_empty() {
            bail!("That is not a valid {service} id");
        }
        let link = metadata::MetadataLink {
            media_id: media_id.into(),
            service: service.into(),
            external_id: id.into(),
            source: metadata::MatchSource::User,
            linked_at_ms: timestamp_ms(),
            label: label.map(|l| l.chars().take(200).collect()),
            year,
        };
        self.store.put("metadata_link", &format!("{media_id}::{service}"), &link)?;
        // Choosing a match again also lifts an earlier "not this one" for the same id.
        let _ = self.store.remove("metadata_blocked", &format!("{media_id}::{service}::{id}"));
        Ok(link)
    }

    /// Unlink a service from a title. If the source itself supplies the id, that id is remembered
    /// as rejected so it does not come back the next time the title is opened.
    pub fn remove_metadata_link(&self, media_id: &str, service: &str, supplied: &std::collections::BTreeMap<String, String>) -> Result<()> {
        let _ = self.store.remove("metadata_link", &format!("{media_id}::{service}"));
        if let Some(id) = metadata::external_ids(supplied).get(service) {
            self.store.put("metadata_blocked", &format!("{media_id}::{service}::{id}"), &json!({ "mediaId": media_id, "service": service, "externalId": id }))?;
        }
        Ok(())
    }

    /// Enrichment for an open title. A failing or disabled service never affects the provider's
    /// own data: the result then simply has less in it, with the reasons listed.
    pub async fn metadata_for(&self, media_id: &str, details: &MediaDetails, locale: &str) -> Result<MetadataResult> {
        let settings = self.store.get_settings()?.global.metadata;
        let links = self.metadata_links(media_id, &details.sync_data)?;
        let mut result = MetadataResult { links: links.clone(), ..Default::default() };
        if !settings.enabled || links.is_empty() {
            result.view = metadata::merge(details, &[], &settings);
            return Ok(result);
        }
        let now = timestamp_ms();
        let mal = links.iter().find(|l| l.service == "mal").map(|l| l.external_id.clone());
        let client = self.metadata_client()?;
        let mut enrichments = Vec::new();
        for link in links.iter().filter(|l| l.service == "anilist" || l.service == "kitsu") {
            let cache_id = format!("{}::{}::{locale}", link.service, link.external_id);
            let cached: Option<metadata::Enrichment> = self.store.get("metadata_cache", &cache_id).ok().flatten();
            if let Some(entry) = cached.as_ref().filter(|e| now.saturating_sub(e.fetched_at_ms) < metadata::CACHE_TTL_MS) {
                enrichments.push(entry.clone());
                continue;
            }
            match client.details(&link.service, &link.external_id, mal.as_deref(), locale, now).await {
                Ok(fresh) => {
                    let _ = self.store.put("metadata_cache", &cache_id, &fresh);
                    enrichments.push(fresh);
                }
                Err(error) => {
                    self.diagnostics.record("metadata", &format!("{} lookup failed: {error:#}", link.service));
                    result.errors.push(format!("{}: {error:#}", link.service));
                    // An outage falls back to what was fetched earlier, marked as out of date.
                    if let Some(old) = cached {
                        result.stale = true;
                        enrichments.push(old);
                    }
                }
            }
        }
        result.view = metadata::merge(details, &enrichments, &settings);
        Ok(result)
    }

    // ---- Diagnostics (T32) ---------------------------------------------------------------

    /// Gather a report from structured facts plus the redacted event rings.
    pub async fn diagnostics_report(&self) -> Result<diagnostics::DiagnosticsReport> {
        let status = self.status().await;
        let caps = self.player.capabilities().await;
        let worker_jar = self.resources.join("worker/worker.jar");
        let worker_jar_sha256 = std::fs::metadata(&worker_jar)
            .ok()
            .filter(|m| m.is_file() && m.len() < 256 * 1024 * 1024)
            .and_then(|_| std::fs::read(&worker_jar).ok())
            .map(|bytes| repository::digest(&bytes));
        let mut packages: Vec<diagnostics::PackageInfo> = self.store.installed()?.iter().take(500).map(diagnostics::package_info).collect();
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        let mut notes = Vec::new();
        let providers = match tokio::time::timeout(std::time::Duration::from_secs(5), self.providers()).await {
            Ok(Ok(list)) => list,
            Ok(Err(error)) => {
                notes.push(format!("Sources could not be listed: {}", diagnostics::redact(&format!("{error:#}"))));
                Vec::new()
            }
            Err(_) => {
                notes.push("Listing sources took too long and was skipped.".into());
                Vec::new()
            }
        };
        Ok(diagnostics::DiagnosticsReport {
            generated_at_ms: timestamp_ms(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            java_available: status.jvm_available,
            worker_available: status.worker_available,
            worker_jar_sha256,
            mpv_version: caps.version.clone(),
            player_unsupported: caps.capabilities.iter().filter(|c| !c.supported).map(|c| format!("{}: {}", c.label, c.reason.clone().unwrap_or_default())).collect(),
            packages,
            providers: providers.len(),
            disabled_providers: providers.iter().filter(|p| p.disabled).count(),
            events: self.diagnostics.events(),
            timings: self.diagnostics.timings(),
            notes,
        })
    }

    /// Look for a newer, signed release. Nothing is downloaded or installed.
    pub async fn check_for_app_update(&self) -> Result<updates::UpdateCheck> {
        let settings = self.store.get_settings()?.global.updates;
        let executable = std::env::current_exe().unwrap_or_default();
        let result = updates::check(&settings, updates::configured_public_key(), env!("CARGO_PKG_VERSION"), &executable, |url| async move { updates::fetch_feed(&url).await }).await;
        match &result {
            Ok(updates::UpdateCheck::Available { release, .. }) => *self.last_release_notes.lock().unwrap_or_else(|e| e.into_inner()) = Some(release.notes_url.clone()),
            Ok(_) => *self.last_release_notes.lock().unwrap_or_else(|e| e.into_inner()) = None,
            Err(error) => self.diagnostics.record("updates", &format!("{error:#}")),
        }
        result
    }

    // ---- Entry points (T27) --------------------------------------------------------------

    /// Accept links/files from outside. Valid ones wait in the queue; refused ones are kept as
    /// messages for the user. Returns how many actions were queued.
    pub fn submit_entry_arguments(&self, arguments: &[String]) -> usize {
        let parsed = entry::parse_arguments(arguments);
        let queued = parsed.actions.into_iter().filter(|action| self.entries.push(action.clone())).count();
        if !parsed.errors.is_empty() {
            let mut errors = self.entry_errors.lock().unwrap_or_else(|e| e.into_inner());
            for error in parsed.errors {
                if errors.len() < 10 && !errors.contains(&error) {
                    errors.push(error);
                }
            }
        }
        queued
    }

    /// Hand the waiting actions and messages to the interface, once.
    pub fn take_entries(&self) -> (Vec<entry::EntryAction>, Vec<String>) {
        let errors = std::mem::take(&mut *self.entry_errors.lock().unwrap_or_else(|e| e.into_inner()));
        (self.entries.take(), errors)
    }

    /// Carry out a control that arrived over MPRIS. Everything goes through the same validated
    /// player commands as the interface; nothing here can start playback of a new address.
    pub async fn handle_mpris(&self, request: mpris::MprisRequest) -> Result<()> {
        use mpris::MprisRequest as R;
        match request {
            R::Play => self.player_command(PlayerCommand::Play).await.map(drop),
            R::Pause => self.player_command(PlayerCommand::Pause).await.map(drop),
            R::PlayPause => self.player_command(PlayerCommand::TogglePause).await.map(drop),
            R::Stop => {
                let _ = self.save_progress().await;
                self.player.stop().await;
                Ok(())
            }
            R::Next => self.advance_queue_and_play(true).await.map(drop),
            R::Previous => self.advance_queue_and_play(false).await.map(drop),
            R::Seek(offset_us) => self.player_command(PlayerCommand::SeekRelative { seconds: offset_us as f64 / 1_000_000.0 }).await.map(drop),
            R::SetPosition { track_id, position_us } => {
                let state = self.player.state().await;
                // A position for a track that is no longer playing must not move the current one.
                if state.session_id.as_deref().map(mpris::track_id_for).as_deref() != Some(track_id.as_str()) {
                    return Ok(());
                }
                self.player_command(PlayerCommand::SeekAbsolute { seconds: (position_us.max(0)) as f64 / 1_000_000.0 }).await.map(drop)
            }
            R::SetVolume(volume) => self.player_command(PlayerCommand::SetVolume { volume: (volume * 100.0).clamp(0.0, 100.0) }).await.map(drop),
            R::SetRate(rate) => self.player_command(PlayerCommand::SetSpeed { speed: rate }).await.map(drop),
            // Focusing the window belongs to the shell, which owns the window.
            R::Raise => Ok(()),
            R::OpenUri(uri) => {
                self.submit_entry_arguments(&[uri]);
                Ok(())
            }
        }
    }

    /// Play a media file from disk after the user confirmed it.
    pub async fn play_local_file(&self, path: &str, profile_id: Option<&str>) -> Result<String> {
        let _ = self.save_progress().await;
        *self.playback_context.lock().await = None;
        let previous = self.active_transport_token.lock().await.take();
        if let (Some(old), Ok(installed)) = (previous, self.store.installed()) {
            for token in &old.tokens {
                let _ = self.worker.close_media_session(&installed, token).await;
            }
        }
        self.player.play_local_file(std::path::Path::new(path), 0.0, profile_id).await
    }

    /// Play only a finished, still-owned download belonging to the active profile.
    pub async fn play_download(&self, id: &str, profile_id: &str) -> Result<String> {
        let job = self.store.get_download_job(id)?.filter(|job| job.profile_id == profile_id)
            .context("Download not found")?;
        if job.state != download_queue::DownloadState::Completed { bail!("This download is not finished"); }
        let verified = job.clone();
        let healthy = tokio::task::spawn_blocking(move || -> Result<bool> {
            if verified.extension == downloads::HLS_EXTENSION {
                Ok(hls_download::check_folder(&verified.destination, verified.destination_identity, &verified.id, true)?.healthy())
            } else {
                downloads::verify_completed_file(&verified.destination, verified.destination_identity, &verified.id, &verified.extension, verified.bytes)
            }
        }).await.context("Could not check the downloaded files")??;
        if !healthy { bail!("The downloaded files are missing or have changed. Remove this download and download it again."); }
        let output = job.output.as_ref().context("The downloaded files are unavailable")?;
        if output != &job.destination.join(format!("cloudstream-{}.{}", job.id, job.extension)) { bail!("The downloaded files are unavailable"); }
        let _ = self.save_progress().await;
        let resume = self.store.get_watch_progress(profile_id, &job.episode_id)?
            .map(|progress| progress.position_seconds).filter(|position| position.is_finite() && *position > 0.0).unwrap_or(0.0);
        let metadata = self.store.get::<download_queue::DownloadMetadata>("download_metadata", &job.id)?
            .or_else(|| self.store.history(Some(profile_id)).ok()?.into_iter().find(|entry| entry.id == job.episode_id)
                .map(|entry| download_queue::DownloadMetadata { item: entry.item, episode: entry.episode }));
        let watch = if let Some(mut metadata) = metadata {
            metadata.episode.data = self.store.get_episode_payload(&job.episode_id)?.unwrap_or_default();
            Some(WatchEntry { id: job.episode_id.clone(), item: metadata.item, episode: metadata.episode, position: resume, duration: 0.0, watched_at: timestamp_ms() })
        } else { None };
        *self.playback_context.lock().await = None;
        let previous = self.active_transport_token.lock().await.take();
        if let (Some(old), Ok(installed)) = (previous, self.store.installed()) {
            for token in &old.tokens { let _ = self.worker.close_media_session(&installed, token).await; }
        }
        if job.extension == downloads::HLS_EXTENSION {
            let manifest = hls_download::read_manifest(&job.destination, job.destination_identity, &job.id)?.context("The downloaded files are unavailable")?;
            let mut subtitles = Vec::new();
            for track in manifest.external_subtitles {
                if !track.path.starts_with("external/") || !hls_download::safe_relative(&track.path)
                    || !manifest.files.iter().any(|file| file.path == track.path) { bail!("The downloaded subtitle list is invalid"); }
                let path = output.join(&track.path);
                subtitles.push(Subtitle { lang: track.name, lang_tag: track.language, url: path.to_str().context("The downloaded subtitle path is invalid")?.to_owned(), ..Default::default() });
            }
            self.player.play_offline_hls(&output.join(hls_download::MASTER_NAME), &job.title, &subtitles, watch, resume, Some(profile_id)).await
        } else {
            let subtitles = direct_subtitles::read(&job.destination, job.destination_identity, &job.id, &job.extension)
                .context("The downloaded subtitles are unavailable")?.map(|(tracks, _)| tracks).unwrap_or_default();
            self.player.play_offline_file(output, &subtitles, watch, resume, Some(profile_id)).await
        }
    }

    // ---- Backup and restore (T25) -------------------------------------------------------

    /// Write a backup file atomically and return what was exported.
    pub fn export_backup_to(&self, path: &std::path::Path, options: &backup::ExportOptions) -> Result<backup::BackupFile> {
        let file = self.store.export_backup(options, env!("CARGO_PKG_VERSION"))?;
        backup::write_backup_atomically(path, &file)?;
        Ok(file)
    }

    async fn provider_refs(&self, notes: &mut Vec<String>) -> Vec<backup_android::ProviderRef> {
        match self.providers().await {
            Ok(list) => list
                .into_iter()
                .map(|p| backup_android::ProviderRef { id: p.id, name: p.name, main_url: p.main_url })
                .collect(),
            Err(error) => {
                notes.push(format!("Installed sources could not be listed ({error}), so no title could be matched to a source yet. They are kept and matched later."));
                Vec::new()
            }
        }
    }

    async fn import_set_for(&self, bytes: &[u8], options: &backup::ImportOptions) -> Result<(backup::ImportSet, Option<u64>, Option<String>, Vec<String>)> {
        let parsed = backup::parse_backup(bytes, self.store.schema_version()?)?;
        let mut notes = Vec::new();
        let (set, created, version) = match parsed {
            backup::ParsedBackup::Desktop(file) => (backup::desktop_import_set(&file), Some(file.created_at_ms), Some(file.app_version.clone())),
            backup::ParsedBackup::Android(root) => {
                let android = backup_android::parse_android(&root)?;
                let providers = self.provider_refs(&mut notes).await;
                let profiles: Vec<(String, String)> = self.store.list_profiles()?.into_iter().map(|p| (p.id, p.name)).collect();
                (backup_android::plan_android(&android, &providers, &profiles, options), None, None)
            }
        };
        if let Some(requested) = &options.sections {
            for section in requested {
                if !set.sections.iter().any(|s| s == section) && !backup::ALL_SECTIONS.contains(&section.as_str()) {
                    bail!("This backup has no '{section}' section");
                }
            }
        }
        Ok((set, created, version, notes))
    }

    /// Validate a backup file and report what importing it would do. Changes nothing.
    pub async fn preview_import(&self, bytes: &[u8], options: &backup::ImportOptions) -> Result<backup::ImportPreview> {
        let (set, created_at_ms, app_version, extra_notes) = self.import_set_for(bytes, options).await?;
        let mut report = self.store.run_import(&set, options, true)?;
        report.notes.extend(extra_notes);
        Ok(backup::ImportPreview {
            source: set.source.unwrap_or(backup::ImportSource::Desktop),
            created_at_ms,
            app_version,
            available_sections: set.sections.clone(),
            report,
            token: backup::preview_token(bytes, options)?,
        })
    }

    /// Apply a previewed import. The token must come from the preview of this exact file and
    /// these exact options. A copy of the current data is saved first, and the import is one
    /// transaction: any failure leaves everything as it was.
    pub async fn apply_import(&self, bytes: &[u8], options: &backup::ImportOptions, token: &str) -> Result<backup::ImportReport> {
        if backup::preview_token(bytes, options)? != token {
            bail!("The backup or the import choices changed since the preview. Preview the import again.");
        }
        let (set, _, _, extra_notes) = self.import_set_for(bytes, options).await?;
        let safety = self.store.export_backup(&backup::ExportOptions::default(), env!("CARGO_PKG_VERSION"))?;
        let safety_path = self.data.join("backups").join(format!("pre-import-{}.json", timestamp_ms()));
        backup::write_backup_atomically(&safety_path, &safety).context("Could not save a safety copy before importing, so nothing was changed")?;
        let mut report = self.store.run_import(&set, options, false)?;
        report.notes.extend(extra_notes);
        report.pre_import_backup = Some(safety_path.display().to_string());
        self.home_cache.lock().unwrap().clear();
        Ok(report)
    }

    /// Match titles kept from an Android import to the sources installed now.
    pub async fn resolve_unresolved_imports(&self, profile_id: &str) -> Result<usize> {
        let kept: Vec<(String, Value)> = self
            .store
            .list::<Value>("unresolved_import")?
            .into_iter()
            .filter(|value| value.get("profileId").and_then(Value::as_str) == Some(profile_id))
            .filter_map(|value| Some((value.get("recordId")?.as_str()?.to_string(), value)))
            .collect();
        if kept.is_empty() {
            return Ok(0);
        }
        let mut notes = Vec::new();
        let providers = self.provider_refs(&mut notes).await;
        let mut resolved = 0;
        for (id, value) in kept {
            let Some((library, progress)) = backup_android::resolve_kept(profile_id, &value, &providers) else { continue };
            if let Some(record) = library {
                if self.store.get::<Value>("library", &record.id)?.is_none() {
                    self.store.put("library", &record.id, &record.value)?;
                }
            }
            if let Some(row) = progress {
                let set = backup::ImportSet { progress: vec![row], ..Default::default() };
                self.store.run_import(&set, &backup::ImportOptions::default(), false)?;
            }
            self.store.remove("unresolved_import", &id)?;
            resolved += 1;
        }
        Ok(resolved)
    }

    pub fn get_search_history(&self, profile_id: &str) -> Result<Vec<SearchHistoryEntry>> {
        self.store.get_search_history(profile_id)
    }

    pub fn add_search_history(&self, profile_id: &str, query: &str) -> Result<SearchHistoryEntry> {
        self.store.add_search_history(profile_id, query)
    }

    pub fn delete_search_history_entry(&self, profile_id: &str, query: &str) -> Result<()> {
        self.store.delete_search_history_entry(profile_id, query)
    }

    pub fn clear_search_history(&self, profile_id: &str) -> Result<()> {
        self.store.clear_search_history(profile_id)
    }

    pub async fn search_provider(
        &self,
        provider_id: &str,
        query: &str,
        page: u32,
        request_id: Option<&str>,
    ) -> Result<SearchResult> {
        let val = self
            .call_cancellable(
                request_id,
                "search",
                serde_json::json!({
                    "providerId": provider_id,
                    "query": query,
                    "page": page,
                }),
            )
            .await?;
        let res: SearchResult = serde_json::from_value(val)?;
        Ok(res)
    }

    pub async fn quick_search_provider(
        &self,
        provider_id: &str,
        query: &str,
        request_id: Option<&str>,
    ) -> Result<SearchResult> {
        let val = match self
            .call_cancellable(
                request_id,
                "quickSearch",
                serde_json::json!({
                    "providerId": provider_id,
                    "query": query,
                }),
            )
            .await
        {
            Ok(v) => v,
            Err(_) => {
                self.call_cancellable(
                    request_id,
                    "search",
                    serde_json::json!({
                        "providerId": provider_id,
                        "query": query,
                        "page": 1,
                    }),
                )
                .await?
            }
        };
        let res: SearchResult = serde_json::from_value(val)?;
        Ok(res)
    }

    pub async fn search_suggestions(
        &self,
        profile_id: &str,
        query: &str,
        provider_id: Option<&str>,
    ) -> Result<Vec<String>> {
        let trimmed = query.trim();
        if trimmed.len() < 2 {
            return Ok(Vec::new());
        }

        let mut suggestions = Vec::new();
        let query_lower = trimmed.to_lowercase();

        // 1. History matches
        if let Ok(history) = self.store.get_search_history(profile_id) {
            for entry in history {
                if entry.query.to_lowercase().contains(&query_lower) {
                    if !suggestions.iter().any(|s: &String| s.eq_ignore_ascii_case(&entry.query)) {
                        suggestions.push(entry.query);
                    }
                }
                if suggestions.len() >= 5 {
                    break;
                }
            }
        }

        // 2. Provider quickSearch matches (if requested and supported)
        if let Some(pid) = provider_id {
            let providers = self.providers().await.unwrap_or_default();
            if let Some(prov) = providers.iter().find(|p| p.id == pid) {
                if prov.capabilities.as_ref().is_some_and(|c| c.has_quick_search) {
                    if let Ok(res) = self.quick_search_provider(pid, trimmed, None).await {
                        for item in res.items {
                            if !suggestions.iter().any(|s: &String| s.eq_ignore_ascii_case(&item.name)) {
                                suggestions.push(item.name);
                            }
                            if suggestions.len() >= 10 {
                                break;
                            }
                        }
                    }
                }
            }
        }

        suggestions.truncate(10);
        Ok(suggestions)
    }
}

pub fn clean_interrupted_updates(dir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                clean_interrupted_updates(&path);
            } else if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| name.ends_with(".tmp") || name.contains(".tmp."))
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
