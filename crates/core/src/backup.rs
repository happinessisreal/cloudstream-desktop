//! Desktop backup/restore (T25).
//!
//! A backup is a versioned JSON document: a header, a section list, the data, and a SHA-256
//! checksum of the canonical form of that data. Export reads one consistent database snapshot and
//! never includes PIN hashes, installed packages, download jobs, caches or keyring secrets.
//!
//! Import is two steps over one engine: `preview` runs the exact apply logic inside a transaction
//! that is always rolled back, `apply` runs it for real. Both work on an `ImportSet`, which is what
//! both the desktop format and the Android adapter (`backup_android`) produce.
use crate::{models::*, store::Store};
use anyhow::{bail, Context, Result};
use rusqlite::{params_from_iter, Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const BACKUP_FORMAT: &str = "cloudstream-desktop-backup";
pub const BACKUP_VERSION: u32 = 1;
pub const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
pub const ALL_SECTIONS: [&str; 7] = [
    "profiles",
    "library",
    "history",
    "progress",
    "settings",
    "repositories",
    "provider_preferences",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecord {
    pub id: String,
    pub value: Value,
    /// The record's own `updated` column (epoch seconds).
    pub updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupRepository {
    pub url: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreferences {
    #[serde(default)]
    pub disabled: Vec<String>,
    #[serde(default)]
    pub clones: Vec<BackupRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupData {
    /// Profile rows without PIN material.
    #[serde(default)]
    pub profiles: Vec<Map<String, Value>>,
    /// Library entries, including titles whose source is not installed yet.
    #[serde(default)]
    pub library: Vec<BackupRecord>,
    #[serde(default)]
    pub unresolved: Vec<BackupRecord>,
    #[serde(default)]
    pub history: Vec<BackupRecord>,
    #[serde(default)]
    pub progress: Vec<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
    #[serde(default)]
    pub repositories: Vec<BackupRepository>,
    #[serde(default)]
    pub provider_preferences: ProviderPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub format: String,
    pub version: u32,
    pub created_at_ms: u64,
    pub app_version: String,
    /// The database schema the data was exported from (`PRAGMA user_version`).
    pub schema_version: u32,
    /// The settings revision at export time, to tell two backups of one install apart.
    pub source_revision: u64,
    pub sections: Vec<String>,
    pub checksum: String,
    pub data: BackupData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    /// Sections to include; `None` means all of them.
    #[serde(default)]
    pub sections: Option<Vec<String>>,
    /// Restrict profile-scoped data to these profiles; `None` means all profiles.
    #[serde(default)]
    pub profile_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Keep what is already here; only add what is missing.
    #[default]
    SkipExisting,
    /// Take the backup's copy of anything that differs.
    Overwrite,
    /// Keep whichever copy was changed most recently.
    KeepNewest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    /// Sections to import; `None` means every section the file offers.
    #[serde(default)]
    pub sections: Option<Vec<String>>,
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
    /// Android accounts only: where each Android account index goes. A desktop profile id, or
    /// `"new"` (the default for every account except the default account `0`, which maps to the
    /// `default` profile).
    #[serde(default)]
    pub account_targets: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Desktop,
    Android,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SectionReport {
    pub section: String,
    pub incoming: usize,
    pub new: usize,
    pub replaced: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreatedProfile {
    pub name: String,
    pub id: String,
}

/// An Android title that could not be tied to an installed source (yet). Kept for later matching.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedTitle {
    pub id: String,
    pub name: String,
    pub api_name: String,
    pub url: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub sections: Vec<SectionReport>,
    pub created_profiles: Vec<CreatedProfile>,
    pub unresolved: Vec<UnresolvedTitle>,
    pub repositories_to_add: Vec<BackupRepository>,
    pub notes: Vec<String>,
    /// Where the copy of the data from before this import was saved (set when applied).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_import_backup: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source: ImportSource,
    /// Backup file facts shown before importing.
    pub created_at_ms: Option<u64>,
    pub app_version: Option<String>,
    pub available_sections: Vec<String>,
    pub report: ImportReport,
    /// Must be presented to `apply_import` unchanged. Ties the apply to this exact file+options.
    pub token: String,
}

/// A new profile to create. `key` is the placeholder used inside ids until the real id exists.
#[derive(Debug, Clone, Default)]
pub struct NewProfile {
    pub key: String,
    pub name: String,
    pub avatar: String,
}

/// Everything an import wants to write, independent of where it came from.
#[derive(Debug, Clone, Default)]
pub struct ImportSet {
    pub source: Option<ImportSource>,
    pub new_profiles: Vec<NewProfile>,
    pub profiles: Vec<Map<String, Value>>,
    pub library: Vec<BackupRecord>,
    pub unresolved: Vec<BackupRecord>,
    pub history: Vec<BackupRecord>,
    pub progress: Vec<Map<String, Value>>,
    /// A complete `Settings` value (desktop) or a global patch (Android).
    pub settings: Option<Value>,
    pub settings_is_patch: bool,
    pub repositories: Vec<BackupRepository>,
    pub disabled_providers: Vec<String>,
    pub provider_clones: Vec<BackupRecord>,
    pub notes: Vec<String>,
    pub unresolved_titles: Vec<UnresolvedTitle>,
    pub sections: Vec<String>,
}

pub fn placeholder(key: &str) -> String {
    format!("@new:{key}@")
}

/// Stable JSON text: object keys are sorted (serde_json's default map), so equal data has equal
/// bytes regardless of how it was built.
pub fn canonical_checksum(data: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(data)?)))
}

fn now_ms() -> u64 {
    timestamp_ms()
}

fn sql_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null | rusqlite::types::Value::Blob(_) => Value::Null,
        rusqlite::types::Value::Integer(i) => json!(i),
        rusqlite::types::Value::Real(f) => json!(f),
        rusqlite::types::Value::Text(s) => Value::String(s),
    }
}

fn json_to_sql(value: &Value) -> rusqlite::types::Value {
    match value {
        Value::Null => rusqlite::types::Value::Null,
        Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(rusqlite::types::Value::Integer)
            .unwrap_or_else(|| rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => rusqlite::types::Value::Text(s.clone()),
        other => rusqlite::types::Value::Text(other.to_string()),
    }
}

fn dump_rows(tx: &Transaction, sql: &str, args: &[String]) -> Result<Vec<Map<String, Value>>> {
    let mut statement = tx.prepare(sql)?;
    let names: Vec<String> = statement.column_names().into_iter().map(String::from).collect();
    let rows = statement.query_map(params_from_iter(args.iter()), |row| {
        let mut map = Map::new();
        for (index, name) in names.iter().enumerate() {
            map.insert(name.clone(), sql_to_json(row.get::<_, rusqlite::types::Value>(index)?));
        }
        Ok(map)
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

fn records(tx: &Transaction, kind: &str) -> Result<Vec<BackupRecord>> {
    let mut statement = tx.prepare("SELECT id, value, updated FROM records WHERE kind=?1 ORDER BY id")?;
    let rows = statement.query_map([kind], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, value, updated) = row?;
        out.push(BackupRecord { id, value: serde_json::from_str(&value).context("A stored record could not be read")?, updated });
    }
    Ok(out)
}

fn table_columns(tx: &Transaction, table: &str) -> Result<Vec<String>> {
    let mut statement = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement.query_map([], |row| row.get::<_, String>(1))?;
    names.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

fn scoped_to(id: &str, profiles: &Option<BTreeSet<String>>) -> bool {
    match profiles {
        None => true,
        Some(set) => id.split_once("::").is_some_and(|(profile, _)| set.contains(profile)),
    }
}

fn wanted(sections: &[String], name: &str) -> bool {
    sections.iter().any(|s| s == name)
}

fn normalise_sections(requested: &Option<Vec<String>>) -> Result<Vec<String>> {
    let sections: Vec<String> = match requested {
        None => ALL_SECTIONS.iter().map(|s| s.to_string()).collect(),
        Some(list) => list.clone(),
    };
    for section in &sections {
        if !ALL_SECTIONS.contains(&section.as_str()) {
            bail!("Unknown backup section '{section}'");
        }
    }
    let mut seen = BTreeSet::new();
    Ok(sections.into_iter().filter(|s| seen.insert(s.clone())).collect())
}

impl Store {
    /// Export one consistent snapshot (a single read transaction across every table).
    pub fn export_backup(&self, options: &ExportOptions, app_version: &str) -> Result<BackupFile> {
        let sections = normalise_sections(&options.sections)?;
        let profile_filter: Option<BTreeSet<String>> = options.profile_ids.as_ref().map(|ids| ids.iter().cloned().collect());
        self.with_connection(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let schema_version: u32 = tx.query_row("PRAGMA user_version;", [], |row| row.get(0))?;
            let mut data = BackupData::default();
            let mut source_revision = 0;
            if let Some(raw) = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |row| row.get::<_, String>(0)).ok() {
                let settings: Value = serde_json::from_str(&raw)?;
                source_revision = settings.get("revision").and_then(Value::as_u64).unwrap_or(0);
                if wanted(&sections, "settings") {
                    data.settings = Some(settings);
                }
            }
            if wanted(&sections, "profiles") {
                // Explicit columns: PIN salt and hash never leave the database.
                let mut rows = dump_rows(&tx, "SELECT id, name, avatar, created_at, updated_at FROM profiles ORDER BY id", &[])?;
                if let Some(filter) = &profile_filter {
                    rows.retain(|row| row.get("id").and_then(Value::as_str).is_some_and(|id| filter.contains(id)));
                }
                data.profiles = rows;
            }
            if wanted(&sections, "library") {
                data.library = records(&tx, "library")?.into_iter().filter(|r| scoped_to(&r.id, &profile_filter)).collect();
                data.unresolved = records(&tx, "unresolved_import")?.into_iter().filter(|r| scoped_to(&r.id, &profile_filter)).collect();
            }
            if wanted(&sections, "history") {
                data.history = records(&tx, "history")?.into_iter().filter(|r| scoped_to(&r.id, &profile_filter)).collect();
            }
            if wanted(&sections, "progress") {
                let mut rows = dump_rows(&tx, "SELECT * FROM watch_progress ORDER BY profile_id, episode_id", &[])?;
                if let Some(filter) = &profile_filter {
                    rows.retain(|row| row.get("profile_id").and_then(Value::as_str).is_some_and(|id| filter.contains(id)));
                }
                data.progress = rows;
            }
            if wanted(&sections, "repositories") {
                // References only: the URL and a display name. Packages are never exported.
                data.repositories = records(&tx, "repository")?
                    .into_iter()
                    .filter_map(|record| {
                        let url = record.value.get("url")?.as_str()?.to_string();
                        let name = record.value.pointer("/manifest/name").and_then(Value::as_str).unwrap_or("").to_string();
                        Some(BackupRepository { url, name })
                    })
                    .collect();
            }
            if wanted(&sections, "provider_preferences") {
                data.provider_preferences.disabled = records(&tx, "provider_disabled")?.into_iter().map(|r| r.id).collect();
                data.provider_preferences.clones = records(&tx, "provider_clone")?;
            }
            let checksum = canonical_checksum(&serde_json::to_value(&data)?)?;
            Ok(BackupFile {
                format: BACKUP_FORMAT.into(),
                version: BACKUP_VERSION,
                created_at_ms: now_ms(),
                app_version: app_version.into(),
                schema_version,
                source_revision,
                sections,
                checksum,
                data,
            })
        })
    }

    /// Run an import. With `dry_run` every write is rolled back, so the report describes exactly
    /// what a real apply would do right now.
    pub fn run_import(&self, set: &ImportSet, options: &ImportOptions, dry_run: bool) -> Result<ImportReport> {
        self.with_connection(|connection| import_in_transaction(connection, set, options, dry_run))
    }
}

fn existing_ids(tx: &Transaction, sql: &str) -> Result<BTreeSet<String>> {
    let mut statement = tx.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<std::result::Result<BTreeSet<_>, _>>().map_err(Into::into)
}

/// Timestamp (ms) that decides which copy is newer.
fn record_time(kind: &str, value: &Value, updated_secs: i64) -> u64 {
    let field = match kind {
        "library" => value.get("updatedAt").or_else(|| value.get("savedAt")),
        "history" => value.get("watchedAt"),
        _ => None,
    };
    field.and_then(Value::as_u64).map(to_epoch_millis).unwrap_or((updated_secs.max(0) as u64) * 1000)
}

#[derive(PartialEq, Eq)]
enum Decision {
    Insert,
    Replace,
    Unchanged,
    Skip,
}

fn decide(policy: ConflictPolicy, existing: Option<(&Value, u64)>, incoming: &Value, incoming_time: u64) -> Decision {
    let Some((current, current_time)) = existing else {
        return Decision::Insert;
    };
    if current == incoming {
        return Decision::Unchanged;
    }
    match policy {
        ConflictPolicy::SkipExisting => Decision::Skip,
        ConflictPolicy::Overwrite => Decision::Replace,
        ConflictPolicy::KeepNewest => {
            if incoming_time > current_time {
                Decision::Replace
            } else {
                Decision::Skip
            }
        }
    }
}

fn tally(report: &mut SectionReport, decision: &Decision) {
    report.incoming += 1;
    match decision {
        Decision::Insert => report.new += 1,
        Decision::Replace => report.replaced += 1,
        Decision::Unchanged => report.unchanged += 1,
        Decision::Skip => report.skipped += 1,
    }
}

fn substitute(text: &str, ids: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for (key, id) in ids {
        out = out.replace(&placeholder(key), id);
    }
    out
}

fn substitute_value(value: &Value, ids: &HashMap<String, String>) -> Value {
    match value {
        Value::String(s) => Value::String(substitute(s, ids)),
        Value::Array(a) => Value::Array(a.iter().map(|v| substitute_value(v, ids)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), substitute_value(v, ids))).collect()),
        other => other.clone(),
    }
}

fn import_in_transaction(connection: &mut Connection, set: &ImportSet, options: &ImportOptions, dry_run: bool) -> Result<ImportReport> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut report = ImportReport { notes: set.notes.clone(), unresolved: set.unresolved_titles.clone(), ..Default::default() };
    let wanted_section = |name: &str| options.sections.as_ref().map_or(true, |list| list.iter().any(|s| s == name));
    let policy = options.conflict_policy;

    // Profiles first: everything else can then refer to them.
    let mut known_profiles = existing_ids(&tx, "SELECT id FROM profiles")?;
    let mut placeholders: HashMap<String, String> = HashMap::new();
    for new_profile in &set.new_profiles {
        let id = if dry_run { format!("preview-{}", new_profile.key) } else { format!("prof_{}", uuid::Uuid::new_v4().simple()) };
        if !dry_run {
            let now = now_ms();
            tx.execute(
                "INSERT INTO profiles (id, name, avatar, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                rusqlite::params![&id, &new_profile.name, &new_profile.avatar, now],
            )?;
        }
        known_profiles.insert(id.clone());
        report.created_profiles.push(CreatedProfile { name: new_profile.name.clone(), id: id.clone() });
        placeholders.insert(new_profile.key.clone(), id);
    }
    if wanted_section("profiles") && !set.profiles.is_empty() {
        let mut section = SectionReport { section: "profiles".into(), ..Default::default() };
        let current: HashMap<String, (String, String)> = {
            let mut statement = tx.prepare("SELECT id, name, avatar FROM profiles")?;
            let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, (row.get::<_, String>(1)?, row.get::<_, String>(2)?))))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        for row in &set.profiles {
            let (Some(id), Some(name)) = (row.get("id").and_then(Value::as_str), row.get("name").and_then(Value::as_str)) else {
                bail!("The backup contains a profile without an id or name");
            };
            let avatar = row.get("avatar").and_then(Value::as_str).unwrap_or("default");
            let existing = current.get(id).map(|(n, a)| json!([n, a]));
            let incoming = json!([name, avatar]);
            let decision = decide(policy, existing.as_ref().map(|v| (v, 0)), &incoming, 0);
            tally(&mut section, &decision);
            if dry_run {
                if decision == Decision::Insert {
                    known_profiles.insert(id.to_string());
                }
                continue;
            }
            match decision {
                Decision::Insert => {
                    let now = now_ms();
                    tx.execute(
                        "INSERT INTO profiles (id, name, avatar, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                        rusqlite::params![id, name, avatar, row.get("created_at").and_then(Value::as_i64).unwrap_or(now as i64)],
                    )?;
                    known_profiles.insert(id.to_string());
                }
                Decision::Replace => {
                    tx.execute("UPDATE profiles SET name=?2, avatar=?3, updated_at=?4 WHERE id=?1", rusqlite::params![id, name, avatar, now_ms()])?;
                }
                _ => {}
            }
        }
        report.sections.push(section);
    }

    let profile_of = |key: &str| key.split_once("::").map(|(p, _)| p.to_string()).unwrap_or_default();
    let write_records = |tx: &Transaction, kind: &str, items: &[BackupRecord], section_name: &str, scoped: bool, report: &mut ImportReport| -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut section = SectionReport { section: section_name.into(), ..Default::default() };
        for item in items {
            let id = substitute(&item.id, &placeholders);
            if scoped && !known_profiles.contains(&profile_of(&id)) {
                // Never create data for a profile that does not exist here.
                section.incoming += 1;
                section.skipped += 1;
                continue;
            }
            let value = substitute_value(&item.value, &placeholders);
            let current: Option<(String, i64)> = tx
                .query_row("SELECT value, updated FROM records WHERE kind=?1 AND id=?2", rusqlite::params![kind, &id], |row| Ok((row.get(0)?, row.get(1)?)))
                .ok();
            let current_value = current.as_ref().and_then(|(raw, _)| serde_json::from_str::<Value>(raw).ok());
            let existing = current_value.as_ref().map(|v| (v, record_time(kind, v, current.as_ref().map_or(0, |c| c.1))));
            let decision = decide(policy, existing, &value, record_time(kind, &value, item.updated));
            tally(&mut section, &decision);
            if !dry_run && matches!(decision, Decision::Insert | Decision::Replace) {
                tx.execute(
                    "INSERT INTO records(kind,id,value,updated) VALUES (?1,?2,?3,?4) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
                    rusqlite::params![kind, &id, serde_json::to_string(&value)?, item.updated],
                )?;
            }
        }
        report.sections.push(section);
        Ok(())
    };
    if wanted_section("library") {
        write_records(&tx, "library", &set.library, "library", true, &mut report)?;
        write_records(&tx, "unresolved_import", &set.unresolved, "unresolved", true, &mut report)?;
    }
    if wanted_section("history") {
        write_records(&tx, "history", &set.history, "history", true, &mut report)?;
    }

    if wanted_section("progress") && !set.progress.is_empty() {
        let columns = table_columns(&tx, "watch_progress")?;
        let mut section = SectionReport { section: "progress".into(), ..Default::default() };
        for row in &set.progress {
            let mut row = row.clone();
            for value in row.values_mut() {
                *value = substitute_value(value, &placeholders);
            }
            let (Some(profile), Some(episode)) = (row.get("profile_id").and_then(Value::as_str), row.get("episode_id").and_then(Value::as_str)) else {
                bail!("The backup contains a progress entry without a profile or episode");
            };
            if !known_profiles.contains(profile) {
                section.incoming += 1;
                section.skipped += 1;
                continue;
            }
            let (profile, episode) = (profile.to_string(), episode.to_string());
            let incoming_time = row.get("updated_at_ms").and_then(Value::as_u64).unwrap_or(0);
            let current: Option<Map<String, Value>> = dump_rows(&tx, "SELECT * FROM watch_progress WHERE profile_id=?1 AND episode_id=?2", &[profile.clone(), episode.clone()])?.into_iter().next();
            let decision = match &current {
                None => Decision::Insert,
                Some(existing) => {
                    let same = columns.iter().all(|c| existing.get(c).unwrap_or(&Value::Null) == row.get(c).unwrap_or(&Value::Null) || !row.contains_key(c));
                    if same {
                        Decision::Unchanged
                    } else {
                        let existing_time = existing.get("updated_at_ms").and_then(Value::as_u64).unwrap_or(0);
                        match policy {
                            ConflictPolicy::SkipExisting => Decision::Skip,
                            ConflictPolicy::Overwrite => Decision::Replace,
                            ConflictPolicy::KeepNewest => {
                                if incoming_time > existing_time {
                                    Decision::Replace
                                } else {
                                    Decision::Skip
                                }
                            }
                        }
                    }
                }
            };
            tally(&mut section, &decision);
            if !dry_run && matches!(decision, Decision::Insert | Decision::Replace) {
                let present: Vec<&String> = columns.iter().filter(|c| row.contains_key(*c)).collect();
                let sql = format!(
                    "INSERT OR REPLACE INTO watch_progress ({}) VALUES ({})",
                    present.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(","),
                    present.iter().map(|_| "?").collect::<Vec<_>>().join(",")
                );
                let values: Vec<rusqlite::types::Value> = present.iter().map(|c| json_to_sql(&row[*c])).collect();
                tx.execute(&sql, params_from_iter(values.iter()))?;
            }
        }
        report.sections.push(section);
    }

    if wanted_section("provider_preferences") && (!set.disabled_providers.is_empty() || !set.provider_clones.is_empty()) {
        let mut section = SectionReport { section: "provider_preferences".into(), ..Default::default() };
        for id in &set.disabled_providers {
            let present = tx.query_row("SELECT 1 FROM records WHERE kind='provider_disabled' AND id=?1", [id], |_| Ok(())).is_ok();
            tally(&mut section, if present { &Decision::Unchanged } else { &Decision::Insert });
            if !present && !dry_run {
                tx.execute("INSERT INTO records(kind,id,value,updated) VALUES ('provider_disabled',?1,'true',?2)", rusqlite::params![id, timestamp()])?;
            }
        }
        report.sections.push(section);
        write_records(&tx, "provider_clone", &set.provider_clones, "provider_clones", false, &mut report)?;
    }

    if wanted_section("repositories") && !set.repositories.is_empty() {
        let have: BTreeSet<String> = records(&tx, "repository")?.into_iter().filter_map(|r| r.value.get("url").and_then(Value::as_str).map(str::to_string)).collect();
        let mut section = SectionReport { section: "repositories".into(), ..Default::default() };
        for repository in &set.repositories {
            if have.contains(&repository.url) {
                tally(&mut section, &Decision::Unchanged);
            } else {
                tally(&mut section, &Decision::Insert);
                report.repositories_to_add.push(repository.clone());
            }
        }
        report.sections.push(section);
    }

    if wanted_section("settings") {
        if let Some(incoming) = &set.settings {
            let raw: Option<String> = tx.query_row("SELECT value FROM records WHERE kind='settings' AND id='app'", [], |row| row.get(0)).ok();
            let mut current: Settings = match &raw {
                Some(text) => serde_json::from_str(text).context("Current settings could not be read")?,
                None => Settings::default(),
            };
            let mut updated = if set.settings_is_patch {
                let mut copy = current.clone();
                crate::store::apply_settings_patch(&mut copy, incoming, Some("global"), None)?;
                copy
            } else {
                serde_json::from_value::<Settings>(incoming.clone()).context("The backup's settings are not valid")?
            };
            updated.validate()?;
            updated.revision = current.revision.saturating_add(1);
            let unchanged = {
                let mut left = updated.clone();
                left.revision = current.revision;
                left == current
            };
            let mut section = SectionReport { section: "settings".into(), incoming: 1, ..Default::default() };
            if unchanged {
                section.unchanged = 1;
            } else if raw.is_some() {
                section.replaced = 1;
            } else {
                section.new = 1;
            }
            if !dry_run && !unchanged {
                // Settings are a single document: choosing the section is the explicit request
                // to take the backup's values, so the per-record conflict policy does not apply.
                current = updated;
                tx.execute(
                    "INSERT INTO records(kind,id,value,updated) VALUES ('settings','app',?1,?2) ON CONFLICT(kind,id) DO UPDATE SET value=excluded.value,updated=excluded.updated",
                    rusqlite::params![serde_json::to_string(&current)?, timestamp()],
                )?;
            }
            report.sections.push(section);
        }
    }

    if dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
    }
    Ok(report)
}

