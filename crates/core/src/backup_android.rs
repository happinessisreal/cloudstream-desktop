//! Android backup import adapter (T25).
//!
//! Reads Cloudstream Android's `BackupFile { datastore, settings }`: six typed maps per part
//! (`_Bool`, `_Int`, `_String`, `_Float`, `_Long`, `_StringSet`), where structured values are JSON
//! text stored as strings and every key belongs to a local account namespace (`<account>/<folder>/<id>`,
//! or the bare `<folder>/<id>` of very old backups, which is the default account).
//!
//! Android's numeric hash ids are never used as desktop ids. A title is tied to an installed
//! desktop source only by evidence: the source's display name, and the host of its URL when
//! several sources share a name. Anything that cannot be tied is kept as an `unresolved_import`
//! record and shown to the user, never guessed. Credentials, PINs, tokens, download paths and
//! packages are never read.
use crate::backup::*;
use crate::models::*;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

const FOLDER_BOOKMARK: &str = "result_watch_state_data";
const FOLDER_STATUS: &str = "result_watch_state";
const FOLDER_FAVORITE: &str = "result_favorites_state_data";
const FOLDER_SUBSCRIBED: &str = "result_subscribed_state_data";
const FOLDER_RESUME: &str = "result_resume_watching_2";
const FOLDER_RESUME_OLD: &str = "result_resume_watching";
const KEY_ACCOUNTS: &str = "data_store_helper/account";
const KEY_REPOSITORIES: &str = "REPOSITORIES_KEY";
/// Folders that are recognised but cannot be mapped (their ids are Android hashes of episodes).
const FOLDERS_UNMAPPABLE: [&str; 2] = ["video_pos_dur", "video_watch_state"];

