//! OpenSubtitles.com (REST API v1).
//!
//! Needs three things from the person: an API key from their own OpenSubtitles API consumer, and
//! their account name and password (downloads require a signed-in account). The app ships no key
//! of its own. Sign-in gives a token that expires; it is refreshed with the saved password the
//! first time a download is refused as unauthorised. Query parameters are sent lower-case and
//! sorted, which the service requires.
use super::http::{allowed_url, HttpRequest, MAX_DOWNLOAD_BYTES};
use super::*;
use serde_json::{json, Value};

pub struct OpenSubtitles;

const DEFAULT_BASE: &str = "https://api.opensubtitles.com/api/v1";

fn base(credentials: &Credentials) -> String {
    // The sign-in reply names the host to use from then on; accept it only on the service's domain.
    credentials
        .get("baseUrl")
        .map(|host| format!("https://{}/api/v1", host.trim().trim_start_matches("https://").trim_end_matches('/')))
        .filter(|url| allowed_url(url).is_some())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn authorised(request: HttpRequest, credentials: &Credentials, with_token: bool) -> HttpRequest {
    let mut request = request.header("api-key", credentials.get("apiKey").cloned().unwrap_or_default()).header("accept", "application/json");
    if let (true, Some(token)) = (with_token, credentials.get("token").filter(|t| !t.is_empty())) {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request
}

/// Turn a refusal into the kind of problem it is. `None` means the status is not a refusal.
fn refusal(response: &HttpResponse) -> Option<ProviderProblem> {
    match response.status {
        200..=299 => None,
        401 | 403 => Some(ProviderProblem::SignInRejected),
        406 => Some(ProviderProblem::QuotaExhausted { retry_after_secs: response.retry_after() }),
        429 => Some(ProviderProblem::RateLimited { retry_after_secs: response.retry_after() }),
        500..=599 => Some(ProviderProblem::Unavailable),
        _ => Some(ProviderProblem::BadResponse),
    }
}

async fn log_in(http: &dyn Transport, credentials: &mut Credentials) -> Result<(), ProviderProblem> {
    let (user, password) = match (credentials.get("username"), credentials.get("password")) {
        (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => (u.clone(), p.clone()),
        _ => return Err(ProviderProblem::NotConnected),
    };
    if credentials.get("apiKey").is_none_or(|k| k.is_empty()) {
        return Err(ProviderProblem::NotConnected);
    }
    let request = authorised(HttpRequest::post_json(format!("{}/login", base(credentials)), &json!({ "username": user, "password": password })), credentials, false);
    let response = http.send(request).await?;
    if response.status == 400 || response.status == 422 {
        // The service answers a wrong user name or password this way.
        return Err(ProviderProblem::SignInRejected);
    }
    if let Some(problem) = refusal(&response) {
        return Err(problem);
    }
    let body = response.json()?;
    let token = body.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or(ProviderProblem::BadResponse)?;
    credentials.insert("token".into(), token.to_string());
    if let Some(host) = body.get("base_url").and_then(Value::as_str) {
        if allowed_url(&format!("https://{}/", host.trim())).is_some() {
            credentials.insert("baseUrl".into(), host.trim().to_string());
        }
    }
    Ok(())
}

impl SubtitleProvider for OpenSubtitles {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "opensubtitles",
            name: "OpenSubtitles",
            description: "Large community subtitle library. Needs your own API key and a free account.",
            site: "https://www.opensubtitles.com",
            fields: vec![
                Field { name: "apiKey", label: "API key", secret: true, required: true, help: "Create one under API consumers in your OpenSubtitles account." },
                Field { name: "username", label: "Username", secret: false, required: true, help: "Your OpenSubtitles account name." },
                Field { name: "password", label: "Password", secret: true, required: true, help: "Stored in the system keyring, used only to sign in to OpenSubtitles." },
            ],
            movies: true,
            episodes: true,
        }
    }

    fn connect<'a>(&'a self, http: &'a dyn Transport, fields: &'a Credentials) -> BoxFuture<'a, Result<Credentials, ProviderProblem>> {
        Box::pin(async move {
            let mut credentials: Credentials = ["apiKey", "username", "password"].iter().filter_map(|k| fields.get(*k).map(|v| (k.to_string(), v.trim().to_string()))).collect();
            log_in(http, &mut credentials).await?;
            Ok(credentials)
        })
    }

    fn search<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, query: &'a SubtitleQuery) -> BoxFuture<'a, Result<Vec<Candidate>, ProviderProblem>> {
        Box::pin(async move {
            if credentials.get("apiKey").is_none_or(|k| k.is_empty()) {
                return Err(ProviderProblem::NotConnected);
            }
            let query = query.cleaned();
            // Alphabetical order comes from the sorted map.
            let mut params: BTreeMap<&str, String> = BTreeMap::new();
            params.insert("ai_translated", "exclude".into());
            params.insert("machine_translated", "exclude".into());
            if !query.languages.is_empty() {
                params.insert("languages", query.languages.join(","));
            }
            match query.hearing_impaired {
                HearingImpaired::Include => {}
                HearingImpaired::Exclude => {
                    params.insert("hearing_impaired", "exclude".into());
                }
                HearingImpaired::Only => {
                    params.insert("hearing_impaired", "only".into());
                }
            }
            let has_ids = query.imdb_id.is_some() || query.tmdb_id.is_some();
            if query.is_episode() {
                params.insert("type", "episode".into());
                if let Some(season) = query.season {
                    params.insert("season_number", season.to_string());
                }
                params.insert("episode_number", query.episode.unwrap_or_default().to_string());
                // The IDs we hold belong to the series, not to the episode.
                if let Some(id) = query.imdb_id.as_deref().and_then(imdb_digits) {
                    params.insert("parent_imdb_id", id.trim_start_matches('0').to_string());
                }
                if let Some(id) = &query.tmdb_id {
                    params.insert("parent_tmdb_id", id.clone());
                }
            } else {
                params.insert("type", "movie".into());
                if let Some(id) = query.imdb_id.as_deref().and_then(imdb_digits) {
                    params.insert("imdb_id", id.trim_start_matches('0').to_string());
                }
                if let Some(id) = &query.tmdb_id {
                    params.insert("tmdb_id", id.clone());
                }
                if let (Some(year), false) = (query.year, has_ids) {
                    params.insert("year", year.to_string());
                }
            }
            if !has_ids {
                if query.title.is_empty() {
                    return Ok(Vec::new());
                }
                params.insert("query", query.title.to_lowercase());
            }
            let mut url = url::Url::parse(&format!("{}/subtitles", base(credentials))).map_err(|_| ProviderProblem::BadResponse)?;
            for (name, value) in &params {
                url.query_pairs_mut().append_pair(name, value);
            }
            let response = http.send(authorised(HttpRequest::get(url.as_str()), credentials, false)).await?;
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            let body = response.json()?;
            let items = body.get("data").and_then(Value::as_array).ok_or(ProviderProblem::BadResponse)?;
            let mut candidates = Vec::new();
            for item in items {
                let Some(attributes) = item.get("attributes") else { continue };
                let Some(file_id) = attributes.get("files").and_then(Value::as_array).and_then(|f| f.first()).and_then(|f| f.get("file_id")).and_then(|v| v.as_u64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string))) else { continue };
                let feature = attributes.get("feature_details").cloned().unwrap_or(Value::Null);
                let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(|s| clean_text(s, 160)).filter(|s| !s.is_empty());
                let flag = |key: &str| attributes.get(key).and_then(Value::as_bool).unwrap_or(false);
                let is_episode_item = feature.get("episode_number").is_some_and(|v| !v.is_null());
                candidates.push(Candidate {
                    provider: "opensubtitles".into(),
                    id: file_id,
                    language: text(attributes, "language").unwrap_or_default().to_lowercase(),
                    release: text(attributes, "release").or_else(|| text(attributes, "comments")).unwrap_or_default(),
                    hearing_impaired: flag("hearing_impaired"),
                    forced: flag("foreign_parts_only"),
                    ai_translated: flag("ai_translated") || flag("machine_translated"),
                    season: feature.get("season_number").and_then(Value::as_i64).map(|n| n as i32).filter(|n| *n > 0),
                    episode: feature.get("episode_number").and_then(Value::as_i64).map(|n| n as i32).filter(|n| *n > 0),
                    pack: false,
                    downloads: attributes.get("download_count").and_then(Value::as_u64),
                    uploader: attributes.get("uploader").and_then(|u| text(u, "name")),
                    matched_title: if is_episode_item { text(&feature, "parent_title").or_else(|| text(&feature, "title")) } else { text(&feature, "title").or_else(|| text(&feature, "movie_name")) },
                    matched_year: feature.get("year").and_then(Value::as_i64).map(|n| n as i32),
                    locator: BTreeMap::new(),
                });
            }
            let mut candidates = keep_hearing_impaired(&query, keep_requested_episode(&query, candidates));
            rank(&query, &mut candidates);
            Ok(candidates)
        })
    }

    fn download<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded, ProviderProblem>> {
        Box::pin(async move {
            let file_id: u64 = candidate.id.parse().map_err(|_| ProviderProblem::BadResponse)?;
            if credentials.get("token").is_none_or(|t| t.is_empty()) {
                log_in(http, credentials).await?;
            }
            let mut refreshed = false;
            let body = loop {
                let request = authorised(HttpRequest::post_json(format!("{}/download", base(credentials)), &json!({ "file_id": file_id, "sub_format": "srt" })), credentials, true);
                let response = http.send(request).await?;
                if response.status == 401 && !refreshed {
                    // The sign-in expired: sign in again once with the saved details.
                    refreshed = true;
                    credentials.remove("token");
                    log_in(http, credentials).await?;
                    continue;
                }
                if let Some(problem) = refusal(&response) {
                    return Err(problem);
                }
                break response.json()?;
            };
            let link = body.get("link").and_then(Value::as_str).and_then(allowed_url).ok_or(ProviderProblem::BadResponse)?;
            // The file itself needs no key, so none is sent to that address.
            let file = http.send(HttpRequest::get(link.as_str()).limit(MAX_DOWNLOAD_BYTES)).await?;
            if let Some(problem) = refusal(&file) {
                return Err(problem);
            }
            if !looks_like_subtitle_payload(&file.body) {
                return Err(ProviderProblem::BadResponse);
            }
            Ok(Downloaded { file_name: body.get("file_name").and_then(Value::as_str).map(|n| clean_text(n, 120)).filter(|n| !n.is_empty()).unwrap_or_else(|| "subtitle.srt".into()), bytes: file.body, remaining: body.get("remaining").and_then(Value::as_u64) })
        })
    }
}
