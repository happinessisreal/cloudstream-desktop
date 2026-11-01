pub mod models;
pub mod player;
pub mod repository;
pub mod store;
pub mod worker;

use anyhow::{bail, Context, Result};
use models::*;
use repository::{digest, plugin_id, RepositoryManager};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use store::Store;
use tokio::sync::Mutex;
use worker::{Worker, WorkerConfig};

pub struct App {
    pub store: Store,
    pub repositories: RepositoryManager,
    pub worker: Worker,
    pub player: player::Player,
    pub resources: PathBuf,
    pub data: PathBuf,
    pub mutation: Mutex<()>,
    pub client_requests: Mutex<std::collections::HashMap<String, u64>>,
}
impl App {
    pub fn open(data: PathBuf, resources: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data)?;
        let ext_dir = data.join("extensions");
        clean_interrupted_updates(&ext_dir);
        let bundled_java = resources.join("jre/bin/java");
        let java = if bundled_java.is_file() {
            bundled_java
        } else {
            PathBuf::from("java")
        };
        Ok(Arc::new(Self {
            store: Store::open(&data.join("cloudstream.db"))?,
            repositories: RepositoryManager::new(ext_dir)?,
            worker: Worker::new(WorkerConfig {
                java,
                jar: resources.join("worker/worker.jar"),
            }),
            player: player::Player::new(std::env::temp_dir()),
            resources,
            data,
            mutation: Mutex::new(()),
            client_requests: Mutex::new(std::collections::HashMap::new()),
        }))
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
        self.store.set_provider_disabled(provider_id, !enabled)
    }
    pub fn configure_provider_clone(&self, clone: ProviderCloneConfig) -> Result<()> {
        self.store.save_provider_clone(&clone)
    }
    pub fn delete_provider_clone(&self, clone_id: &str) -> Result<()> {
        self.store.delete_provider_clone(clone_id)
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
        let res = self
            .worker
            .request_with_id(&installed, id, method, effective_params, None)
            .await;
        if let Some(req_id) = request_id {
            self.client_requests.lock().await.remove(req_id);
        }
        res
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
    pub async fn remove_plugin(&self, id: &str) -> Result<()> {
        let _guard = self.mutation.lock().await;
        if !self.store.installed()?.iter().any(|p| p.id == id) {
            bail!("Extension is not installed")
        }
        self.worker.stop().await;
        self.store.remove("plugin", id)?;
        // Retain content-addressed packages as a local recovery cache.
        Ok(())
    }
    pub async fn remove_repository(&self, url: &str) -> Result<()> {
        let _guard = self.mutation.lock().await;
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

        let entry = LibraryItem {
            id: media_id.clone(),
            item,
            saved_at: now_ms,
        };
        self.store.put("library", &entry.id, &entry)?;
        self.store.put_alias(&legacy_id, "media", &media_id)?;
        Ok(entry)
    }
    pub async fn play(
        &self,
        stream: Stream,
        subtitles: Vec<Subtitle>,
        item: Option<MediaItem>,
        episode: Option<Episode>,
        resume: f64,
    ) -> Result<()> {
        self.save_progress().await?;
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
                    self.store.get_watch_progress("default", &episode_id)
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
        let start_pos = watch.as_ref().map(|w| w.position).unwrap_or(resume);
        self.player
            .play(&stream, &subtitles, watch.clone(), start_pos)
            .await?;
        if let Some(watch) = watch {
            self.store.save_watch_entry(&watch, "default")?;
        }
        Ok(())
    }
    pub async fn save_progress(&self) -> Result<()> {
        if let Some(watch) = self.player.progress().await? {
            self.store.save_watch_entry(&watch, "default")?;
        }
        Ok(())
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