/// What a file turned out to be, after validation but before anything touches the database.
pub enum ParsedBackup {
    Desktop(BackupFile),
    Android(Value),
}

/// Validate size, type, version and checksum. Nothing here touches the database.
pub fn parse_backup(bytes: &[u8], current_schema_version: u32) -> Result<ParsedBackup> {
    if bytes.is_empty() {
        bail!("The backup file is empty");
    }
    if bytes.len() > MAX_BACKUP_BYTES {
        bail!("The backup file is larger than {} MiB, which this app does not accept", MAX_BACKUP_BYTES / (1024 * 1024));
    }
    let value: Value = serde_json::from_slice(bytes).context("This file is not a valid backup (it is not readable JSON)")?;
    let Some(object) = value.as_object() else {
        bail!("This file is not a backup");
    };
    if object.get("format").and_then(Value::as_str) == Some(BACKUP_FORMAT) {
        let version = object.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
        if version == 0 {
            bail!("The backup has no format version");
        }
        if version > BACKUP_VERSION {
            bail!("This backup was made by a newer version of the app (backup format {version}). Update the app to restore it.");
        }
        let schema = object.get("schemaVersion").and_then(Value::as_u64).unwrap_or(0) as u32;
        if schema > current_schema_version {
            bail!("This backup was made by a newer version of the app (database version {schema}). Update the app to restore it.");
        }
        let data = object.get("data").context("The backup has no data")?;
        let expected = object.get("checksum").and_then(Value::as_str).unwrap_or("");
        if canonical_checksum(data)? != expected {
            bail!("The backup is damaged or was edited: its checksum does not match");
        }
        let file: BackupFile = serde_json::from_value(value).context("The backup has an unexpected structure")?;
        return Ok(ParsedBackup::Desktop(file));
    }
    if object.contains_key("datastore") && object.contains_key("settings") {
        return Ok(ParsedBackup::Android(value));
    }
    bail!("This file is not a Cloudstream Desktop or Cloudstream Android backup")
}

