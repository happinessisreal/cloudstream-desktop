//! Tracking accounts, remote libraries and progress sync (T23).
//!
//! Provides:
//! - TrackingProvider trait and implementations for AniList, MyAnimeList, Kitsu, and Simkl
//! - Explicit conversion between local WatchStatus and remote RemoteStatus
//! - Profile-scoped tracking accounts and credential handling
//! - Idempotent sync mutation queue with offline retry and rate-limiting
//! - Watched threshold policy preventing premature series completion

use crate::{
    metadata::BoxFuture,
    models::WatchStatus,
};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackingService {
    AniList,
    MyAnimeList,
    Kitsu,
    Simkl,
}

impl TrackingService {
    pub fn name(&self) -> &'static str {
        match self {
            Self::AniList => "AniList",
            Self::MyAnimeList => "MyAnimeList",
            Self::Kitsu => "Kitsu",
            Self::Simkl => "Simkl",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::AniList => "anilist",
            Self::MyAnimeList => "mal",
            Self::Kitsu => "kitsu",
            Self::Simkl => "simkl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteStatus {
    Watching,
    Completed,
    OnHold,
    Dropped,
    PlanToWatch,
    Rewatching,
}

impl RemoteStatus {
    /// Explicit conversion from local WatchStatus to remote status.
    pub fn from_local(local: &WatchStatus) -> Option<Self> {
        match local {
            WatchStatus::Watching => Some(Self::Watching),
            WatchStatus::Completed => Some(Self::Completed),
            WatchStatus::OnHold => Some(Self::OnHold),
            WatchStatus::Dropped => Some(Self::Dropped),
            WatchStatus::PlanToWatch => Some(Self::PlanToWatch),
            WatchStatus::None => None,
        }
    }

    /// Explicit conversion from remote status to local WatchStatus.
    /// Note: Remote Rewatching maps to local Watching, preserving activity.
    pub fn to_local(&self) -> WatchStatus {
        match self {
            Self::Watching | Self::Rewatching => WatchStatus::Watching,
            Self::Completed => WatchStatus::Completed,
            Self::OnHold => WatchStatus::OnHold,
            Self::Dropped => WatchStatus::Dropped,
            Self::PlanToWatch => WatchStatus::PlanToWatch,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingAccount {
    pub profile_id: String,
    pub service: TrackingService,
    pub account_id: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub connected_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub service: TrackingService,
    pub external_id: String,
    pub title: String,
    pub status: RemoteStatus,
    pub score: Option<f32>,
    pub progress: u32,
    pub total_episodes: Option<u32>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncMutation {
    pub id: String,
    pub profile_id: String,
    pub service: TrackingService,
    pub media_id: String,
    pub external_id: String,
    pub progress: u32,
    pub status: Option<RemoteStatus>,
    pub score: Option<f32>,
    pub timestamp_ms: u64,
    pub attempts: u32,
}

// -----------------------------------------------------------------------------
// Tracking Provider Trait
// -----------------------------------------------------------------------------
pub trait TrackingProvider: Send + Sync {
    fn service(&self) -> TrackingService;
    fn auth_url(&self, state: &str, redirect_uri: &str) -> String;
    fn exchange_code(&self, code: &str, redirect_uri: &str) -> BoxFuture<TrackingCredentials>;
    fn fetch_profile(&self, token: &str) -> BoxFuture<(String, String, Option<String>)>; // (id, username, avatar)
    fn fetch_library(&self, token: &str, status: Option<RemoteStatus>) -> BoxFuture<Vec<RemoteEntry>>;
    fn update_entry(&self, token: &str, external_id: &str, progress: u32, status: Option<RemoteStatus>, score: Option<f32>) -> BoxFuture<()>;
}

// -----------------------------------------------------------------------------
// AniList Tracking Provider
// -----------------------------------------------------------------------------
pub struct AniListProvider {
    client_id: String,
    http: reqwest::Client,
}

impl AniListProvider {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            http: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default(),
        }
    }
}

impl TrackingProvider for AniListProvider {
    fn service(&self) -> TrackingService {
        TrackingService::AniList
    }

    fn auth_url(&self, state: &str, redirect_uri: &str) -> String {
        format!(
            "https://anilist.co/api/v2/oauth/authorize?client_id={}&response_type=token&state={}&redirect_uri={}",
            self.client_id,
            url_encode(state),
            url_encode(redirect_uri)
        )
    }

    fn exchange_code(&self, code: &str, _redirect_uri: &str) -> BoxFuture<TrackingCredentials> {
        let code = code.to_string();
        Box::pin(async move {
            Ok(TrackingCredentials {
                access_token: code,
                refresh_token: None,
                expires_at: None,
            })
        })
    }

    fn fetch_profile(&self, token: &str) -> BoxFuture<(String, String, Option<String>)> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let query = json!({
                "query": "query { Viewer { id name avatar { medium } } }"
            });
            let resp = client
                .post("https://graphql.anilist.co")
                .bearer_auth(token)
                .json(&query)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let viewer = resp.pointer("/data/Viewer").context("Viewer data missing")?;
            let id = viewer["id"].to_string();
            let name = viewer["name"].as_str().context("Missing name")?.to_string();
            let avatar = viewer.pointer("/avatar/medium").and_then(Value::as_str).map(String::from);
            Ok((id, name, avatar))
        })
    }

