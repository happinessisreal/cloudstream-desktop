use crate::models::*;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;

pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub run: fn(&Transaction) -> Result<()>,
}

fn migration_1(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS records (
            kind TEXT NOT NULL,
            id TEXT NOT NULL,
            value TEXT NOT NULL,
            updated INTEGER NOT NULL,
            PRIMARY KEY(kind, id)
        );",
    )?;
    Ok(())
}

fn migration_2(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_records_kind_updated ON records(kind, updated DESC, id);",
    )?;
    Ok(())
}

fn migration_3(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS aliases (
            alias_key TEXT NOT NULL PRIMARY KEY,
            target_kind TEXT NOT NULL,
            target_id TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_aliases_target ON aliases(target_kind, target_id);

        CREATE TABLE IF NOT EXISTS episode_payloads (
            episode_id TEXT NOT NULL PRIMARY KEY,
            media_id TEXT NOT NULL,
            data TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS watch_progress (
            profile_id TEXT NOT NULL,
            media_id TEXT NOT NULL,
            episode_id TEXT NOT NULL,
            position_seconds REAL NOT NULL,
            duration_seconds REAL,
            updated_at_ms INTEGER NOT NULL,
            is_watched INTEGER NOT NULL DEFAULT 0,
            completion_reason TEXT,
            extra TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY(profile_id, episode_id)
        );
        CREATE INDEX IF NOT EXISTS idx_watch_progress_media ON watch_progress(profile_id, media_id, updated_at_ms DESC);",
    )?;

    // 1. Migrate library bookmarks
    {
        let mut stmt = tx.prepare("SELECT id, value, updated FROM records WHERE kind = 'library'")?;
        let rows: Vec<(String, String, u64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (legacy_id, value_str, updated) in rows {
            if let Ok(mut item_record) = serde_json::from_str::<serde_json::Value>(&value_str) {
                if let Some(item_obj) = item_record.get_mut("item") {
                    let provider_id = item_obj
                        .get("providerId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let url = item_obj
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let unique_url = item_obj
                        .get("uniqueUrl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let media_id = compute_media_id(&provider_id, &url, unique_url.as_deref());
                    item_obj
                        .as_object_mut()
                        .unwrap()
                        .insert("mediaId".into(), serde_json::json!(media_id));

                    let now_ms = to_epoch_millis(updated);
                    tx.execute(
                        "INSERT OR IGNORE INTO aliases (alias_key, target_kind, target_id, created_at) VALUES (?1, 'media', ?2, ?3)",
                        params![legacy_id, media_id, now_ms],
                    )?;

                    if let Some(saved_at) = item_record.get("savedAt").and_then(|v| v.as_u64()) {
                        item_record
                            .as_object_mut()
                            .unwrap()
                            .insert("savedAt".into(), serde_json::json!(to_epoch_millis(saved_at)));
                    }
                    item_record
                        .as_object_mut()
                        .unwrap()
                        .insert("id".into(), serde_json::json!(media_id));

                    let new_value = serde_json::to_string(&item_record)?;
                    tx.execute(
                        "DELETE FROM records WHERE kind = 'library' AND id = ?1",
                        params![legacy_id],
                    )?;
                    tx.execute(
                        "INSERT INTO records (kind, id, value, updated) VALUES ('library', ?1, ?2, ?3) ON CONFLICT(kind, id) DO UPDATE SET value = excluded.value, updated = excluded.updated",
                        params![media_id, new_value, now_ms],
                    )?;
                }
            }
        }
    }

    // 2. Migrate history records
    {
        let mut stmt = tx.prepare("SELECT id, value, updated FROM records WHERE kind = 'history'")?;
        let rows: Vec<(String, String, u64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (legacy_id, value_str, updated) in rows {
            if let Ok(mut hist_record) = serde_json::from_str::<serde_json::Value>(&value_str) {
                let provider_id = hist_record
                    .get("item")
                    .and_then(|i| i.get("providerId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = hist_record
                    .get("item")
                    .and_then(|i| i.get("url"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let unique_url = hist_record
                    .get("item")
                    .and_then(|i| i.get("uniqueUrl"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let media_id = compute_media_id(&provider_id, &url, unique_url.as_deref());

                let episode_val = hist_record.get("episode").cloned().unwrap_or(serde_json::json!({}));
                let ep_name = episode_val.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let ep_data = episode_val.get("data").and_then(|v| v.as_str()).unwrap_or("");
                let ep_season = episode_val.get("season").and_then(|v| v.as_i64()).map(|s| s as i32);
                let ep_number = episode_val.get("episode").and_then(|v| v.as_i64()).map(|e| e as i32);
                let ep_id_explicit = episode_val.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());

                let dummy_episode = Episode {
                    name: ep_name.to_string(),
                    data: ep_data.to_string(),
                    season: ep_season,
                    episode: ep_number,
                    poster_url: None,
                    id: ep_id_explicit,
                    dub_status: None,
                    index: None,
                    score: None,
                    rating: None,
                    description: None,
                    date: None,
                    run_time: None,
                    extra: std::collections::BTreeMap::new(),
                };

                let episode_id = compute_episode_id(&media_id, &dummy_episode, None, None);
                let legacy_data_alias = crate::repository::plugin_id(&provider_id, ep_data);
                let now_ms = to_epoch_millis(updated);

                tx.execute(
                    "INSERT OR IGNORE INTO aliases (alias_key, target_kind, target_id, created_at) VALUES (?1, 'episode', ?2, ?3)",
                    params![legacy_data_alias, episode_id, now_ms],
                )?;
                if legacy_id != episode_id {
                    tx.execute(
                        "INSERT OR IGNORE INTO aliases (alias_key, target_kind, target_id, created_at) VALUES (?1, 'episode', ?2, ?3)",
                        params![legacy_id, episode_id, now_ms],
                    )?;
                }

                if !ep_data.is_empty() {
                    tx.execute(
                        "INSERT OR REPLACE INTO episode_payloads (episode_id, media_id, data, updated_at) VALUES (?1, ?2, ?3, ?4)",
                        params![episode_id, media_id, ep_data, now_ms],
                    )?;
                }

                let position = hist_record.get("position").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let duration = hist_record.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let is_watched = if duration > 0.0 && position >= 0.9 * duration { 1 } else { 0 };

                tx.execute(
                    "INSERT OR REPLACE INTO watch_progress (profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched) VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6)",
                    params![media_id, episode_id, position, duration, now_ms, is_watched],
                )?;

                if let Some(item_obj) = hist_record.get_mut("item") {
                    item_obj.as_object_mut().unwrap().insert("mediaId".into(), serde_json::json!(media_id));
                }
                if let Some(ep_obj) = hist_record.get_mut("episode") {
                    ep_obj.as_object_mut().unwrap().insert("id".into(), serde_json::json!(episode_id));
                }
                hist_record.as_object_mut().unwrap().insert("id".into(), serde_json::json!(episode_id));
                hist_record.as_object_mut().unwrap().insert("watchedAt".into(), serde_json::json!(now_ms));

                let new_value = serde_json::to_string(&hist_record)?;
                tx.execute(
                    "DELETE FROM records WHERE kind = 'history' AND id = ?1",
                    params![legacy_id],
                )?;
                tx.execute(
                    "INSERT INTO records (kind, id, value, updated) VALUES ('history', ?1, ?2, ?3) ON CONFLICT(kind, id) DO UPDATE SET value = excluded.value, updated = excluded.updated",
                    params![episode_id, new_value, now_ms],
                )?;
            }
        }
    }

    Ok(())
}

pub fn standard_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "Initial records table",
            run: migration_1,
        },
        Migration {
            version: 2,
            description: "Records index and settings schema",
            run: migration_2,
        },
        Migration {
            version: 3,
            description: "Stable identity, aliases, payloads and watch progress",
            run: migration_3,
        },
    ]
}

pub fn execute_migrations(connection: &mut Connection, migrations: &[Migration]) -> Result<()> {
    let current_version: u32 =
        connection.query_row("PRAGMA user_version;", [], |r| r.get(0))?;

    for m in migrations {
        if m.version > current_version {
            let tx = connection.transaction()?;
            (m.run)(&tx).with_context(|| {
                format!("Migration {} failed ({})", m.version, m.description)
            })?;
            tx.pragma_update(None, "user_version", m.version)?;
            tx.commit()?;
        }
    }
    Ok(())
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA journal_mode=WAL;")?;
        execute_migrations(&mut connection, &standard_migrations())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute(
                "INSERT INTO records(kind,id,value,updated) VALUES (?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
                params![kind, id, serde_json::to_string(value)?, timestamp()],
            )?;
        Ok(())
    }

    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut statement = connection
            .prepare("SELECT value FROM records WHERE kind=?1 ORDER BY updated DESC,id")?;
        let result = statement
            .query_map([kind], |r| r.get::<_, String>(0))?
            .map(|r| serde_json::from_str(&r?).context("Stored record could not be read"))
            .collect();
        result
    }

    pub fn remove(&self, kind: &str, id: &str) -> Result<()> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute(
                "DELETE FROM records WHERE kind=?1 AND id=?2",
                params![kind, id],
            )?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut statement = connection
            .prepare("SELECT value FROM records WHERE kind=?1 AND id=?2")?;
        let mut rows = statement.query(params![kind, id])?;
        if let Some(row) = rows.next()? {
            let s: String = row.get(0)?;
            let item = serde_json::from_str(&s).context("Stored record could not be read")?;
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }

    pub fn repositories(&self) -> Result<Vec<Repository>> {
        self.list("repository")
    }

    pub fn list_disabled_providers(&self) -> Result<std::collections::HashSet<String>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut statement = connection
            .prepare("SELECT id FROM records WHERE kind='provider_disabled'")?;
        let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for r in rows {
            set.insert(r?);
        }
        Ok(set)
    }

    pub fn set_provider_disabled(&self, provider_id: &str, disabled: bool) -> Result<()> {
        if disabled {
            self.put("provider_disabled", provider_id, &true)
        } else {
            self.remove("provider_disabled", provider_id)
        }
    }

    pub fn list_provider_clones(&self) -> Result<Vec<ProviderCloneConfig>> {
        self.list("provider_clone")
    }

    pub fn save_provider_clone(&self, clone: &ProviderCloneConfig) -> Result<()> {
        self.put("provider_clone", &clone.id, clone)
    }

    pub fn delete_provider_clone(&self, clone_id: &str) -> Result<()> {
        self.remove("provider_clone", clone_id)
    }

    pub fn installed(&self) -> Result<Vec<InstalledPlugin>> {
        self.list("plugin")
    }

    pub fn library(&self) -> Result<Vec<LibraryItem>> {
        self.list("library")
    }

    pub fn history(&self) -> Result<Vec<WatchEntry>> {
        self.list("history")
    }

    pub fn get_settings(&self) -> Result<Settings> {
        let list: Vec<Settings> = self.list("settings")?;
        if let Some(s) = list.into_iter().next() {
            Ok(s)
        } else {
            let default_settings = Settings::default();
            self.put("settings", "app", &default_settings)?;
            Ok(default_settings)
        }
    }

    pub fn update_settings(
        &self,
        patch: serde_json::Value,
        scope: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<Settings> {
        let mut settings = self.get_settings()?;
        apply_settings_patch(&mut settings, &patch, scope, profile_id)?;
        settings.revision = settings.revision.saturating_add(1);
        self.put("settings", "app", &settings)?;
        Ok(settings)
    }

    pub fn schema_version(&self) -> Result<u32> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let version: u32 = connection.query_row("PRAGMA user_version;", [], |r| r.get(0))?;
        Ok(version)
    }

    pub fn put_alias(&self, alias_key: &str, target_kind: &str, target_id: &str) -> Result<()> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute(
                "INSERT OR REPLACE INTO aliases (alias_key, target_kind, target_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![alias_key, target_kind, target_id, timestamp_ms()],
            )?;
        Ok(())
    }

    pub fn resolve_alias(&self, alias_key: &str, target_kind: &str) -> Result<Option<String>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT target_id FROM aliases WHERE alias_key = ?1 AND target_kind = ?2",
        )?;
        let mut rows = stmt.query([alias_key, target_kind])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn save_episode_payload(&self, episode_id: &str, media_id: &str, data: &str) -> Result<()> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute(
                "INSERT OR REPLACE INTO episode_payloads (episode_id, media_id, data, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![episode_id, media_id, data, timestamp_ms()],
            )?;
        Ok(())
    }

    pub fn get_episode_payload(&self, episode_id: &str) -> Result<Option<String>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare("SELECT data FROM episode_payloads WHERE episode_id = ?1")?;
        let mut rows = stmt.query([episode_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn save_watch_progress(&self, progress: &WatchProgress) -> Result<()> {
        let extra_str = serde_json::to_string(&progress.extra)?;
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute(
                "INSERT OR REPLACE INTO watch_progress (profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched, completion_reason, extra) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    progress.profile_id,
                    progress.media_id,
                    progress.episode_id,
                    progress.position_seconds,
                    progress.duration_seconds,
                    progress.updated_at_ms,
                    if progress.is_watched { 1 } else { 0 },
                    progress.completion_reason,
                    extra_str
                ],
            )?;
        Ok(())
    }

    pub fn get_watch_progress(&self, profile_id: &str, episode_id: &str) -> Result<Option<WatchProgress>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched, completion_reason, extra FROM watch_progress WHERE profile_id = ?1 AND episode_id = ?2"
        )?;
        let mut rows = stmt.query([profile_id, episode_id])?;
        if let Some(row) = rows.next()? {
            let extra_str: String = row.get(8).unwrap_or_else(|_| "{}".into());
            let extra: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(&extra_str).unwrap_or_default();
            Ok(Some(WatchProgress {
                profile_id: row.get(0)?,
                media_id: row.get(1)?,
                episode_id: row.get(2)?,
                position_seconds: row.get(3)?,
                duration_seconds: row.get(4)?,
                updated_at_ms: row.get(5)?,
                is_watched: row.get::<_, i64>(6)? != 0,
                completion_reason: row.get(7)?,
                extra,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn save_watch_entry(&self, watch: &WatchEntry, profile_id: &str) -> Result<()> {
        let media_id = watch
            .item
            .media_id
            .clone()
            .unwrap_or_else(|| compute_media_id(&watch.item.provider_id, &watch.item.url, watch.item.unique_url.as_deref()));
        let episode_id = watch.id.clone();

        self.put("history", &episode_id, watch)?;
        self.save_episode_payload(&episode_id, &media_id, &watch.episode.data)?;

        let legacy_data_alias = crate::repository::plugin_id(&watch.item.provider_id, &watch.episode.data);
        self.put_alias(&legacy_data_alias, "episode", &episode_id)?;

        let is_watched = watch.duration > 0.0 && watch.position >= 0.9 * watch.duration;
        let progress = WatchProgress {
            profile_id: profile_id.to_string(),
            media_id,
            episode_id,
            position_seconds: watch.position,
            duration_seconds: if watch.duration > 0.0 { Some(watch.duration) } else { None },
            updated_at_ms: watch.watched_at,
            is_watched,
            completion_reason: None,
            extra: std::collections::BTreeMap::new(),
        };
        self.save_watch_progress(&progress)?;
        Ok(())
    }

    pub fn get_search_history(&self, profile_id: &str) -> Result<Vec<SearchHistoryEntry>> {
        let prefix = format!("{profile_id}::");
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT value FROM records WHERE kind = 'search_history' AND substr(id, 1, length(?1)) = ?1 ORDER BY updated DESC, rowid DESC LIMIT 50",
        )?;
        let rows = stmt.query_map([&prefix], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::new();
        for r in rows {
            let s = r?;
            if let Ok(entry) = serde_json::from_str::<SearchHistoryEntry>(&s) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn add_search_history(&self, profile_id: &str, query: &str) -> Result<SearchHistoryEntry> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            bail!("Search query cannot be empty");
        }
        let now_ms = timestamp_ms();
        let id = format!("{profile_id}::{trimmed}");
        let entry = SearchHistoryEntry {
            id: id.clone(),
            profile_id: profile_id.to_string(),
            query: trimmed.to_string(),
            searched_at: now_ms,
        };
        let value = serde_json::to_string(&entry)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "INSERT INTO records (kind, id, value, updated) VALUES ('search_history', ?1, ?2, ?3)
             ON CONFLICT(kind, id) DO UPDATE SET value = excluded.value, updated = excluded.updated",
            params![id, value, now_ms],
        )?;
        Ok(entry)
    }

    pub fn delete_search_history_entry(&self, profile_id: &str, query: &str) -> Result<()> {
        let id = format!("{profile_id}::{}", query.trim());
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'search_history' AND id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn clear_search_history(&self, profile_id: &str) -> Result<()> {
        let prefix = format!("{profile_id}::");
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'search_history' AND substr(id, 1, length(?1)) = ?1",
            params![prefix],
        )?;
        Ok(())
    }
}

pub fn json_merge(target: &mut serde_json::Value, patch: &serde_json::Value) {
    match (target, patch) {
        (serde_json::Value::Object(target_map), serde_json::Value::Object(patch_map)) => {
            for (key, val) in patch_map {
                if let Some(target_val) = target_map.get_mut(key) {
                    if target_val.is_object() && val.is_object() {
                        json_merge(target_val, val);
                    } else {
                        *target_val = val.clone();
                    }
                } else {
                    target_map.insert(key.clone(), val.clone());
                }
            }
        }
        (t, p) => {
            *t = p.clone();
        }
    }
}

pub fn apply_settings_patch(
    settings: &mut Settings,
    patch: &serde_json::Value,
    scope: Option<&str>,
    profile_id: Option<&str>,
) -> Result<()> {
    if !patch.is_object() {
        bail!("Settings patch must be a JSON object");
    }

    validate_patch_values(patch)?;

    if let Some(sv) = patch.get("schemaVersion") {
        if let Some(n) = sv.as_u64() {
            if n == 0 {
                bail!("schemaVersion must be >= 1");
            }
        } else {
            bail!("schemaVersion must be a positive integer");
        }
    }

    let mut current_json = serde_json::to_value(&*settings)?;

    let is_profile = scope == Some("profile") || profile_id.is_some();
    if is_profile {
        let pid = profile_id.unwrap_or("default");
        let profiles = current_json
            .as_object_mut()
            .unwrap()
            .entry("profiles")
            .or_insert_with(|| serde_json::json!({}));
        if !profiles.is_object() {
            *profiles = serde_json::json!({});
        }
        let profile_obj = profiles
            .as_object_mut()
            .unwrap()
            .entry(pid)
            .or_insert_with(|| serde_json::json!({}));

        let mut normalized_patch = patch.clone();
        if let Some(pd) = patch.get("posterDensity") {
            let mut app = normalized_patch
                .get("appearance")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            if let Some(app_obj) = app.as_object_mut() {
                app_obj.insert("posterDensity".to_string(), pd.clone());
            }
            if let Some(obj) = normalized_patch.as_object_mut() {
                obj.remove("posterDensity");
                obj.insert("appearance".to_string(), app);
            }
        }

        json_merge(profile_obj, &normalized_patch);
    } else {
        let mut normalized_patch = patch.clone();
        if let Some(pd) = patch.get("posterDensity") {
            let mut app = normalized_patch
                .get("appearance")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            if let Some(app_obj) = app.as_object_mut() {
                app_obj.insert("posterDensity".to_string(), pd.clone());
            }
            if let Some(obj) = normalized_patch.as_object_mut() {
                obj.remove("posterDensity");
                obj.insert("appearance".to_string(), app);
            }
        }

        if normalized_patch.get("global").is_none()
            && (scope == Some("global") || normalized_patch.get("appearance").is_some())
        {
            let mut global_patch = serde_json::Map::new();
            let mut root_patch = serde_json::Map::new();

            if let Some(app) = normalized_patch.get("appearance") {
                global_patch.insert("appearance".to_string(), app.clone());
            }

            if let Some(obj) = normalized_patch.as_object() {
                for (k, v) in obj {
                    if k == "appearance" || k == "posterDensity" {
                        continue;
                    }
                    if scope == Some("global") {
                        global_patch.insert(k.clone(), v.clone());
                    } else {
                        root_patch.insert(k.clone(), v.clone());
                    }
                }
            }

            if !global_patch.is_empty() {
                let global_json = current_json
                    .as_object_mut()
                    .unwrap()
                    .entry("global")
                    .or_insert_with(|| serde_json::json!({}));
                json_merge(global_json, &serde_json::Value::Object(global_patch));
            }

            if !root_patch.is_empty() {
                json_merge(&mut current_json, &serde_json::Value::Object(root_patch));
            }
        } else {
            json_merge(&mut current_json, &normalized_patch);
        }
    }

    let updated: Settings = serde_json::from_value(current_json)?;
    *settings = updated;
    Ok(())
}

fn validate_patch_values(val: &serde_json::Value) -> Result<()> {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "posterDensity" {
                    if let Some(s) = v.as_str() {
                        if PosterDensity::parse(s).is_none() {
                            bail!(
                                "Invalid poster density: '{s}'. Expected 'comfortable' or 'compact'"
                            );
                        }
                    } else {
                        bail!("Invalid poster density: expected string 'comfortable' or 'compact'");
                    }
                } else if k == "schemaVersion" {
                    if let Some(n) = v.as_u64() {
                        if n == 0 {
                            bail!("schemaVersion must be >= 1");
                        }
                    } else {
                        bail!("schemaVersion must be a positive integer");
                    }
                } else {
                    validate_patch_values(v)?;
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                validate_patch_values(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