/// A desktop source that Android titles can be matched to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    pub id: String,
    pub name: String,
    pub main_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(String),
    Ambiguous(Vec<String>),
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AndroidResume {
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AndroidTitle {
    pub account: String,
    pub api_name: String,
    pub url: String,
    pub name: String,
    pub media_type: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    /// Android watch status id, if one was stored (`WatchType.internalId`).
    pub status: Option<i32>,
    pub favorite: bool,
    pub subscribed: bool,
    /// `false` for a title that only appears in "continue watching".
    pub in_library: bool,
    pub saved_at_ms: u64,
    pub updated_at_ms: u64,
    pub resume: Option<AndroidResume>,
    #[serde(skip)]
    pub android_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AndroidAccount {
    pub index: String,
    pub name: String,
    pub has_pin: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AndroidParsed {
    pub accounts: Vec<AndroidAccount>,
    pub titles: Vec<AndroidTitle>,
    pub repositories: Vec<BackupRepository>,
    pub settings_patch: Map<String, Value>,
    pub unmappable_entries: usize,
    pub orphan_status_entries: usize,
    pub ignored_keys: usize,
}

/// Flatten the typed maps of one part into `(key, value)`. JSON text in `_String` is parsed.
fn flatten(part: &Value) -> Vec<(String, Value)> {
    let Some(object) = part.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (kind, map) in object {
        let Some(map) = map.as_object() else { continue };
        for (key, value) in map {
            let value = match (kind.as_str(), value) {
                ("_String", Value::String(text)) => serde_json::from_str::<Value>(text).unwrap_or_else(|_| value.clone()),
                _ => value.clone(),
            };
            out.push((key.clone(), value));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `<account>/<folder>/<id>`, or the old unscoped `<folder>/<id>` meaning the default account.
fn split_key(key: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = key.splitn(3, '/').collect();
    match parts.as_slice() {
        [account, folder, id] if !account.is_empty() && account.bytes().all(|b| b.is_ascii_digit()) => {
            Some(((*account).into(), (*folder).into(), (*id).into()))
        }
        [folder, id] => Some(("0".into(), (*folder).into(), (*id).into())),
        [folder, rest @ ..] if !rest.is_empty() => Some(("0".into(), (*folder).into(), rest.join("/"))),
        _ => None,
    }
}

fn text(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn number(value: &Value, field: &str) -> Option<i64> {
    value.get(field).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn as_int(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn parse_android(root: &Value) -> Result<AndroidParsed> {
    let datastore = root.get("datastore").context("The Android backup has no datastore section")?;
    let settings = root.get("settings").context("The Android backup has no settings section")?;
    if !datastore.is_object() || !settings.is_object() {
        bail!("The Android backup's datastore and settings sections must be objects");
    }
    let mut parsed = AndroidParsed::default();
    let entries = flatten(datastore);

    // Accounts (profiles).
    if let Some((_, value)) = entries.iter().find(|(k, _)| k == KEY_ACCOUNTS) {
        for account in value.as_array().into_iter().flatten() {
            let Some(index) = number(account, "keyIndex") else { continue };
            parsed.accounts.push(AndroidAccount {
                index: index.to_string(),
                name: text(account, "name").unwrap_or_else(|| format!("Profile {index}")),
                has_pin: text(account, "lockPin").is_some(),
            });
        }
    }
    if let Some((_, value)) = entries.iter().find(|(k, _)| k == KEY_REPOSITORIES) {
        for repository in value.as_array().into_iter().flatten() {
            if let Some(url) = text(repository, "url") {
                parsed.repositories.push(BackupRepository { url, name: text(repository, "name").unwrap_or_default() });
            }
        }
    }

    let mut titles: BTreeMap<(String, String, String), AndroidTitle> = BTreeMap::new();
    let mut by_id: HashMap<(String, String), (String, String, String)> = HashMap::new();
    let mut statuses: Vec<(String, String, i64)> = Vec::new();
    let mut resumes: Vec<(String, Value)> = Vec::new();

    for (key, value) in &entries {
        if key == KEY_ACCOUNTS || key == KEY_REPOSITORIES || key == "data_store_helper/account_key_index" {
            continue;
        }
        let Some((account, folder, id)) = split_key(key) else {
            parsed.ignored_keys += 1;
            continue;
        };
        match folder.as_str() {
            FOLDER_BOOKMARK | FOLDER_FAVORITE | FOLDER_SUBSCRIBED => {
                let (Some(api_name), Some(url)) = (text(value, "apiName"), text(value, "url")) else {
                    parsed.ignored_keys += 1;
                    continue;
                };
                let title_key = (account.clone(), api_name.clone(), url.clone());
                let title = titles.entry(title_key.clone()).or_insert_with(|| AndroidTitle {
                    account: account.clone(),
                    api_name,
                    url,
                    name: text(value, "name").unwrap_or_default(),
                    media_type: text(value, "type"),
                    poster_url: text(value, "posterUrl"),
                    year: number(value, "year").map(|y| y as i32),
                    status: None,
                    favorite: false,
                    subscribed: false,
                    in_library: true,
                    saved_at_ms: 0,
                    updated_at_ms: 0,
                    resume: None,
                    android_ids: Vec::new(),
                });
                match folder.as_str() {
                    FOLDER_FAVORITE => title.favorite = true,
                    FOLDER_SUBSCRIBED => title.subscribed = true,
                    _ => {
                        if let Some(time) = number(value, "bookmarkedTime") {
                            title.saved_at_ms = time.max(0) as u64;
                        }
                    }
                }
                if title.name.is_empty() {
                    title.name = text(value, "name").unwrap_or_default();
                }
                if let Some(time) = number(value, "latestUpdatedTime") {
                    title.updated_at_ms = title.updated_at_ms.max(time.max(0) as u64);
                }
                title.android_ids.push(id.clone());
                by_id.insert((account.clone(), id), title_key.clone());
                if let Some(inner) = number(value, "id") {
                    by_id.insert((account, inner.to_string()), title_key);
                }
            }
            FOLDER_STATUS => match as_int(value) {
                Some(status) => statuses.push((account, id, status)),
                None => parsed.ignored_keys += 1,
            },
            FOLDER_RESUME | FOLDER_RESUME_OLD => resumes.push((account, value.clone())),
            folder if FOLDERS_UNMAPPABLE.contains(&folder) => parsed.unmappable_entries += 1,
            _ => parsed.ignored_keys += 1,
        }
    }

    for (account, id, status) in statuses {
        match by_id.get(&(account, id)) {
            Some(key) => {
                if let Some(title) = titles.get_mut(key) {
                    title.status = Some(status as i32);
                }
            }
            None => parsed.orphan_status_entries += 1,
        }
    }
    for (account, value) in resumes {
        let (Some(api_name), Some(url)) = (text(&value, "apiName"), text(&value, "url")) else { continue };
        let Some(position) = value.get("watchPos").filter(|p| p.is_object()) else { continue };
        let position_ms = number(position, "position").unwrap_or(0).max(0);
        let duration_ms = number(position, "duration").unwrap_or(0).max(0);
        let key = (account.clone(), api_name.clone(), url.clone());
        let title = titles.entry(key).or_insert_with(|| AndroidTitle {
            account,
            api_name,
            url,
            name: text(&value, "name").unwrap_or_default(),
            media_type: text(&value, "type"),
            poster_url: text(&value, "posterUrl"),
            year: None,
            status: None,
            favorite: false,
            subscribed: false,
            in_library: false,
            saved_at_ms: 0,
            updated_at_ms: 0,
            resume: None,
            android_ids: Vec::new(),
        });
        title.resume = Some(AndroidResume {
            season: number(&value, "season").map(|s| s as i32),
            episode: number(&value, "episode").map(|e| e as i32),
            position_seconds: position_ms as f64 / 1000.0,
            duration_seconds: (duration_ms > 0).then(|| duration_ms as f64 / 1000.0),
        });
    }
    parsed.titles = titles.into_values().collect();

    // Settings: only an explicit allow-list is read. Credentials and device paths are never touched.
    let mut playback = Map::new();
    let mut player = Map::new();
    for (key, value) in flatten(settings) {
        match key.as_str() {
            "autoplay_next_key" => {
                if let Some(v) = value.as_bool() {
                    playback.insert("autoNext".into(), json!(v));
                }
            }
            "start_paused_key" => {
                if let Some(v) = value.as_bool() {
                    playback.insert("startPaused".into(), json!(v));
                }
            }
            "compressor_enabled_key" => {
                if let Some(v) = value.as_bool() {
                    player.insert("audioCompressor".into(), json!(v));
                }
            }
            "video_buffer_size_key" => {
                if let Some(v) = as_int(&value).filter(|v| (0..=i64::from(BUFFER_RAM_LIMIT_MB)).contains(v)) {
                    player.insert("bufferRamMb".into(), json!(v));
                }
            }
            "video_buffer_length_key" => {
                if let Some(v) = as_int(&value).filter(|v| (0..=i64::from(BUFFER_SECONDS_LIMIT)).contains(v)) {
                    player.insert("bufferSeconds".into(), json!(v));
                }
            }
            _ => parsed.ignored_keys += 1,
        }
    }
    if !playback.is_empty() {
        parsed.settings_patch.insert("playback".into(), Value::Object(playback));
    }
    if !player.is_empty() {
        parsed.settings_patch.insert("player".into(), Value::Object(player));
    }
    Ok(parsed)
}

/// Tie an Android title to an installed source by name, then by URL host when names collide.
pub fn resolve_provider(api_name: &str, url: &str, providers: &[ProviderRef]) -> Resolution {
    let wanted = api_name.trim().to_lowercase();
    let candidates: Vec<&ProviderRef> = providers.iter().filter(|p| p.name.trim().to_lowercase() == wanted).collect();
    match candidates.len() {
        0 => Resolution::Missing,
        1 => Resolution::Resolved(candidates[0].id.clone()),
        _ => {
            let host = |text: &str| url::Url::parse(text).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_lowercase()));
            let title_host = host(url);
            let matching: Vec<&ProviderRef> = candidates
                .iter()
                .copied()
                .filter(|p| title_host.is_some() && host(&p.main_url) == title_host)
                .collect();
            match matching.len() {
                1 => Resolution::Resolved(matching[0].id.clone()),
                _ => Resolution::Ambiguous(candidates.iter().map(|p| p.id.clone()).collect()),
            }
        }
    }
}

fn status_name(status: Option<i32>) -> WatchStatus {
    status.map(WatchStatus::from_android_int).unwrap_or_default()
}

/// The desktop records a resolved title becomes: a library entry (when it was in the Android
/// library) and a progress row (when it was being watched).
pub fn title_records(profile: &str, title: &AndroidTitle, provider_id: &str) -> (Option<BackupRecord>, Option<Map<String, Value>>) {
    let media_id = compute_media_id(provider_id, &title.url, None);
    let now = timestamp_ms();
    let saved = if title.saved_at_ms > 0 { title.saved_at_ms } else { now };
    let updated = if title.updated_at_ms > 0 { title.updated_at_ms } else { saved };
    let library = title.in_library.then(|| {
        let item = json!({
            "providerId": provider_id,
            "name": title.name,
            "url": title.url,
            "posterUrl": title.poster_url,
            "mediaType": title.media_type,
            "year": title.year,
            "mediaId": media_id,
        });
        let entry = json!({
            "id": media_id,
            "item": item,
            "savedAt": saved,
            "watchStatus": serde_json::to_value(status_name(title.status)).unwrap_or(Value::Null),
            "isFavorite": title.favorite,
            "isSubscribed": title.subscribed,
            "profileId": profile,
            "updatedAt": updated,
        });
        BackupRecord { id: format!("{profile}::{media_id}"), value: entry, updated: (updated / 1000) as i64 }
    });
    let progress = title.resume.as_ref().map(|resume| {
        let episode = Episode { season: resume.season, episode: resume.episode, ..Default::default() };
        let episode_id = compute_episode_id(&media_id, &episode, None, None);
        let watched = resume.duration_seconds.is_some_and(|d| d > 0.0 && resume.position_seconds >= 0.9 * d);
        let mut row = Map::new();
        row.insert("profile_id".into(), json!(profile));
        row.insert("media_id".into(), json!(media_id));
        row.insert("episode_id".into(), json!(episode_id));
        row.insert("position_seconds".into(), json!(resume.position_seconds));
        row.insert("duration_seconds".into(), resume.duration_seconds.map_or(Value::Null, |d| json!(d)));
        row.insert("updated_at_ms".into(), json!(updated));
        row.insert("is_watched".into(), json!(i64::from(watched)));
        row.insert("extra".into(), json!(json!({ "imported": "android", "dubUnknown": true }).to_string()));
        row
    });
    (library, progress)
}

fn short_hash(text: &str) -> String {
    digest_string(text).chars().take(16).collect()
}

/// Build the import set for an Android backup. Pure: needs only the source list and the
/// existing profiles, so it is fully testable with fixtures.
pub fn plan_android(
    parsed: &AndroidParsed,
    providers: &[ProviderRef],
    existing_profiles: &[(String, String)],
    options: &ImportOptions,
) -> ImportSet {
    let mut set = ImportSet { source: Some(ImportSource::Android), sections: vec!["profiles".into(), "library".into(), "progress".into(), "settings".into(), "repositories".into()], ..Default::default() };
    let mut taken_names: Vec<String> = existing_profiles.iter().map(|(_, name)| name.to_lowercase()).collect();

    // One target per Android account that actually has data.
    let mut accounts: Vec<AndroidAccount> = parsed.accounts.clone();
    for title in &parsed.titles {
        if !accounts.iter().any(|a| a.index == title.account) {
            let name = if title.account == "0" { "Default".to_string() } else { format!("Profile {}", title.account) };
            accounts.push(AndroidAccount { index: title.account.clone(), name, has_pin: false });
        }
    }
    let mut targets: HashMap<String, String> = HashMap::new();
    let mut pins = 0;
    for account in &accounts {
        if !parsed.titles.iter().any(|t| t.account == account.index) {
            continue;
        }
        let wanted = options.account_targets.get(&account.index).cloned().unwrap_or_else(|| if account.index == "0" { "default".into() } else { "new".into() });
        if account.has_pin {
            pins += 1;
        }
        if wanted == "new" {
            let mut name = account.name.trim().to_string();
            if name.is_empty() {
                name = format!("Profile {}", account.index);
            }
            if taken_names.contains(&name.to_lowercase()) {
                let base = format!("{name} (imported)");
                name = base.clone();
                let mut n = 2;
                while taken_names.contains(&name.to_lowercase()) {
                    name = format!("{base} {n}");
                    n += 1;
                }
            }
            taken_names.push(name.to_lowercase());
            let key = format!("android:{}", account.index);
            targets.insert(account.index.clone(), placeholder(&key));
            set.new_profiles.push(NewProfile { key, name, avatar: "default".into() });
        } else {
            targets.insert(account.index.clone(), wanted);
        }
    }

    let mut unresolved_count = 0;
    for title in &parsed.titles {
        let Some(profile) = targets.get(&title.account) else { continue };
        match resolve_provider(&title.api_name, &title.url, providers) {
            Resolution::Resolved(provider_id) => {
                let (library, progress) = title_records(profile, title, &provider_id);
                set.library.extend(library);
                set.progress.extend(progress);
            }
            other => {
                unresolved_count += 1;
                let reason = match other {
                    Resolution::Ambiguous(ids) => format!("{} installed sources are called '{}' and none matches this title's website", ids.len(), title.api_name),
                    _ => format!("No installed source is called '{}'", title.api_name),
                };
                let id = format!("{profile}::android:{}:{}", title.account, short_hash(&format!("{}\u{1f}{}", title.api_name, title.url)));
                let mut value = serde_json::to_value(title).unwrap_or(Value::Null);
                if let Some(object) = value.as_object_mut() {
                    object.insert("reason".into(), json!(reason));
                    object.insert("profileId".into(), json!(profile));
                    object.insert("recordId".into(), json!(id));
                }
                set.unresolved_titles.push(UnresolvedTitle { id: id.clone(), name: title.name.clone(), api_name: title.api_name.clone(), url: title.url.clone(), reason });
                set.unresolved.push(BackupRecord { id, value, updated: (timestamp()) as i64 });
            }
        }
    }

    if !parsed.settings_patch.is_empty() {
        set.settings = Some(Value::Object(parsed.settings_patch.clone()));
        set.settings_is_patch = true;
    }
    set.repositories = parsed.repositories.clone();

    if unresolved_count > 0 {
        set.notes.push(format!("{unresolved_count} titles belong to sources that are not installed here. They are kept and will be added to your library when a matching source is installed."));
    }
    if parsed.unmappable_entries > 0 {
        set.notes.push(format!("{} per-episode watch markers could not be imported: Android identifies episodes by an internal number that cannot be matched to desktop episodes. Continue-watching positions were imported where available.", parsed.unmappable_entries));
    }
    if parsed.orphan_status_entries > 0 {
        set.notes.push(format!("{} watch statuses had no matching saved title and were skipped.", parsed.orphan_status_entries));
    }
    if pins > 0 {
        set.notes.push(format!("{pins} profile PINs were not imported. Set a PIN again in Profiles."));
    }
    set.notes.push("Account tokens, logins, download folders and installed packages are never imported.".into());
    set
}

/// Titles previously kept as unresolved that an installed source can now take.
pub fn resolve_kept(profile: &str, kept: &Value, providers: &[ProviderRef]) -> Option<(Option<BackupRecord>, Option<Map<String, Value>>)> {
    let title: AndroidTitle = serde_json::from_value(kept.clone()).ok()?;
    match resolve_provider(&title.api_name, &title.url, providers) {
        Resolution::Resolved(provider_id) => Some(title_records(profile, &title, &provider_id)),
        _ => None,
    }
}
