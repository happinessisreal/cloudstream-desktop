use crate::models::*;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS records (kind TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL, updated INTEGER NOT NULL, PRIMARY KEY(kind,id));
             PRAGMA user_version=1;"
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<()> {
        self.connection.lock().map_err(|_| anyhow::anyhow!("Database lock failed"))?
            .execute("INSERT INTO records(kind,id,value,updated) VALUES (?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated", params![kind,id,serde_json::to_string(value)?,timestamp()])?;
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
}