/// The import set a desktop backup describes.
pub fn desktop_import_set(file: &BackupFile) -> ImportSet {
    let data = &file.data;
    ImportSet {
        source: Some(ImportSource::Desktop),
        new_profiles: Vec::new(),
        profiles: data.profiles.clone(),
        library: data.library.clone(),
        unresolved: data.unresolved.clone(),
        history: data.history.clone(),
        progress: data.progress.clone(),
        settings: data.settings.clone(),
        settings_is_patch: false,
        repositories: data.repositories.clone(),
        disabled_providers: data.provider_preferences.disabled.clone(),
        provider_clones: data.provider_preferences.clones.clone(),
        notes: Vec::new(),
        unresolved_titles: Vec::new(),
        sections: file.sections.clone(),
    }
}

/// Binds a preview to the exact bytes and options it was computed for.
pub fn preview_token(bytes: &[u8], options: &ImportOptions) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.update([0]);
    hasher.update(serde_json::to_vec(options)?);
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn write_backup_atomically(path: &std::path::Path, file: &BackupFile) -> Result<()> {
    let parent = path.parent().context("The backup needs a destination folder")?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".cloudstream-backup-{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        use std::io::Write;
        let mut handle = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        handle.write_all(&serde_json::to_vec_pretty(file)?)?;
        handle.sync_all()?;
        std::fs::rename(&temp, path)?;
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if temp.exists() {
        let _ = std::fs::remove_file(&temp);
    }
    result.with_context(|| format!("Could not write the backup to {}", path.display()))
}
