//! Subdl (v1 API).
//!
//! Needs an API key from the person's own Subdl account; the app ships none. The key travels in a
//! header rather than in the address where the service allows it, so it does not end up in
//! request logs. Subdl has changed what a free key may do, so the plain answers are kept
//! distinct: "paid plan required" (HTTP 402), "daily allowance used" and "slow down" (HTTP 429)
//! are different results, as are a rejected key and an empty search.
use super::http::{allowed_url, HttpRequest, MAX_DOWNLOAD_BYTES};
use super::*;
use serde_json::Value;

pub struct Subdl;

const SEARCH: &str = "https://api.subdl.com/api/v1/subtitles";
const DOWNLOADS: &str = "https://dl.subdl.com";

fn code_for(language: &str) -> String {
    match language {
        "pt-br" => "BR_PT".into(),
        other => other.split(['-', '_']).next().unwrap_or("").to_ascii_uppercase(),
    }
}

fn language_for(code: &str) -> String {
    match code.to_ascii_uppercase().as_str() {
        "BR_PT" => "pt-br".into(),
        other => other.to_ascii_lowercase(),
    }
}

/// The service's own error code, from `error` or `message`, lower-case.
fn error_text(response: &HttpResponse) -> String {
    response.json().ok().and_then(|b| b.get("error").or_else(|| b.get("message")).and_then(Value::as_str).map(str::to_lowercase)).unwrap_or_default()
}

fn refusal(response: &HttpResponse) -> Option<ProviderProblem> {
    let text = error_text(response);
    match response.status {
        200..=299 => None,
        402 => Some(ProviderProblem::PaidPlanRequired),
        401 => Some(ProviderProblem::SignInRejected),
        403 => Some(if text.contains("key") || text.contains("auth") || text.contains("forbidden") { ProviderProblem::SignInRejected } else { ProviderProblem::Blocked }),
        404 => Some(ProviderProblem::NotFound),
        429 if text.contains("daily") || text.contains("download_limit") || text.contains("quota") => Some(ProviderProblem::QuotaExhausted { retry_after_secs: response.retry_after() }),
        429 if text.contains("busy") => Some(ProviderProblem::Unavailable),
        429 => Some(ProviderProblem::RateLimited { retry_after_secs: response.retry_after() }),
        500..=599 => Some(ProviderProblem::Unavailable),
        _ => Some(ProviderProblem::BadResponse),
    }
}

fn search_url(key: &str, query: &SubtitleQuery, per_page: u32) -> Result<String, ProviderProblem> {
    let mut url = url::Url::parse(SEARCH).map_err(|_| ProviderProblem::BadResponse)?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("api_key", key);
        pairs.append_pair("type", if query.is_episode() { "tv" } else { "movie" });
        if let Some(imdb) = &query.imdb_id {
            pairs.append_pair("imdb_id", imdb);
        } else if let Some(tmdb) = &query.tmdb_id {
            pairs.append_pair("tmdb_id", tmdb);
        } else {
            pairs.append_pair("film_name", &query.title);
            if let Some(year) = query.year.filter(|_| !query.is_episode()) {
                pairs.append_pair("year", &year.to_string());
            }
        }
        if let Some(season) = query.season {
            pairs.append_pair("season_number", &season.to_string());
        }
        if let Some(episode) = query.episode {
            pairs.append_pair("episode_number", &episode.to_string());
        }
        if !query.languages.is_empty() {
            pairs.append_pair("languages", &query.languages.iter().map(|l| code_for(l)).collect::<Vec<_>>().join(","));
        }
        pairs.append_pair("releases", "1");
        pairs.append_pair("comment", "1");
        pairs.append_pair("subs_per_page", &per_page.to_string());
    }
    Ok(url.into())
}

