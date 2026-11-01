use crate::models::*;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, Transaction};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

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

    pub fn repositories(&self) -> Result<Vec<Repository>> {
        self.list("repository")
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