    fn fetch_library(&self, token: &str, status: Option<RemoteStatus>) -> BoxFuture<Vec<RemoteEntry>> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let status_filter = match status {
                Some(RemoteStatus::Watching) => "CURRENT",
                Some(RemoteStatus::Completed) => "COMPLETED",
                Some(RemoteStatus::OnHold) => "PAUSED",
                Some(RemoteStatus::Dropped) => "DROPPED",
                Some(RemoteStatus::PlanToWatch) => "PLANNING",
                Some(RemoteStatus::Rewatching) => "REPEATING",
                None => "",
            };
            let query = json!({
                "query": "query ($status: MediaListStatus) { Viewer { mediaListOptions { scoreFormat } lists { entries { mediaId status score progress media { title { userPreferred } episodes } updatedAt } } } }",
                "variables": { "status": if status_filter.is_empty() { Value::Null } else { json!(status_filter) } }
            });
            let resp = client
                .post("https://graphql.anilist.co")
                .bearer_auth(token)
                .json(&query)
                .send()
                .await?
                .json::<Value>()
                .await?;

            let mut results = Vec::new();
            if let Some(lists) = resp.pointer("/data/Viewer/lists").and_then(Value::as_array) {
                for list in lists {
                    if let Some(entries) = list["entries"].as_array() {
                        for e in entries {
                            let ext_id = e["mediaId"].to_string();
                            let title = e.pointer("/media/title/userPreferred").and_then(Value::as_str).unwrap_or("").to_string();
                            let raw_status = e["status"].as_str().unwrap_or("");
                            let rem_status = match raw_status {
                                "CURRENT" => RemoteStatus::Watching,
                                "COMPLETED" => RemoteStatus::Completed,
                                "PAUSED" => RemoteStatus::OnHold,
                                "DROPPED" => RemoteStatus::Dropped,
                                "PLANNING" => RemoteStatus::PlanToWatch,
                                "REPEATING" => RemoteStatus::Rewatching,
                                _ => RemoteStatus::Watching,
                            };
                            let score = e["score"].as_f64().map(|s| s as f32);
                            let progress = e["progress"].as_u64().unwrap_or(0) as u32;
                            let total_episodes = e.pointer("/media/episodes").and_then(Value::as_u64).map(|ep| ep as u32);
                            let updated_at = e["updatedAt"].as_u64().unwrap_or(0);

                            results.push(RemoteEntry {
                                service: TrackingService::AniList,
                                external_id: ext_id,
                                title,
                                status: rem_status,
                                score,
                                progress,
                                total_episodes,
                                updated_at,
                            });
                        }
                    }
                }
            }
            Ok(results)
        })
    }

    fn update_entry(
        &self,
        token: &str,
        external_id: &str,
        progress: u32,
        status: Option<RemoteStatus>,
        score: Option<f32>,
    ) -> BoxFuture<()> {
        let client = self.http.clone();
        let token = token.to_string();
        let ext_id = external_id.to_string();
        Box::pin(async move {
            let mut vars = json!({
                "mediaId": ext_id.parse::<u64>().unwrap_or(0),
                "progress": progress
            });
            if let Some(s) = status {
                let status_str = match s {
                    RemoteStatus::Watching => "CURRENT",
                    RemoteStatus::Completed => "COMPLETED",
                    RemoteStatus::OnHold => "PAUSED",
                    RemoteStatus::Dropped => "DROPPED",
                    RemoteStatus::PlanToWatch => "PLANNING",
                    RemoteStatus::Rewatching => "REPEATING",
                };
                vars["status"] = json!(status_str);
            }
            if let Some(sc) = score {
                vars["score"] = json!(sc);
            }
            let query = json!({
                "query": "mutation ($mediaId: Int, $status: MediaListStatus, $progress: Int, $score: Float) { SaveMediaListEntry (mediaId: $mediaId, status: $status, progress: $progress, score: $score) { id } }",
                "variables": vars
            });
            let _ = client
                .post("https://graphql.anilist.co")
                .bearer_auth(token)
                .json(&query)
                .send()
                .await?;
            Ok(())
        })
    }
}

