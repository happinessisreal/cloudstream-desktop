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
}
impl App {
    pub fn open(data: PathBuf, resources: PathBuf) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data)?;
        let bundled_java = resources.join("jre/bin/java");
        let java = if bundled_java.is_file() {
            bundled_java
        } else {
            PathBuf::from("java")
        };
        Ok(Arc::new(Self {
            store: Store::open(&data.join("cloudstream.db"))?,
            repositories: RepositoryManager::new(data.join("extensions"))?,
            worker: Worker::new(WorkerConfig {
                java,
                jar: resources.join("worker/worker.jar"),
            }),
            player: player::Player::new(std::env::temp_dir()),
            resources,
            data,
            mutation: Mutex::new(()),
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
        Ok(serde_json::from_value(
            self.worker
                .request(&installed, "listProviders", json!({}))
                .await?,
        )?)
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.worker
            .request(&self.store.installed()?, method, params)
            .await
    }
    pub async fn extension_states(&self) -> Result<Value> {
        if self.store.installed()?.is_empty() {
            return Ok(json!([]));
        }
        self.call("getPluginStates", json!({})).await
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
    pub fn bookmark(&self, item: MediaItem) -> Result<LibraryItem> {
        let entry = LibraryItem {
            id: plugin_id(&item.provider_id, &item.url),
            item,
            saved_at: timestamp(),
        };
        self.store.put("library", &entry.id, &entry)?;
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
            (Some(item), Some(episode)) => Some(WatchEntry {
                id: plugin_id(&item.provider_id, &episode.data),
                item,
                episode,
                position: resume,
                duration: 0.0,
                watched_at: timestamp(),
            }),
            _ => None,
        };
        self.player
            .play(&stream, &subtitles, watch.clone(), resume)
            .await?;
        if let Some(watch) = watch {
            self.store.put("history", &watch.id, &watch)?;
        }
        Ok(())
    }
    pub async fn save_progress(&self) -> Result<()> {
        if let Some(watch) = self.player.progress().await? {
            self.store.put("history", &watch.id, &watch)?;
        }
        Ok(())
    }
}
