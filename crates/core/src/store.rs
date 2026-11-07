use crate::models::*;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Mutex};

pub const CURRENT_SCHEMA_VERSION: u32 = 5;

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

fn migration_4(tx: &Transaction) -> Result<()> {
    let has_col: bool = tx
        .prepare("PRAGMA table_info(watch_progress)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == "watched_override");
    if !has_col {
        tx.execute_batch("ALTER TABLE watch_progress ADD COLUMN watched_override INTEGER;")?;
    }
    Ok(())
}

fn migration_5(tx: &Transaction) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS profiles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            avatar TEXT NOT NULL,
            pin_salt TEXT,
            pin_hash TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO profiles (id, name, avatar, created_at, updated_at)
        VALUES ('default', 'Default Profile', 'default', 0, 0);
        UPDATE records SET id = 'default::' || id WHERE kind = 'library' AND id NOT LIKE 'default::%';
        UPDATE records SET id = 'default::' || id WHERE kind = 'history' AND id NOT LIKE 'default::%';
        UPDATE watch_progress SET profile_id = 'default' WHERE profile_id IS NULL OR profile_id = '';",
    )?;
    Ok(())
}

pub fn hash_pin(pin: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{salt}:{pin}").as_bytes());
    format!("{:x}", hasher.finalize())
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
        Migration {
            version: 4,
            description: "Add watched_override column to watch_progress",
            run: migration_4,
        },
        Migration {
            version: 5,
            description: "Profiles table, default profile and profile-scoped records",
            run: migration_5,
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

fn provider_enabled_in(connection: &Connection, provider_id: &str) -> Result<bool> {
    let is_disabled = |id: &str| -> Result<bool> {
        Ok(connection.query_row("SELECT 1 FROM records WHERE kind='provider_disabled' AND id=?1", [id], |_| Ok(())).optional()?.is_some())
    };
    if is_disabled(provider_id)? { return Ok(false); }
    let clone: Option<String> = connection.query_row("SELECT value FROM records WHERE kind='provider_clone' AND id=?1", [provider_id], |row| row.get(0)).optional()?;
    if let Some(raw) = clone {
        let clone: ProviderCloneConfig = serde_json::from_str(&raw)?;
        if !clone.enabled || is_disabled(&clone.parent_provider_id)? { return Ok(false); }
    }
    Ok(true)
}

fn validate_download_lease(connection: &Connection, lease: &crate::download_queue::QueueLease) -> Result<()> {
    let db_file: String = connection.query_row("SELECT file FROM pragma_database_list WHERE name='main'", [], |row| row.get(0))?;
    let parent = Path::new(&db_file).parent().context("Download database has no parent folder")?;
    if std::fs::canonicalize(parent)? != lease.directory { bail!("Download queue lock belongs to another database"); }
    Ok(())
}

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    /// Run `f` with exclusive access to the database connection. Used by backup/restore, which
    /// need one transaction across several tables.
    pub(crate) fn with_connection<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        f(&mut connection)
    }

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

    pub fn provider_is_enabled(&self, provider_id: &str) -> Result<bool> {
        let connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        provider_enabled_in(&connection, provider_id)
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

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT id, name, avatar, pin_hash, created_at, updated_at FROM profiles ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let pin_hash: Option<String> = row.get(3)?;
            Ok(Profile {
                id: row.get(0)?,
                name: row.get(1)?,
                avatar: row.get(2)?,
                has_pin: pin_hash.is_some(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        let mut profiles = Vec::new();
        for r in rows {
            profiles.push(r?);
        }
        Ok(profiles)
    }

    pub fn get_profile(&self, id: &str) -> Result<Option<Profile>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT id, name, avatar, pin_hash, created_at, updated_at FROM profiles WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], |row| {
            let pin_hash: Option<String> = row.get(3)?;
            Ok(Profile {
                id: row.get(0)?,
                name: row.get(1)?,
                avatar: row.get(2)?,
                has_pin: pin_hash.is_some(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    pub fn create_profile(&self, input: CreateProfileInput) -> Result<Profile> {
        let name = input.name.trim();
        if name.is_empty() {
            bail!("Profile name cannot be empty");
        }
        let id = format!("prof_{}", uuid::Uuid::new_v4().simple());
        let avatar = input.avatar.unwrap_or_else(|| "default".into());
        let now = timestamp_ms();
        let (pin_salt, pin_hash) = if let Some(pin) = input.pin {
            let pin = pin.trim();
            if pin.is_empty() {
                (None, None)
            } else {
                let salt = uuid::Uuid::new_v4().simple().to_string();
                let hash = hash_pin(pin, &salt);
                (Some(salt), Some(hash))
            }
        } else {
            (None, None)
        };

        let has_pin = pin_hash.is_some();
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "INSERT INTO profiles (id, name, avatar, pin_salt, pin_hash, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, name, avatar, pin_salt, pin_hash, now, now],
        )?;

        Ok(Profile {
            id,
            name: name.to_string(),
            avatar,
            has_pin,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn update_profile(&self, id: &str, input: UpdateProfileInput) -> Result<Profile> {
        let existing = self.get_profile(id)?.context("Profile not found")?;
        let name = input.name.map(|n| n.trim().to_string()).unwrap_or(existing.name);
        if name.is_empty() {
            bail!("Profile name cannot be empty");
        }
        let avatar = input.avatar.unwrap_or(existing.avatar);
        let now = timestamp_ms();

        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;

        if input.remove_pin == Some(true) {
            connection.execute(
                "UPDATE profiles SET name = ?1, avatar = ?2, pin_salt = NULL, pin_hash = NULL, updated_at = ?3 WHERE id = ?4",
                params![name, avatar, now, id],
            )?;
        } else if let Some(pin) = input.pin {
            let pin = pin.trim();
            if pin.is_empty() {
                connection.execute(
                    "UPDATE profiles SET name = ?1, avatar = ?2, pin_salt = NULL, pin_hash = NULL, updated_at = ?3 WHERE id = ?4",
                    params![name, avatar, now, id],
                )?;
            } else {
                let salt = uuid::Uuid::new_v4().simple().to_string();
                let hash = hash_pin(pin, &salt);
                connection.execute(
                    "UPDATE profiles SET name = ?1, avatar = ?2, pin_salt = ?3, pin_hash = ?4, updated_at = ?5 WHERE id = ?6",
                    params![name, avatar, salt, hash, now, id],
                )?;
            }
        } else {
            connection.execute(
                "UPDATE profiles SET name = ?1, avatar = ?2, updated_at = ?3 WHERE id = ?4",
                params![name, avatar, now, id],
            )?;
        }
        drop(connection);

        let updated = self.get_profile(id)?.context("Profile not found after update")?;
        Ok(updated)
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        let profiles = self.list_profiles()?;
        if profiles.len() <= 1 {
            bail!("Cannot delete the only remaining profile");
        }
        if !profiles.iter().any(|p| p.id == id) {
            bail!("Profile not found");
        }

        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;

        let prefix = format!("{id}::");
        connection.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'library' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default'))",
            params![&prefix, id],
        )?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'history' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default'))",
            params![&prefix, id],
        )?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'search_history' AND substr(id, 1, length(?1)) = ?1",
            [&prefix],
        )?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'subscription_snapshot' AND substr(id, 1, length(?1)) = ?1",
            [&prefix],
        )?;
        connection.execute(
            "DELETE FROM watch_progress WHERE profile_id = ?1",
            [id],
        )?;

        drop(connection);
        if let Ok(mut settings) = self.get_settings() {
            let mut changed = false;
            if settings.profiles.remove(id).is_some() {
                changed = true;
            }
            if settings.global.default_profile_id.as_deref() == Some(id) {
                settings.global.default_profile_id = None;
                changed = true;
            }
            if changed {
                let _ = self.put("settings", "app", &settings);
            }
        }

        Ok(())
    }

    pub fn verify_profile_pin(&self, id: &str, pin: &str) -> Result<bool> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT pin_salt, pin_hash FROM profiles WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], |row| {
            let salt: Option<String> = row.get(0)?;
            let hash: Option<String> = row.get(1)?;
            Ok((salt, hash))
        })?;
        if let Some(res) = rows.next() {
            let (salt, hash) = res?;
            match (salt, hash) {
                (Some(salt), Some(expected_hash)) => {
                    let computed = hash_pin(pin.trim(), &salt);
                    Ok(computed == expected_hash)
                }
                _ => Ok(true),
            }
        } else {
            bail!("Profile not found");
        }
    }

    pub fn set_profile_pin(&self, id: &str, pin: Option<&str>) -> Result<()> {
        let _ = self.get_profile(id)?.context("Profile not found")?;
        let (pin_salt, pin_hash) = match pin {
            Some(p) if !p.trim().is_empty() => {
                let salt = uuid::Uuid::new_v4().simple().to_string();
                let hash = hash_pin(p.trim(), &salt);
                (Some(salt), Some(hash))
            }
            _ => (None, None),
        };
        let now = timestamp_ms();
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "UPDATE profiles SET pin_salt = ?1, pin_hash = ?2, updated_at = ?3 WHERE id = ?4",
            params![pin_salt, pin_hash, now, id],
        )?;
        Ok(())
    }

    pub fn library(&self, profile_id: Option<&str>) -> Result<Vec<LibraryItem>> {
        let pid = profile_id.unwrap_or("default");
        let prefix = format!("{pid}::");
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT value FROM records WHERE kind = 'library' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default')) ORDER BY updated DESC, rowid DESC",
        )?;
        let rows = stmt.query_map(params![&prefix, pid], |row| row.get::<_, String>(0))?;
        let mut items = Vec::new();
        for r in rows {
            let s = r?;
            if let Ok(item) = serde_json::from_str::<LibraryItem>(&s) {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub fn get_library_item(&self, id: &str, profile_id: Option<&str>) -> Result<Option<LibraryItem>> {
        let pid = profile_id.unwrap_or("default");
        let scoped_key = format!("{pid}::{id}");
        if let Some(item) = self.get::<LibraryItem>("library", &scoped_key)? {
            return Ok(Some(item));
        }
        if pid == "default" {
            if let Some(item) = self.get::<LibraryItem>("library", id)? {
                return Ok(Some(item));
            }
        }
        if let Some(resolved_id) = self.resolve_alias(id, "media")? {
            let scoped_resolved = format!("{pid}::{resolved_id}");
            if let Some(item) = self.get::<LibraryItem>("library", &scoped_resolved)? {
                return Ok(Some(item));
            }
            if pid == "default" {
                return self.get::<LibraryItem>("library", &resolved_id);
            }
        }
        Ok(None)
    }

    pub fn set_library_status(
        &self,
        mut item: MediaItem,
        status: WatchStatus,
        profile_id: Option<&str>,
    ) -> Result<LibraryItem> {
        let pid = profile_id.unwrap_or("default");
        let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
        item.media_id = Some(media_id.clone());
        let legacy_id = crate::repository::plugin_id(&item.provider_id, &item.url);
        let now_ms = timestamp_ms();

        let mut entry = self
            .get_library_item(&media_id, Some(pid))?
            .unwrap_or_else(|| LibraryItem {
                id: media_id.clone(),
                item: item.clone(),
                saved_at: now_ms,
                watch_status: status,
                is_favorite: false,
                is_subscribed: false,
                profile_id: Some(pid.to_string()),
                updated_at: Some(now_ms),
                extra: std::collections::BTreeMap::new(),
            });

        entry.watch_status = status;
        entry.item = item;
        entry.updated_at = Some(now_ms);
        entry.profile_id = Some(pid.to_string());

        let scoped_key = format!("{pid}::{media_id}");
        self.put("library", &scoped_key, &entry)?;
        self.put_alias(&legacy_id, "media", &media_id)?;
        Ok(entry)
    }

    pub fn set_library_favorite(
        &self,
        mut item: MediaItem,
        is_favorite: bool,
        profile_id: Option<&str>,
    ) -> Result<LibraryItem> {
        let pid = profile_id.unwrap_or("default");
        let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
        item.media_id = Some(media_id.clone());
        let legacy_id = crate::repository::plugin_id(&item.provider_id, &item.url);
        let now_ms = timestamp_ms();

        let mut entry = self
            .get_library_item(&media_id, Some(pid))?
            .unwrap_or_else(|| LibraryItem {
                id: media_id.clone(),
                item: item.clone(),
                saved_at: now_ms,
                watch_status: WatchStatus::None,
                is_favorite,
                is_subscribed: false,
                profile_id: Some(pid.to_string()),
                updated_at: Some(now_ms),
                extra: std::collections::BTreeMap::new(),
            });

        entry.is_favorite = is_favorite;
        entry.item = item;
        entry.updated_at = Some(now_ms);
        entry.profile_id = Some(pid.to_string());

        let scoped_key = format!("{pid}::{media_id}");
        self.put("library", &scoped_key, &entry)?;
        self.put_alias(&legacy_id, "media", &media_id)?;
        Ok(entry)
    }

    pub fn set_library_subscribed(
        &self,
        mut item: MediaItem,
        is_subscribed: bool,
        profile_id: Option<&str>,
    ) -> Result<LibraryItem> {
        let pid = profile_id.unwrap_or("default");
        let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
        item.media_id = Some(media_id.clone());
        let legacy_id = crate::repository::plugin_id(&item.provider_id, &item.url);
        let now_ms = timestamp_ms();
        let scoped_key = format!("{pid}::{media_id}");
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let profile_exists = tx.query_row("SELECT 1 FROM profiles WHERE id=?1", [pid], |_| Ok(())).optional()?.is_some();
        if !profile_exists { bail!("Profile not found"); }
        let current: Option<String> = tx.query_row(
            "SELECT value FROM records WHERE kind='library' AND id=?1", [&scoped_key], |row| row.get(0)
        ).optional()?;
        let current = if current.is_none() && pid == "default" {
            tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&media_id], |row| row.get(0)).optional()?
        } else { current };
        let mut entry: LibraryItem = match current {
            Some(value) => serde_json::from_str(&value).context("Stored library item could not be read")?,
            None => LibraryItem {
                id: media_id.clone(), item: item.clone(), saved_at: now_ms,
                watch_status: WatchStatus::None, is_favorite: false, is_subscribed: false,
                profile_id: Some(pid.to_string()), updated_at: Some(now_ms), extra: Default::default(),
            },
        };
        let was_subscribed = entry.is_subscribed;
        entry.is_subscribed = is_subscribed;
        entry.item = item;
        entry.updated_at = Some(now_ms);
        entry.profile_id = Some(pid.to_string());
        if is_subscribed {
            if !was_subscribed || !entry.extra.get("subscriptionGeneration").is_some_and(|v| v.is_string()) {
                entry.extra.insert("subscriptionGeneration".into(), serde_json::json!(uuid::Uuid::new_v4().to_string()));
            }
        } else {
            entry.extra.remove("subscriptionGeneration");
            tx.execute("DELETE FROM records WHERE kind='subscription_snapshot' AND id=?1", [&scoped_key])?;
        }
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('library',?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
            params![&scoped_key, serde_json::to_string(&entry)?, timestamp()])?;
        tx.execute("INSERT INTO aliases(alias_key,target_kind,target_id,created_at) VALUES (?1,'media',?2,?3) ON CONFLICT(alias_key) DO UPDATE SET target_kind=excluded.target_kind,target_id=excluded.target_id",
            params![legacy_id, &media_id, now_ms])?;
        tx.commit()?;
        Ok(entry)
    }

    /// Upgrade an older subscribed record lazily, without re-subscribing an item that was
    /// removed while a check was being scheduled.
    pub fn ensure_subscription_generation(&self, profile_id: &str, media_id: &str) -> Result<Option<String>> {
        let key = format!("{profile_id}::{media_id}");
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(None) };
        let mut entry: LibraryItem = serde_json::from_str(&raw)?;
        if !entry.is_subscribed { return Ok(None); }
        let generation = match entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()).filter(|v| !v.is_empty()) {
            Some(value) => value.to_owned(),
            None => {
                let value = uuid::Uuid::new_v4().to_string();
                entry.extra.insert("subscriptionGeneration".into(), serde_json::json!(value));
                tx.execute("UPDATE records SET value=?1 WHERE kind='library' AND id=?2", params![serde_json::to_string(&entry)?, &key])?;
                value
            }
        };
        tx.commit()?;
        Ok(Some(generation))
    }

    pub fn subscription_is_current(&self, profile_id: &str, media_id: &str, generation: &str) -> Result<bool> {
        let key = format!("{profile_id}::{media_id}");
        let item: Option<LibraryItem> = self.get("library", &key)?;
        let Some(entry) = item else { return Ok(false) };
        if !entry.is_subscribed || entry.extra.get("subscriptionGeneration").and_then(|value| value.as_str()) != Some(generation) { return Ok(false); }
        self.provider_is_enabled(&entry.item.provider_id)
    }

    /// Atomically merge one successful provider observation if this exact subscription still
    /// exists. The generation guards unsubscribe/re-subscribe and profile deletion races.
    pub fn apply_subscription_snapshot(
        &self, profile_id: &str, media_id: &str, generation: &str,
        observed: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
        checked_at_ms: u64, record_notifications: bool,
    ) -> Result<Option<crate::subscriptions::SubscriptionChanges>> {
        if observed.is_empty() || observed.values().all(|set| set.is_empty()) { bail!("Cannot save an empty subscription observation"); }
        let key = format!("{profile_id}::{media_id}");
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let value: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(value) = value else { return Ok(None) };
        let entry: LibraryItem = serde_json::from_str(&value)?;
        if !entry.is_subscribed || entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()) != Some(generation) || !provider_enabled_in(&tx, &entry.item.provider_id)? { return Ok(None); }
        let settings_raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |row| row.get(0)).optional()?;
        let notifications_enabled = settings_raw.map(|raw| serde_json::from_str::<Settings>(&raw)).transpose()?.unwrap_or_default().global.subscription_notifications;
        let old: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='subscription_snapshot' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let previous: Option<crate::subscriptions::SubscriptionSnapshot> = old.map(|v| serde_json::from_str(&v)).transpose()?;
        let mut snapshot = previous.filter(|old| old.generation == generation).unwrap_or_else(|| crate::subscriptions::SubscriptionSnapshot { generation: generation.into(), observed: Default::default(), pending: Default::default(), unread: Default::default(), checked_at_ms });
        let first_check = snapshot.observed.is_empty();
        let mut new_episodes = std::collections::BTreeMap::new();
        for (group, keys) in observed {
            let known = snapshot.observed.entry(group.clone()).or_default();
            let added: Vec<_> = keys.difference(known).cloned().collect();
            known.extend(keys);
            if !first_check && !added.is_empty() {
                if record_notifications && notifications_enabled { snapshot.pending.entry(group.clone()).or_default().extend(added.iter().cloned()); }
                snapshot.unread.entry(group.clone()).or_default().extend(added.iter().cloned());
                new_episodes.insert(group, added);
            }
        }
        snapshot.checked_at_ms = checked_at_ms;
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('subscription_snapshot',?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
            params![&key, serde_json::to_string(&snapshot)?, checked_at_ms])?;
        tx.commit()?;
        Ok(Some(crate::subscriptions::SubscriptionChanges { first_check, new_episodes }))
    }

    /// Read unread changes without clearing them; the UI explicitly acknowledges what it shows.
    pub fn subscription_unread(&self, profile_id: &str, media_id: &str, generation: &str) -> Result<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> {
        let key = format!("{profile_id}::{media_id}");
        let connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let library: Option<String> = connection.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |r| r.get(0)).optional()?;
        let Some(library) = library else { return Ok(Default::default()) };
        let entry: LibraryItem = serde_json::from_str(&library)?;
        if !entry.is_subscribed || entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()) != Some(generation) { return Ok(Default::default()); }
        let raw: Option<String> = connection.query_row("SELECT value FROM records WHERE kind='subscription_snapshot' AND id=?1", [&key], |r| r.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(Default::default()) };
        let snapshot: crate::subscriptions::SubscriptionSnapshot = serde_json::from_str(&raw)?;
        if snapshot.generation != generation { return Ok(Default::default()); }
        Ok(snapshot.unread)
    }

    /// Acknowledge only identities actually shown; newer arrivals remain unread.
    pub fn acknowledge_subscription_unread(&self, profile_id: &str, update: &crate::subscriptions::SubscriptionUpdate) -> Result<()> {
        let key = format!("{profile_id}::{}", update.media_id);
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let library: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |r| r.get(0)).optional()?;
        let Some(library) = library else { return Ok(()) };
        let entry: LibraryItem = serde_json::from_str(&library)?;
        if !entry.is_subscribed || entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()) != Some(update.generation.as_str()) { return Ok(()); }
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='subscription_snapshot' AND id=?1", [&key], |r| r.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(()) };
        let mut snapshot: crate::subscriptions::SubscriptionSnapshot = serde_json::from_str(&raw)?;
        if snapshot.generation != update.generation { return Ok(()); }
        for (group, episodes) in &update.episodes {
            if let Some(unread) = snapshot.unread.get_mut(group) { for episode in episodes { unread.remove(episode); } }
        }
        snapshot.unread.retain(|_, episodes| !episodes.is_empty());
        tx.execute("UPDATE records SET value=?1 WHERE kind='subscription_snapshot' AND id=?2", params![serde_json::to_string(&snapshot)?, &key])?;
        tx.commit()?;
        Ok(())
    }

    /// Claim a pending batch durably before notifying the desktop. This prevents a restart
    /// after delivery from repeating the same alert; failed sends can be restored below.
    pub fn claim_subscription_alert(&self, profile_id: &str, media_id: &str) -> Result<Option<crate::subscriptions::SubscriptionAlert>> {
        let key = format!("{profile_id}::{media_id}");
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let settings: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |row| row.get(0)).optional()?;
        let enabled = settings.map(|s| serde_json::from_str::<Settings>(&s)).transpose()?.unwrap_or_default().global.subscription_notifications;
        if !enabled { return Ok(None); }
        let library: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(library) = library else { return Ok(None) };
        let entry: LibraryItem = serde_json::from_str(&library)?;
        let Some(generation) = entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()) else { return Ok(None) };
        if !entry.is_subscribed { return Ok(None); }
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='subscription_snapshot' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(None) };
        let mut snapshot: crate::subscriptions::SubscriptionSnapshot = serde_json::from_str(&raw)?;
        if snapshot.generation != generation || snapshot.pending.values().all(|v| v.is_empty()) { return Ok(None); }
        let episodes = std::mem::take(&mut snapshot.pending);
        tx.execute("UPDATE records SET value=?1 WHERE kind='subscription_snapshot' AND id=?2", params![serde_json::to_string(&snapshot)?, &key])?;
        tx.commit()?;
        Ok(Some(crate::subscriptions::SubscriptionAlert { profile_id: profile_id.into(), media_id: media_id.into(), generation: generation.into(), title: entry.item.name, episodes }))
    }

    /// Restore a failed send only if the same subscription and opt-in still exist.
    pub fn restore_subscription_alert(&self, alert: &crate::subscriptions::SubscriptionAlert) -> Result<()> {
        let key = format!("{}::{}", alert.profile_id, alert.media_id);
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let settings: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |row| row.get(0)).optional()?;
        if !settings.map(|s| serde_json::from_str::<Settings>(&s)).transpose()?.unwrap_or_default().global.subscription_notifications { return Ok(()); }
        let library: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='library' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(library) = library else { return Ok(()) };
        let entry: LibraryItem = serde_json::from_str(&library)?;
        if !entry.is_subscribed || entry.extra.get("subscriptionGeneration").and_then(|v| v.as_str()) != Some(alert.generation.as_str()) { return Ok(()); }
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='subscription_snapshot' AND id=?1", [&key], |row| row.get(0)).optional()?;
        let Some(raw) = raw else { return Ok(()) };
        let mut snapshot: crate::subscriptions::SubscriptionSnapshot = serde_json::from_str(&raw)?;
        if snapshot.generation != alert.generation { return Ok(()); }
        for (group, episodes) in &alert.episodes { snapshot.pending.entry(group.clone()).or_default().extend(episodes.iter().cloned()); }
        tx.execute("UPDATE records SET value=?1 WHERE kind='subscription_snapshot' AND id=?2", params![serde_json::to_string(&snapshot)?, &key])?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_library_item(&self, id: &str, profile_id: Option<&str>) -> Result<()> {
        let pid = profile_id.unwrap_or("default");
        let resolved = self.resolve_alias(id, "media")?.unwrap_or_else(|| id.to_string());
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        for value in [resolved.as_str(), id] {
            let key = format!("{pid}::{value}");
            tx.execute("DELETE FROM records WHERE kind IN ('library','subscription_snapshot') AND id=?1", [&key])?;
            if pid == "default" { tx.execute("DELETE FROM records WHERE kind='library' AND id=?1", [value])?; }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn clear_library(&self, status: Option<WatchStatus>, profile_id: Option<&str>) -> Result<()> {
        let pid = profile_id.unwrap_or("default");
        match status {
            None => {
                let prefix = format!("{pid}::");
                let connection = self
                    .connection
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
                connection.execute(
                    "DELETE FROM records WHERE kind = 'library' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default'))",
                    params![&prefix, pid],
                )?;
                connection.execute("DELETE FROM records WHERE kind = 'subscription_snapshot' AND substr(id, 1, length(?1)) = ?1", [&prefix])?;
                Ok(())
            }
            Some(target_status) => {
                let items: Vec<LibraryItem> = self.library(Some(pid))?;
                for mut item in items {
                    if item.watch_status == target_status {
                        if !item.is_favorite && !item.is_subscribed {
                            self.remove_library_item(&item.id, Some(pid))?;
                        } else {
                            item.watch_status = WatchStatus::None;
                            item.updated_at = Some(timestamp_ms());
                            let scoped_key = format!("{pid}::{}", item.id);
                            self.put("library", &scoped_key, &item)?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    pub fn history(&self, profile_id: Option<&str>) -> Result<Vec<WatchEntry>> {
        let pid = profile_id.unwrap_or("default");
        let prefix = format!("{pid}::");
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT value FROM records WHERE kind = 'history' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default')) ORDER BY updated DESC, rowid DESC",
        )?;
        let rows = stmt.query_map(params![&prefix, pid], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::new();
        for r in rows {
            let s = r?;
            if let Ok(entry) = serde_json::from_str::<WatchEntry>(&s) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn remove_history_item(&self, id: &str, profile_id: Option<&str>) -> Result<()> {
        let pid = profile_id.unwrap_or("default");
        let resolved = self.resolve_alias(id, "episode")?.unwrap_or_else(|| id.to_string());
        let scoped_key = format!("{pid}::{resolved}");
        self.remove("history", &scoped_key)?;
        if resolved != id {
            let scoped_alias = format!("{pid}::{id}");
            self.remove("history", &scoped_alias)?;
        }
        if pid == "default" {
            self.remove("history", &resolved)?;
            if resolved != id {
                self.remove("history", id)?;
            }
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "DELETE FROM watch_progress WHERE profile_id = ?1 AND (episode_id = ?2 OR episode_id = ?3)",
            params![pid, resolved, id],
        )?;
        Ok(())
    }

    pub fn clear_history(&self, profile_id: Option<&str>) -> Result<()> {
        let pid = profile_id.unwrap_or("default");
        let prefix = format!("{pid}::");
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        connection.execute(
            "DELETE FROM records WHERE kind = 'history' AND (substr(id, 1, length(?1)) = ?1 OR (id NOT LIKE '%::%' AND ?2 = 'default'))",
            params![&prefix, pid],
        )?;
        connection.execute(
            "DELETE FROM watch_progress WHERE profile_id = ?1",
            [pid],
        )?;
        Ok(())
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
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction()?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |r| r.get(0)).optional()?;
        let mut settings: Settings = raw.map(|value| serde_json::from_str(&value)).transpose()?.unwrap_or_default();
        let notifications_were_enabled = settings.global.subscription_notifications;
        apply_settings_patch(&mut settings, &patch, scope, profile_id)?;
        settings.revision = settings.revision.saturating_add(1);
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('settings','app',?1,?2) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated", params![serde_json::to_string(&settings)?, timestamp()])?;
        if notifications_were_enabled && !settings.global.subscription_notifications {
            let pending: Vec<(String, String)> = {
                let mut stmt = tx.prepare("SELECT id,value FROM records WHERE kind='subscription_snapshot'")?;
                let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
                rows.collect::<std::result::Result<_, _>>()?
            };
            for (id, raw) in pending {
                let mut snapshot: crate::subscriptions::SubscriptionSnapshot = serde_json::from_str(&raw)?;
                if !snapshot.pending.is_empty() {
                    snapshot.pending.clear();
                    tx.execute("UPDATE records SET value=?1 WHERE kind='subscription_snapshot' AND id=?2", params![serde_json::to_string(&snapshot)?, id])?;
                }
            }
        }
        tx.commit()?;
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

    /// Enqueue is idempotent per profile/media/episode, regardless of destination or mirror.
    /// Provider payloads remain in the existing episode store, not in the visible queue record.
    pub fn enqueue_download(&self, profile_id: &str, mut request: crate::download_queue::DownloadRequest) -> Result<crate::download_queue::DownloadJob> {
        use crate::{download_queue::{DownloadJob, DownloadMetadata, DownloadState}, downloads::{DestinationIdentity, supported_direct_extension}};
        if request.item.provider_id.trim().is_empty() || request.item.url.trim().is_empty() || request.episode.data.trim().is_empty() { bail!("The episode has no provider or playable payload"); }
        if !supported_direct_extension(&request.extension) && request.extension != crate::downloads::HLS_EXTENSION { bail!("Unsupported download format"); }
        for name in [&request.policy.source_name, &request.policy.stream_name].into_iter().flatten() {
            if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) { bail!("Invalid preferred source name"); }
        }
        let destination = std::fs::canonicalize(&request.destination).context("Download folder is unavailable")?;
        let destination_identity = DestinationIdentity::from_path(&destination)?;
        let media_id = compute_media_id(&request.item.provider_id, &request.item.url, request.item.unique_url.as_deref());
        let episode_id = match request.episode.id.as_deref() {
            Some(id) if id.starts_with(&format!("{media_id}::")) => id.to_owned(),
            _ => compute_episode_id(&media_id, &request.episode, request.episode.dub_status, request.episode.index.and_then(|n| usize::try_from(n).ok())),
        };
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT 1 FROM profiles WHERE id=?1", [profile_id], |_| Ok(())).optional()?.is_none() { bail!("Profile not found"); }
        let jobs: Vec<DownloadJob> = {
            let mut statement = tx.prepare("SELECT value FROM records WHERE kind='download_job'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?
        };
        // Refresh the resolver payload even for an idempotent enqueue; links may have expired.
        tx.execute("INSERT INTO episode_payloads(episode_id,media_id,data,updated_at) VALUES (?1,?2,?3,?4) ON CONFLICT(episode_id) DO UPDATE SET media_id=excluded.media_id,data=excluded.data,updated_at=excluded.updated_at",
            params![&episode_id, &media_id, &request.episode.data, timestamp_ms()])?;
        let mut item = request.item.clone();
        item.media_id = Some(media_id.clone());
        let mut episode = request.episode.clone();
        episode.id = Some(episode_id.clone());
        episode.data.clear();
        let metadata = serde_json::to_string(&DownloadMetadata { item, episode })?;
        if let Some(existing) = jobs.iter().find(|job| job.profile_id == profile_id && job.media_id == media_id && job.episode_id == episode_id) {
            let existing = existing.clone();
            tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('download_metadata',?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated", params![&existing.id, &metadata, timestamp()])?;
            tx.commit()?; return Ok(existing);
        }
        if jobs.len() >= 10_000 { bail!("The download queue has reached its limit"); }
        let order = jobs.iter().map(|job| job.order).max().unwrap_or(0).checked_add(1).context("Download queue order overflow")?;
        let now = timestamp_ms();
        let job = DownloadJob {
            id: uuid::Uuid::new_v4().to_string(), profile_id: profile_id.into(), media_id, episode_id,
            provider_id: std::mem::take(&mut request.item.provider_id), title: std::mem::take(&mut request.item.name),
            episode_name: std::mem::take(&mut request.episode.name), season: request.episode.season, episode: request.episode.episode,
            destination, destination_identity, extension: request.extension, policy: request.policy,
            state: DownloadState::Queued, bytes: 0, total: None, output: None, error: None, order,
            revision: 1, repair_of: None, created_at_ms: now, updated_at_ms: now,
        };
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('download_job',?1,?2,?3)", params![&job.id, serde_json::to_string(&job)?, timestamp()])?;
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('download_metadata',?1,?2,?3)", params![&job.id, metadata, timestamp()])?;
        tx.commit()?;
        Ok(job)
    }

    pub fn download_jobs(&self, profile_id: Option<&str>) -> Result<Vec<crate::download_queue::DownloadJob>> {
        let mut jobs: Vec<crate::download_queue::DownloadJob> = self.list("download_job")?;
        if let Some(profile) = profile_id { jobs.retain(|job| job.profile_id == profile); }
        jobs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        Ok(jobs)
    }

    /// Requires the app-wide lease so a second process cannot reset running jobs.
    pub fn recover_download_queue(&self, lease: &crate::download_queue::QueueLease) -> Result<usize> {
        use crate::download_queue::{DownloadJob, DownloadState};
        let mut recovered_once = lease.recovered.lock().map_err(|_| anyhow::anyhow!("Download queue lock failed"))?;
        if *recovered_once { bail!("Download queue recovery has already run"); }
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        validate_download_lease(&connection, lease)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let jobs: Vec<DownloadJob> = {
            let mut statement = tx.prepare("SELECT value FROM records WHERE kind='download_job'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?
        };
        let mut recovered = 0;
        for mut job in jobs.into_iter().filter(|job| job.state.active()) {
            let profile_exists = tx.query_row("SELECT 1 FROM profiles WHERE id=?1", [&job.profile_id], |_| Ok(())).optional()?.is_some();
            job.state = if profile_exists { DownloadState::Queued } else { DownloadState::Paused }; job.revision = job.revision.checked_add(1).context("Download revision overflow")?;
            job.updated_at_ms = timestamp_ms(); job.error = None;
            tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), &job.id])?;
            recovered += 1;
        }
        tx.commit()?;
        *recovered_once = true;
        Ok(recovered)
    }

    /// Atomically reserve the next job while enforcing a single global concurrency limit.
    pub fn claim_next_download(&self, lease: &crate::download_queue::QueueLease, max_active: usize) -> Result<Option<crate::download_queue::DownloadJob>> {
        use crate::download_queue::{DownloadJob, DownloadState};
        if !(1..=4).contains(&max_active) { bail!("Download concurrency must be between 1 and 4"); }
        let recovered = lease.recovered.lock().map_err(|_| anyhow::anyhow!("Download queue lock failed"))?;
        if !*recovered { bail!("Recover the download queue before claiming jobs"); }
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        validate_download_lease(&connection, lease)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut jobs: Vec<DownloadJob> = {
            let mut statement = tx.prepare("SELECT value FROM records WHERE kind='download_job'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?
        };
        if jobs.iter().filter(|job| job.state.active()).count() >= max_active { return Ok(None); }
        jobs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        let mut eligible = None;
        for job in jobs.into_iter().filter(|job| job.state == DownloadState::Queued) {
            if tx.query_row("SELECT 1 FROM profiles WHERE id=?1", [&job.profile_id], |_| Ok(())).optional()?.is_some() {
                eligible = Some(job); break;
            }
        }
        let Some(mut job) = eligible else { return Ok(None) };
        job.state = DownloadState::Resolving; job.revision = job.revision.checked_add(1).context("Download revision overflow")?;
        job.updated_at_ms = timestamp_ms(); job.error = None;
        tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), &job.id])?;
        tx.commit()?;
        Ok(Some(job))
    }

    /// Remove a finished, stopped or failed job's record. Active jobs must be paused or cancelled
    /// first; the state is re-checked inside the transaction so a claim cannot race the removal.
    pub fn delete_download_job(&self, id: &str, profile_id: &str) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::DownloadJob;
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let job: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if job.profile_id != profile_id { bail!("Download job not found"); }
        if job.state.active() { bail!("Pause or cancel the download before removing it"); }
        tx.execute("DELETE FROM records WHERE kind='download_job' AND id=?1", [id])?;
        tx.execute("DELETE FROM records WHERE kind='download_metadata' AND id=?1", [id])?;
        tx.commit()?;
        Ok(job)
    }

    pub fn get_download_job(&self, id: &str) -> Result<Option<crate::download_queue::DownloadJob>> {
        self.get("download_job", id)
    }

    /// Queue a separate replacement copy without touching the damaged output. Existing repairs
    /// are returned instead of creating duplicate jobs when the user clicks more than once.
    pub fn enqueue_download_repair(&self, id: &str, profile_id: &str) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::{DownloadJob, DownloadState};
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let original: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if original.profile_id != profile_id { bail!("Download job not found"); }
        if original.state != DownloadState::Completed { bail!("Only finished downloads can be repaired"); }
        if crate::downloads::DestinationIdentity::from_path(&original.destination)? != original.destination_identity { bail!("The download folder has changed. Choose it again"); }
        if tx.query_row("SELECT 1 FROM episode_payloads WHERE episode_id=?1", [&original.episode_id], |_| Ok(())).optional()?.is_none() {
            bail!("The source information for this episode is unavailable");
        }
        let jobs: Vec<DownloadJob> = {
            let mut statement = tx.prepare("SELECT value FROM records WHERE kind='download_job'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?
        };
        if let Some(existing) = jobs.iter().find(|job| job.profile_id == profile_id && job.repair_of.as_deref() == Some(id) && job.state != DownloadState::Canceled) {
            let existing = existing.clone(); tx.commit()?; return Ok(existing);
        }
        if jobs.len() >= 10_000 { bail!("The download queue has reached its limit"); }
        let mut repair = original.clone();
        repair.id = uuid::Uuid::new_v4().to_string();
        repair.repair_of = Some(id.into());
        repair.order = jobs.iter().map(|job| job.order).max().unwrap_or(0).checked_add(1).context("Download queue order overflow")?;
        repair.state = DownloadState::Queued; repair.bytes = 0; repair.total = None; repair.output = None; repair.error = None;
        repair.revision = 1; repair.created_at_ms = timestamp_ms(); repair.updated_at_ms = repair.created_at_ms;
        tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('download_job',?1,?2,?3)", params![&repair.id, serde_json::to_string(&repair)?, timestamp()])?;
        tx.execute("INSERT INTO records(kind,id,value,updated) SELECT 'download_metadata',?1,value,?2 FROM records WHERE kind='download_metadata' AND id=?3", params![&repair.id, timestamp(), id])?;
        tx.commit()?;
        Ok(repair)
    }

    /// User actions read and transition under one transaction, so progress cannot cause a
    /// spurious compare-and-set failure while a pause or cancel is being requested.
    pub fn control_download(&self, id: &str, profile_id: &str, action: crate::download_queue::DownloadAction) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::{DownloadAction, DownloadJob, DownloadState};
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let mut job: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if job.profile_id != profile_id { bail!("Download job not found"); }
        let next = match (action, job.state) {
            (DownloadAction::Pause, DownloadState::Queued | DownloadState::Resolving | DownloadState::Downloading) => DownloadState::Paused,
            (DownloadAction::Cancel, DownloadState::Queued | DownloadState::Resolving | DownloadState::Downloading | DownloadState::Paused | DownloadState::Failed) => DownloadState::Canceled,
            (DownloadAction::Resume, DownloadState::Paused) | (DownloadAction::Retry, DownloadState::Failed | DownloadState::Canceled) => DownloadState::Queued,
            (DownloadAction::Pause, DownloadState::Paused) | (DownloadAction::Cancel, DownloadState::Canceled) => return Ok(job),
            _ => bail!("That action is not available for this download"),
        };
        job.state = next; job.error = None; job.revision = job.revision.checked_add(1).context("Download revision overflow")?; job.updated_at_ms = timestamp_ms();
        tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), id])?;
        tx.commit()?;
        Ok(job)
    }

    /// Compare-and-set prevents a late worker event from reversing pause, cancel or retry.
    pub fn transition_download(&self, id: &str, profile_id: &str, expected_revision: u64,
        next: crate::download_queue::DownloadState, error: Option<&str>, output: Option<&Path>, bytes: u64, total: Option<u64>) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::{DownloadJob, DownloadState};
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let mut job: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if job.profile_id != profile_id { bail!("Download job not found"); }
        if job.revision != expected_revision { bail!("Download changed; refresh the queue"); }
        if !job.state.permits(next) { bail!("Invalid download state transition"); }
        if total.is_some_and(|total| bytes > total) { bail!("Invalid download progress"); }
        if error.is_some_and(|message| message.len() > 512 || message.chars().any(char::is_control) || message.contains("://")) { bail!("Invalid download error message"); }
        if next == DownloadState::Failed && error.is_none_or(|message| message.trim().is_empty()) { bail!("A failed download needs a reason"); }
        if next == DownloadState::Completed {
            let expected = job.destination.join(format!("cloudstream-{}.{}", job.id, job.extension));
            if output != Some(expected.as_path()) { bail!("Unexpected download output path"); }
            if crate::downloads::DestinationIdentity::from_path(&job.destination)? != job.destination_identity { bail!("The download folder has changed"); }
            if job.extension == crate::downloads::HLS_EXTENSION {
                if !crate::hls_download::verify_completed_folder(&job.destination, job.destination_identity, &job.id, bytes)? { bail!("Downloaded folder was not published by this job or is incomplete"); }
            } else {
                let metadata = std::fs::symlink_metadata(&expected).context("Downloaded file is unavailable")?;
                if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != bytes || bytes == 0 { bail!("Downloaded file does not match the completed job"); }
                if !crate::downloads::verify_completed_file(&job.destination, job.destination_identity, &job.id, &job.extension, bytes)? { bail!("Downloaded file was not published by this job"); }
            }
            job.output = Some(expected);
        } else if output.is_some() { bail!("Only completed downloads have an output file"); }
        job.state = next; job.bytes = bytes; job.total = total;
        job.error = if next == DownloadState::Failed { error.map(str::to_owned) } else { None };
        job.revision = job.revision.checked_add(1).context("Download revision overflow")?; job.updated_at_ms = timestamp_ms();
        tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), id])?;
        tx.commit()?;
        Ok(job)
    }

    /// Reorder only the queued jobs of this profile, preserving other profiles' queue slots.
    pub fn reorder_downloads(&self, profile_id: &str, ids: &[String]) -> Result<()> {
        use crate::download_queue::{DownloadJob, DownloadState};
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut jobs: Vec<DownloadJob> = {
            let mut statement = tx.prepare("SELECT value FROM records WHERE kind='download_job'")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<_>>()?
        };
        jobs.retain(|job| job.profile_id == profile_id && job.state == DownloadState::Queued);
        jobs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        let expected: std::collections::HashSet<_> = jobs.iter().map(|job| job.id.as_str()).collect();
        let provided: std::collections::HashSet<_> = ids.iter().map(String::as_str).collect();
        if ids.len() != jobs.len() || provided.len() != ids.len() || expected != provided { bail!("The download queue changed; refresh before reordering"); }
        let slots: Vec<u64> = jobs.iter().map(|job| job.order).collect();
        let mut by_id: std::collections::HashMap<_, _> = jobs.into_iter().map(|job| (job.id.clone(), job)).collect();
        for (id, order) in ids.iter().zip(slots) {
            let mut job = by_id.remove(id).unwrap();
            if job.order == order { continue; }
            job.order = order; job.updated_at_ms = timestamp_ms(); job.revision = job.revision.checked_add(1).context("Download revision overflow")?;
            tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record the last durable partial length after a user stop, without undoing the stop.
    pub fn update_stopped_download_progress(&self, id: &str, profile_id: &str, expected_revision: u64, bytes: u64, total: Option<u64>) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::{DownloadJob, DownloadState};
        if total.is_some_and(|size| bytes > size) { bail!("Invalid download progress"); }
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let mut job: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if job.profile_id != profile_id || job.revision != expected_revision || !matches!(job.state, DownloadState::Paused | DownloadState::Canceled) { bail!("Download changed; refresh the queue"); }
        job.bytes = bytes; job.total = total; job.revision = job.revision.checked_add(1).context("Download revision overflow")?; job.updated_at_ms = timestamp_ms();
        tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), id])?;
        tx.commit()?;
        Ok(job)
    }

    pub fn update_download_progress(&self, id: &str, profile_id: &str, expected_revision: u64, bytes: u64, total: Option<u64>) -> Result<crate::download_queue::DownloadJob> {
        use crate::download_queue::{DownloadJob, DownloadState};
        if total.is_some_and(|size| bytes > size) { bail!("Invalid download progress"); }
        let mut connection = self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='download_job' AND id=?1", [id], |row| row.get(0)).optional()?;
        let mut job: DownloadJob = serde_json::from_str(&raw.context("Download job not found")?)?;
        if job.profile_id != profile_id { bail!("Download job not found"); }
        if job.revision != expected_revision || job.state != DownloadState::Downloading { bail!("Download changed; refresh the queue"); }
        job.bytes = bytes; job.total = total; job.revision = job.revision.checked_add(1).context("Download revision overflow")?; job.updated_at_ms = timestamp_ms();
        tx.execute("UPDATE records SET value=?1,updated=?2 WHERE kind='download_job' AND id=?3", params![serde_json::to_string(&job)?, timestamp(), id])?;
        tx.commit()?;
        Ok(job)
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
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;

        // Determine watched_override and is_watched:
        // If progress.watched_override is None, check if existing record has a manual override!
        let (watched_override_val, is_watched_val) = match progress.watched_override {
            Some(manual) => (Some(manual), manual),
            None => {
                let existing_override: Option<Option<i64>> = connection
                    .query_row(
                        "SELECT watched_override FROM watch_progress WHERE profile_id = ?1 AND episode_id = ?2",
                        params![progress.profile_id, progress.episode_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                match existing_override.flatten() {
                    Some(1) => (Some(true), true),
                    Some(0) => (Some(false), false),
                    _ => (None, progress.is_watched),
                }
            }
        };

        let override_int = watched_override_val.map(|v| if v { 1 } else { 0 });

        connection.execute(
            "INSERT OR REPLACE INTO watch_progress (profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched, completion_reason, extra, watched_override) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                progress.profile_id,
                progress.media_id,
                progress.episode_id,
                progress.position_seconds,
                progress.duration_seconds,
                progress.updated_at_ms,
                if is_watched_val { 1 } else { 0 },
                progress.completion_reason,
                extra_str,
                override_int
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
            "SELECT profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched, completion_reason, extra, watched_override FROM watch_progress WHERE profile_id = ?1 AND episode_id = ?2"
        )?;
        let mut rows = stmt.query([profile_id, episode_id])?;
        if let Some(row) = rows.next()? {
            let extra_str: String = row.get(8).unwrap_or_else(|_| "{}".into());
            let extra: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(&extra_str).unwrap_or_default();
            let override_opt: Option<i64> = row.get(9)?;
            let watched_override = override_opt.map(|v| v != 0);
            Ok(Some(WatchProgress {
                profile_id: row.get(0)?,
                media_id: row.get(1)?,
                episode_id: row.get(2)?,
                position_seconds: row.get(3)?,
                duration_seconds: row.get(4)?,
                updated_at_ms: row.get(5)?,
                is_watched: row.get::<_, i64>(6)? != 0,
                completion_reason: row.get(7)?,
                watched_override,
                extra,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_media_watch_progress(&self, profile_id: &str, media_id: &str) -> Result<Vec<WatchProgress>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Database lock failed"))?;
        let mut stmt = connection.prepare(
            "SELECT profile_id, media_id, episode_id, position_seconds, duration_seconds, updated_at_ms, is_watched, completion_reason, extra, watched_override FROM watch_progress WHERE profile_id = ?1 AND media_id = ?2 ORDER BY updated_at_ms DESC"
        )?;
        let rows = stmt.query_map([profile_id, media_id], |row| {
            let extra_str: String = row.get(8).unwrap_or_else(|_| "{}".into());
            let extra: std::collections::BTreeMap<String, serde_json::Value> = serde_json::from_str(&extra_str).unwrap_or_default();
            let override_opt: Option<i64> = row.get(9)?;
            let watched_override = override_opt.map(|v| v != 0);
            Ok(WatchProgress {
                profile_id: row.get(0)?,
                media_id: row.get(1)?,
                episode_id: row.get(2)?,
                position_seconds: row.get(3)?,
                duration_seconds: row.get(4)?,
                updated_at_ms: row.get(5)?,
                is_watched: row.get::<_, i64>(6)? != 0,
                completion_reason: row.get(7)?,
                watched_override,
                extra,
            })
        })?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn mark_episode_watched(
        &self,
        profile_id: &str,
        media_id: &str,
        episode_id: &str,
        is_watched: bool,
    ) -> Result<WatchProgress> {
        let now_ms = timestamp_ms();
        let existing = self.get_watch_progress(profile_id, episode_id)?;
        let mut progress = existing.unwrap_or_else(|| WatchProgress {
            profile_id: profile_id.to_string(),
            media_id: media_id.to_string(),
            episode_id: episode_id.to_string(),
            position_seconds: 0.0,
            duration_seconds: None,
            updated_at_ms: now_ms,
            is_watched,
            watched_override: Some(is_watched),
            completion_reason: if is_watched { Some("manual_mark".into()) } else { None },
            extra: std::collections::BTreeMap::new(),
        });

        progress.is_watched = is_watched;
        progress.watched_override = Some(is_watched);
        progress.updated_at_ms = now_ms;
        if is_watched {
            if progress.position_seconds == 0.0 {
                if let Some(dur) = progress.duration_seconds {
                    progress.position_seconds = dur;
                }
            }
            progress.completion_reason = Some("manual_mark".into());
        } else {
            progress.completion_reason = None;
        }

        self.save_watch_progress(&progress)?;
        Ok(progress)
    }

    pub fn mark_episodes_up_to(
        &self,
        profile_id: &str,
        media_id: &str,
        target_episode_id: &str,
        episodes: &[Episode],
    ) -> Result<Vec<WatchProgress>> {
        let mut enriched: Vec<(String, &Episode)> = Vec::with_capacity(episodes.len());
        let mut target_dub: Option<DubStatus> = None;
        let mut found_target = false;

        for (idx, ep) in episodes.iter().enumerate() {
            let ep_id = ep.id.clone().unwrap_or_else(|| {
                compute_episode_id(media_id, ep, ep.dub_status, ep.index.map(|i| i as usize).or(Some(idx)))
            });
            if ep_id == target_episode_id {
                target_dub = ep.dub_status;
                found_target = true;
            }
            enriched.push((ep_id, ep));
        }

        if !found_target {
            return Ok(Vec::new());
        }

        let mut matching_group: Vec<(String, &Episode)> = enriched
            .into_iter()
            .filter(|(_, ep)| ep.dub_status == target_dub)
            .collect();

        matching_group.sort_by_key(|(_, ep)| {
            (
                ep.season.unwrap_or(1),
                ep.episode.unwrap_or(0),
                ep.index.unwrap_or(0),
            )
        });

        let Some(pos) = matching_group
            .iter()
            .position(|(id, _)| id == target_episode_id)
        else {
            return Ok(Vec::new());
        };

        let mut updated = Vec::new();
        for (ep_id, _ep) in &matching_group[0..=pos] {
            let res = self.mark_episode_watched(profile_id, media_id, ep_id, true)?;
            updated.push(res);
        }

        Ok(updated)
    }

    pub fn save_watch_entry(&self, watch: &WatchEntry, profile_id: &str) -> Result<()> {
        let media_id = watch
            .item
            .media_id
            .clone()
            .unwrap_or_else(|| compute_media_id(&watch.item.provider_id, &watch.item.url, watch.item.unique_url.as_deref()));
        let episode_id = watch.id.clone();
        let scoped_history_id = format!("{profile_id}::{episode_id}");

        self.put("history", &scoped_history_id, watch)?;
        self.save_episode_payload(&episode_id, &media_id, &watch.episode.data)?;

        let legacy_data_alias = crate::repository::plugin_id(&watch.item.provider_id, &watch.episode.data);
        self.put_alias(&legacy_data_alias, "episode", &episode_id)?;

        let threshold = self.get_settings()
            .map(|s| s.global.playback.completion_threshold)
            .unwrap_or(0.90);
        let is_watched = watch.duration > 0.0 && watch.position >= threshold * watch.duration;
        let progress = WatchProgress {
            profile_id: profile_id.to_string(),
            media_id,
            episode_id,
            position_seconds: watch.position,
            duration_seconds: if watch.duration > 0.0 { Some(watch.duration) } else { None },
            updated_at_ms: watch.watched_at,
            is_watched,
            watched_override: None,
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
            && (scope == Some("global")
                || normalized_patch.get("appearance").is_some()
                || normalized_patch.get("playback").is_some())
        {
            let mut global_patch = serde_json::Map::new();
            let mut root_patch = serde_json::Map::new();

            if let Some(app) = normalized_patch.get("appearance") {
                global_patch.insert("appearance".to_string(), app.clone());
            }
            if let Some(pb) = normalized_patch.get("playback") {
                global_patch.insert("playback".to_string(), pb.clone());
            }

            if let Some(obj) = normalized_patch.as_object() {
                for (k, v) in obj {
                    if k == "appearance" || k == "posterDensity" || k == "playback" {
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
    updated.validate()?;
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