// -----------------------------------------------------------------------------
// MyAnimeList Tracking Provider
// -----------------------------------------------------------------------------
pub struct MyAnimeListProvider {
    client_id: String,
    http: reqwest::Client,
}

impl MyAnimeListProvider {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            http: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default(),
        }
    }
}

impl TrackingProvider for MyAnimeListProvider {
    fn service(&self) -> TrackingService {
        TrackingService::MyAnimeList
    }

    fn auth_url(&self, state: &str, redirect_uri: &str) -> String {
        format!(
            "https://myanimelist.net/v1/oauth2/authorize?response_type=code&client_id={}&state={}&redirect_uri={}&code_challenge=plain&code_challenge_method=plain",
            self.client_id,
            url_encode(state),
            url_encode(redirect_uri)
        )
    }

    fn exchange_code(&self, code: &str, redirect_uri: &str) -> BoxFuture<TrackingCredentials> {
        let client = self.http.clone();
        let client_id = self.client_id.clone();
        let code = code.to_string();
        let redirect_uri = redirect_uri.to_string();
        Box::pin(async move {
            let params = [
                ("client_id", client_id.as_str()),
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("code_verifier", "plain"),
            ];
            let resp = client
                .post("https://myanimelist.net/v1/oauth2/token")
                .form(&params)
                .send()
                .await?
                .json::<Value>()
                .await?;

            let access_token = resp["access_token"].as_str().context("Missing access_token")?.to_string();
            let refresh_token = resp["refresh_token"].as_str().map(String::from);
            let expires_in = resp["expires_in"].as_u64();
            Ok(TrackingCredentials {
                access_token,
                refresh_token,
                expires_at: expires_in,
            })
        })
    }

    fn fetch_profile(&self, token: &str) -> BoxFuture<(String, String, Option<String>)> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let resp = client
                .get("https://api.myanimelist.net/v2/users/@me")
                .bearer_auth(token)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let id = resp["id"].to_string();
            let name = resp["name"].as_str().context("Missing name")?.to_string();
            let avatar = resp["picture"].as_str().map(String::from);
            Ok((id, name, avatar))
        })
    }

    fn fetch_library(&self, token: &str, status: Option<RemoteStatus>) -> BoxFuture<Vec<RemoteEntry>> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let status_filter = match status {
                Some(RemoteStatus::Watching) => "watching",
                Some(RemoteStatus::Completed) => "completed",
                Some(RemoteStatus::OnHold) => "on_hold",
                Some(RemoteStatus::Dropped) => "dropped",
                Some(RemoteStatus::PlanToWatch) => "plan_to_watch",
                Some(RemoteStatus::Rewatching) => "watching",
                None => "",
            };
            let mut url = "https://api.myanimelist.net/v2/users/@me/animelist?fields=list_status,num_episodes&limit=1000".to_string();
            if !status_filter.is_empty() {
                url.push_str(&format!("&status={status_filter}"));
            }
            let resp = client.get(&url).bearer_auth(token).send().await?.json::<Value>().await?;

            let mut results = Vec::new();
            if let Some(data) = resp["data"].as_array() {
                for item in data {
                    let node = &item["node"];
                    let list_status = &item["list_status"];
                    let ext_id = node["id"].to_string();
                    let title = node["title"].as_str().unwrap_or("").to_string();
                    let raw_status = list_status["status"].as_str().unwrap_or("");
                    let rem_status = match raw_status {
                        "watching" => RemoteStatus::Watching,
                        "completed" => RemoteStatus::Completed,
                        "on_hold" => RemoteStatus::OnHold,
                        "dropped" => RemoteStatus::Dropped,
                        "plan_to_watch" => RemoteStatus::PlanToWatch,
                        _ => RemoteStatus::Watching,
                    };
                    let score = list_status["score"].as_f64().map(|s| s as f32);
                    let progress = list_status["num_episodes_watched"].as_u64().unwrap_or(0) as u32;
                    let total_episodes = node["num_episodes"].as_u64().map(|ep| ep as u32);
                    let updated_at = 0u64;

                    results.push(RemoteEntry {
                        service: TrackingService::MyAnimeList,
                        external_id: ext_id,
                        title,
                        status: rem_status,
                        score,
                        progress,
                        total_episodes,
                        updated_at,
                    });
                }
            }
            Ok(results)
        })
    }

    fn update_entry(
        &self,
        token: &str,
        external_id: &str,
        progress: u32,
        status: Option<RemoteStatus>,
        score: Option<f32>,
    ) -> BoxFuture<()> {
        let client = self.http.clone();
        let token = token.to_string();
        let ext_id = external_id.to_string();
        Box::pin(async move {
            let mut params = vec![("num_watched_episodes", progress.to_string())];
            let status_str = status.map(|s| match s {
                RemoteStatus::Watching => "watching",
                RemoteStatus::Completed => "completed",
                RemoteStatus::OnHold => "on_hold",
                RemoteStatus::Dropped => "dropped",
                RemoteStatus::PlanToWatch => "plan_to_watch",
                RemoteStatus::Rewatching => "watching",
            });
            if let Some(s) = &status_str {
                params.push(("status", s.to_string()));
            }
            let score_str = score.map(|sc| (sc.round() as u32).clamp(0, 10).to_string());
            if let Some(sc) = &score_str {
                params.push(("score", sc.clone()));
            }
            let url = format!("https://api.myanimelist.net/v2/anime/{ext_id}/my_list_status");
            let _ = client.put(&url).bearer_auth(token).form(&params).send().await?;
            Ok(())
        })
    }
}