impl SubtitleProvider for Subdl {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "subdl",
            name: "Subdl",
            description: "Subtitle search by title or IMDb/TMDB ID. Needs your own API key; the service may limit free keys.",
            site: "https://subdl.com",
            fields: vec![Field { name: "apiKey", label: "API key", secret: true, required: true, help: "Create one in your Subdl account panel." }],
            movies: true,
            episodes: true,
        }
    }

    fn connect<'a>(&'a self, http: &'a dyn Transport, fields: &'a Credentials) -> BoxFuture<'a, Result<Credentials, ProviderProblem>> {
        Box::pin(async move {
            let key = fields.get("apiKey").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).ok_or(ProviderProblem::NotConnected)?;
            // A search with a known title proves the key is accepted without using a download.
            let probe = SubtitleQuery { title: "Inception".into(), ..Default::default() };
            let response = http.send(HttpRequest::get(search_url(&key, &probe, 1)?)).await?;
            if let Some(problem) = refusal(&response) {
                // The key was accepted but this plan cannot search: say so instead of "connected".
                return Err(problem);
            }
            let body = response.json()?;
            if body.get("status").and_then(Value::as_bool) == Some(false) {
                let text = body.get("error").and_then(Value::as_str).unwrap_or("").to_lowercase();
                if text.contains("key") || text.contains("auth") {
                    return Err(ProviderProblem::SignInRejected);
                }
            }
            Ok(Credentials::from([("apiKey".to_string(), key)]))
        })
    }

    fn search<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, query: &'a SubtitleQuery) -> BoxFuture<'a, Result<Vec<Candidate>, ProviderProblem>> {
        Box::pin(async move {
            let key = credentials.get("apiKey").filter(|k| !k.is_empty()).cloned().ok_or(ProviderProblem::NotConnected)?;
            let query = query.cleaned();
            if query.title.is_empty() && query.imdb_id.is_none() && query.tmdb_id.is_none() {
                return Ok(Vec::new());
            }
            let response = http.send(HttpRequest::get(search_url(&key, &query, 30)?)).await?;
            if let Some(problem) = refusal(&response) {
                return if problem == ProviderProblem::NotFound { Ok(Vec::new()) } else { Err(problem) };
            }
            let body = response.json()?;
            let failed = body.get("status").and_then(Value::as_bool) == Some(false) || body.get("success").and_then(Value::as_bool) == Some(false);
            if failed {
                let text = body.get("error").and_then(Value::as_str).unwrap_or("").to_lowercase();
                return if text.contains("can't find") || text.contains("cannot find") || text.contains("not found") || text.contains("no subtitle") {
                    Ok(Vec::new())
                } else if text.contains("key") || text.contains("auth") {
                    Err(ProviderProblem::SignInRejected)
                } else {
                    Err(ProviderProblem::BadResponse)
                };
            }
            let items = body.get("subtitles").and_then(Value::as_array).ok_or(ProviderProblem::BadResponse)?;
            // Subtitles are not tied to a title in the reply, so a title is shown only when the
            // service matched exactly one.
            let titles = body.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
            let (matched_title, matched_year) = match titles.as_slice() {
                [only] => (only.get("name").and_then(Value::as_str).map(|n| clean_text(n, 160)), only.get("year").and_then(Value::as_i64).map(|y| y as i32)),
                _ => (None, None),
            };
            let mut candidates = Vec::new();
            for item in items {
                let Some(path) = item.get("url").and_then(Value::as_str).filter(|u| u.starts_with("/subtitle/") && !u.contains("..")) else { continue };
                let language = item.get("language").or_else(|| item.get("lang")).and_then(Value::as_str).map(language_for).unwrap_or_default();
                let release = item.get("releases").and_then(Value::as_array).and_then(|r| r.first()).and_then(Value::as_str).or_else(|| item.get("release_name").and_then(Value::as_str)).or_else(|| item.get("name").and_then(Value::as_str)).map(|s| clean_text(s, 200)).unwrap_or_default();
                let number = |key: &str| item.get(key).and_then(Value::as_i64).map(|n| n as i32).filter(|n| *n > 0);
                let season = number("season");
                let episode = number("episode");
                let from_release = season_episode_from_release(&release);
                let (from, to) = (number("episode_from"), number("episode_end"));
                let covers_range = matches!((from, to, query.episode), (Some(a), Some(b), Some(e)) if a <= e && e <= b);
                let full_season = item.get("full_season").and_then(Value::as_bool).unwrap_or(false);
                candidates.push(Candidate {
                    provider: "subdl".into(),
                    id: path.to_string(),
                    language,
                    release,
                    hearing_impaired: item.get("hi").and_then(Value::as_bool).unwrap_or(false),
                    forced: false,
                    ai_translated: item.get("ai_translated").and_then(Value::as_bool).unwrap_or(false),
                    season: season.or(from_release.0),
                    episode: if full_season { None } else { episode.or(from_release.1) },
                    pack: full_season || (episode.is_none() && (covers_range || from_release.1.is_none() && season.or(from_release.0).is_some() && query.is_episode())),
                    downloads: None,
                    uploader: item.get("author").and_then(Value::as_str).map(|a| clean_text(a, 60)).filter(|a| !a.is_empty()),
                    matched_title: matched_title.clone(),
                    matched_year,
                    locator: BTreeMap::new(),
                });
            }
            let mut candidates = keep_hearing_impaired(&query, keep_requested_episode(&query, candidates));
            // A pack that does not cover the requested episode range is not offered as a match.
            candidates.retain(|c| !c.pack || c.season.is_some() || !query.is_episode());
            rank(&query, &mut candidates);
            Ok(candidates)
        })
    }

    fn download<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded, ProviderProblem>> {
        Box::pin(async move {
            let key = credentials.get("apiKey").filter(|k| !k.is_empty()).cloned().ok_or(ProviderProblem::NotConnected)?;
            if !candidate.id.starts_with("/subtitle/") || candidate.id.contains("..") {
                return Err(ProviderProblem::BadResponse);
            }
            let url = allowed_url(&format!("{DOWNLOADS}{}", candidate.id)).ok_or(ProviderProblem::BadResponse)?;
            let response = http.send(HttpRequest::get(url.as_str()).header("x-api-key", key).limit(MAX_DOWNLOAD_BYTES)).await?;
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            if !looks_like_subtitle_payload(&response.body) {
                return Err(ProviderProblem::BadResponse);
            }
            let name = clean_text(candidate.id.rsplit('/').next().unwrap_or("subtitle.zip"), 120);
            let remaining = response.header("x-ratelimit-remaining").and_then(|v| v.trim().parse().ok());
            Ok(Downloaded { file_name: name, bytes: response.body, remaining })
        })
    }
}