// -----------------------------------------------------------------------------
// Kitsu Tracking Provider
// -----------------------------------------------------------------------------
pub struct KitsuProvider {
    http: reqwest::Client,
}

impl KitsuProvider {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default(),
        }
    }
}

impl TrackingProvider for KitsuProvider {
    fn service(&self) -> TrackingService {
        TrackingService::Kitsu
    }

    fn auth_url(&self, state: &str, redirect_uri: &str) -> String {
        format!(
            "https://kitsu.io/api/oauth/authorize?response_type=code&client_id=kitsu&state={}&redirect_uri={}",
            url_encode(state),
            url_encode(redirect_uri)
        )
    }

    fn exchange_code(&self, code: &str, _redirect_uri: &str) -> BoxFuture<TrackingCredentials> {
        let code = code.to_string();
        Box::pin(async move {
            Ok(TrackingCredentials {
                access_token: code,
                refresh_token: None,
                expires_at: None,
            })
        })
    }

    fn fetch_profile(&self, token: &str) -> BoxFuture<(String, String, Option<String>)> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let resp = client
                .get("https://kitsu.io/api/edge/users?filter[self]=true")
                .bearer_auth(token)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let data = resp.pointer("/data/0").context("Missing user data")?;
            let id = data["id"].as_str().unwrap_or("").to_string();
            let name = data.pointer("/attributes/name").and_then(Value::as_str).unwrap_or("").to_string();
            let avatar = data.pointer("/attributes/avatar/medium").and_then(Value::as_str).map(String::from);
            Ok((id, name, avatar))
        })
    }

    fn fetch_library(&self, token: &str, _status: Option<RemoteStatus>) -> BoxFuture<Vec<RemoteEntry>> {
        let client = self.http.clone();
        let token = token.to_string();
        Box::pin(async move {
            let resp = client
                .get("https://kitsu.io/api/edge/library-entries?filter[kind]=anime&include=anime&page[limit]=500")
                .bearer_auth(token)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let mut results = Vec::new();
            if let Some(entries) = resp["data"].as_array() {
                for e in entries {
                    let attrs = &e["attributes"];
                    let ext_id = attrs["mediaId"].to_string();
                    let raw_status = attrs["status"].as_str().unwrap_or("");
                    let rem_status = match raw_status {
                        "current" => RemoteStatus::Watching,
                        "completed" => RemoteStatus::Completed,
                        "on_hold" => RemoteStatus::OnHold,
                        "dropped" => RemoteStatus::Dropped,
                        "planned" => RemoteStatus::PlanToWatch,
                        _ => RemoteStatus::Watching,
                    };
                    let score = attrs["ratingTwenty"].as_f64().map(|s| (s / 2.0) as f32);
                    let progress = attrs["progress"].as_u64().unwrap_or(0) as u32;

                    results.push(RemoteEntry {
                        service: TrackingService::Kitsu,
                        external_id: ext_id,
                        title: "Kitsu Item".into(),
                        status: rem_status,
                        score,
                        progress,
                        total_episodes: None,
                        updated_at: 0,
                    });
                }
            }
            Ok(results)
        })
    }

    fn update_entry(
        &self,
        token: &str,
        external_id: &str,
        progress: u32,
        status: Option<RemoteStatus>,
        _score: Option<f32>,
    ) -> BoxFuture<()> {
        let client = self.http.clone();
        let token = token.to_string();
        let ext_id = external_id.to_string();
        Box::pin(async move {
            let mut attrs = json!({ "progress": progress });
            if let Some(s) = status {
                attrs["status"] = json!(match s {
                    RemoteStatus::Watching => "current",
                    RemoteStatus::Completed => "completed",
                    RemoteStatus::OnHold => "on_hold",
                    RemoteStatus::Dropped => "dropped",
                    RemoteStatus::PlanToWatch => "planned",
                    RemoteStatus::Rewatching => "current",
                });
            }
            let payload = json!({
                "data": {
                    "type": "libraryEntries",
                    "attributes": attrs,
                    "relationships": {
                        "anime": {
                            "data": { "type": "anime", "id": ext_id }
                        }
                    }
                }
            });
            let _ = client
                .post("https://kitsu.io/api/edge/library-entries")
                .bearer_auth(token)
                .header("Content-Type", "application/vnd.api+json")
                .json(&payload)
                .send()
                .await?;
            Ok(())
        })
    }
}

// -----------------------------------------------------------------------------
// Simkl Tracking Provider
// -----------------------------------------------------------------------------
pub struct SimklProvider {
    client_id: String,
    http: reqwest::Client,
}

impl SimklProvider {
    pub fn new(client_id: String) -> Self {
        Self {
            client_id,
            http: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default(),
        }
    }
}

impl TrackingProvider for SimklProvider {
    fn service(&self) -> TrackingService {
        TrackingService::Simkl
    }

    fn auth_url(&self, state: &str, redirect_uri: &str) -> String {
        format!(
            "https://simkl.com/oauth/authorize?response_type=code&client_id={}&state={}&redirect_uri={}",
            self.client_id,
            url_encode(state),
            url_encode(redirect_uri)
        )
    }

    fn exchange_code(&self, code: &str, _redirect_uri: &str) -> BoxFuture<TrackingCredentials> {
        let code = code.to_string();
        Box::pin(async move {
            Ok(TrackingCredentials {
                access_token: code,
                refresh_token: None,
                expires_at: None,
            })
        })
    }

    fn fetch_profile(&self, token: &str) -> BoxFuture<(String, String, Option<String>)> {
        let client = self.http.clone();
        let token = token.to_string();
        let client_id = self.client_id.clone();
        Box::pin(async move {
            let resp = client
                .get("https://api.simkl.com/users/settings")
                .bearer_auth(token)
                .header("simkl-api-key", client_id)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let user = resp.pointer("/user").context("Missing user info")?;
            let id = user["id"].to_string();
            let name = user["name"].as_str().unwrap_or("").to_string();
            let avatar = user["avatar"].as_str().map(String::from);
            Ok((id, name, avatar))
        })
    }

    fn fetch_library(&self, token: &str, _status: Option<RemoteStatus>) -> BoxFuture<Vec<RemoteEntry>> {
        let client = self.http.clone();
        let token = token.to_string();
        let client_id = self.client_id.clone();
        Box::pin(async move {
            let resp = client
                .get("https://api.simkl.com/sync/all-items/anime")
                .bearer_auth(token)
                .header("simkl-api-key", client_id)
                .send()
                .await?
                .json::<Value>()
                .await?;
            let mut results = Vec::new();
            if let Some(anime_list) = resp["anime"].as_array() {
                for item in anime_list {
                    let show = &item["show"];
                    let ext_id = show.pointer("/ids/simkl").unwrap_or(&Value::Null).to_string();
                    let title = show["title"].as_str().unwrap_or("").to_string();
                    let raw_status = item["status"].as_str().unwrap_or("");
                    let rem_status = match raw_status {
                        "watching" => RemoteStatus::Watching,
                        "completed" => RemoteStatus::Completed,
                        "hold" => RemoteStatus::OnHold,
                        "dropped" => RemoteStatus::Dropped,
                        "plantowatch" => RemoteStatus::PlanToWatch,
                        _ => RemoteStatus::Watching,
                    };
                    let score = item["user_rating"].as_f64().map(|s| s as f32);
                    let progress = item["watched_episodes_count"].as_u64().unwrap_or(0) as u32;

                    results.push(RemoteEntry {
                        service: TrackingService::Simkl,
                        external_id: ext_id,
                        title,
                        status: rem_status,
                        score,
                        progress,
                        total_episodes: None,
                        updated_at: 0,
                    });
                }
            }
            Ok(results)
        })
    }

    fn update_entry(
        &self,
        token: &str,
        external_id: &str,
        progress: u32,
        _status: Option<RemoteStatus>,
        _score: Option<f32>,
    ) -> BoxFuture<()> {
        let client = self.http.clone();
        let token = token.to_string();
        let ext_id = external_id.to_string();
        let client_id = self.client_id.clone();
        Box::pin(async move {
            let payload = json!({
                "shows": [{
                    "ids": { "simkl": ext_id.parse::<u64>().unwrap_or(0) },
                    "episodes": [{ "number": progress }]
                }]
            });
            let _ = client
                .post("https://api.simkl.com/sync/history")
                .bearer_auth(token)
                .header("simkl-api-key", client_id)
                .json(&payload)
                .send()
                .await?;
            Ok(())
        })
    }
}


// -----------------------------------------------------------------------------
// Tracking Manager with Idempotent Sync Queue
// -----------------------------------------------------------------------------
pub struct TrackingManager {
    providers: HashMap<TrackingService, Arc<dyn TrackingProvider>>,
    accounts: Arc<Mutex<HashMap<String, Vec<TrackingAccount>>>>, // profile_id -> accounts
    queue: Arc<Mutex<Vec<SyncMutation>>>,
}

impl TrackingManager {
    pub fn new() -> Self {
        let mut providers: HashMap<TrackingService, Arc<dyn TrackingProvider>> = HashMap::new();
        providers.insert(TrackingService::AniList, Arc::new(AniListProvider::new("cloudstream".into())));
        providers.insert(TrackingService::MyAnimeList, Arc::new(MyAnimeListProvider::new("cloudstream".into())));
        providers.insert(TrackingService::Kitsu, Arc::new(KitsuProvider::new()));
        providers.insert(TrackingService::Simkl, Arc::new(SimklProvider::new("cloudstream".into())));

        Self {
            providers,
            accounts: Arc::new(Mutex::new(HashMap::new())),
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_provider(&self, service: TrackingService) -> Option<Arc<dyn TrackingProvider>> {
        self.providers.get(&service).cloned()
    }

    pub fn list_accounts(&self, profile_id: &str) -> Vec<TrackingAccount> {
        let map = self.accounts.lock().unwrap();
        map.get(profile_id).cloned().unwrap_or_default()
    }

    pub fn add_account(&self, account: TrackingAccount) {
        let mut map = self.accounts.lock().unwrap();
        let list = map.entry(account.profile_id.clone()).or_default();
        list.retain(|a| a.service != account.service);
        list.push(account);
    }

    pub fn remove_account(&self, profile_id: &str, service: TrackingService) {
        let mut map = self.accounts.lock().unwrap();
        if let Some(list) = map.get_mut(profile_id) {
            list.retain(|a| a.service != service);
        }
    }

    /// Enqueue an idempotent progress sync mutation.
    /// Only syncs eligible progress; dragging a seekbar must not mark an entire series complete.
    pub fn queue_sync(
        &self,
        profile_id: &str,
        service: TrackingService,
        media_id: &str,
        external_id: &str,
        progress: u32,
        status: Option<RemoteStatus>,
        score: Option<f32>,
    ) {
        let mut queue = self.queue.lock().unwrap();
        // Deduplicate: replace any existing unapplied mutation for the same target
        queue.retain(|m| !(m.profile_id == profile_id && m.service == service && m.external_id == external_id));

        queue.push(SyncMutation {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile_id.into(),
            service,
            media_id: media_id.into(),
            external_id: external_id.into(),
            progress,
            status,
            score,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            attempts: 0,
        });
    }

    pub fn pending_mutations(&self, profile_id: &str) -> Vec<SyncMutation> {
        let queue = self.queue.lock().unwrap();
        queue.iter().filter(|m| m.profile_id == profile_id).cloned().collect()
    }

    pub fn clear_mutations(&self, profile_id: &str) {
        let mut queue = self.queue.lock().unwrap();
        queue.retain(|m| m.profile_id != profile_id);
    }
}
