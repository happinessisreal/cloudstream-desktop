//! SubSource (v1 API).
//!
//! Needs an API key from the person's own SubSource account. SubSource lists subtitles per title
//! and per language, so a search first finds the title (by IMDb ID when known, otherwise by
//! exact title text and year) and then lists that title's subtitles. If the title cannot be
//! identified exactly, the answer is "no results": a similarly named title is never substituted.
//! Only languages whose SubSource names are known are requested.
use super::http::{HttpRequest, MAX_DOWNLOAD_BYTES};
use super::*;
use serde_json::Value;

pub struct SubSource;

const BASE: &str = "https://api.subsource.net/api/v1";

/// SubSource language names for the tags this app can ask for.
const LANGUAGES: &[(&str, &str)] = &[
    ("en", "english"),
    ("es", "spanish"),
    ("fr", "french"),
    ("de", "german"),
    ("it", "italian"),
    ("pt", "portuguese"),
    ("ru", "russian"),
    ("ja", "japanese"),
    ("ko", "korean"),
    ("zh", "chinese"),
    ("ar", "arabic"),
    ("tr", "turkish"),
    ("nl", "dutch"),
    ("pl", "polish"),
    ("sv", "swedish"),
    ("id", "indonesian"),
    ("vi", "vietnamese"),
    ("he", "hebrew"),
    ("hi", "hindi"),
    ("th", "thai"),
];

fn name_for(tag: &str) -> Option<&'static str> {
    let primary = tag.split(['-', '_']).next().unwrap_or("");
    LANGUAGES.iter().find(|(code, _)| *code == primary).map(|(_, name)| *name)
}

fn tag_for(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    LANGUAGES.iter().find(|(_, n)| *n == lower).map(|(code, _)| code.to_string()).unwrap_or(lower)
}

fn normalise(title: &str) -> String {
    title.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn refusal(response: &HttpResponse) -> Option<ProviderProblem> {
    match response.status {
        200..=299 => None,
        401 | 403 => Some(ProviderProblem::SignInRejected),
        404 => Some(ProviderProblem::NotFound),
        429 => {
            let from_body = response.json().ok().and_then(|b| b.get("retryAfter").and_then(Value::as_u64));
            Some(ProviderProblem::RateLimited { retry_after_secs: response.retry_after().or(from_body).map(|s| s.min(86_400)) })
        }
        500..=599 => Some(ProviderProblem::Unavailable),
        _ => Some(ProviderProblem::BadResponse),
    }
}

fn request(path: &str, params: &[(&str, String)], key: &str) -> Result<HttpRequest, ProviderProblem> {
    let mut url = url::Url::parse(&format!("{BASE}/{path}")).map_err(|_| ProviderProblem::BadResponse)?;
    if !params.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in params {
            pairs.append_pair(name, value);
        }
    }
    Ok(HttpRequest::get(url.as_str()).header("x-api-key", key).header("accept", "application/json"))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_array().and_then(|a| a.first()).and_then(Value::as_str).map(str::to_string))).map(|s| clean_text(&s, 200)).filter(|s| !s.is_empty())
}

/// Find the title's SubSource ID without ever substituting a different title.
async fn find_title(http: &dyn Transport, key: &str, query: &SubtitleQuery) -> Result<Option<(String, String, Option<i32>)>, ProviderProblem> {
    let pick = |body: &Value, exact: bool| -> Option<(String, String, Option<i32>)> {
        let wanted = normalise(&query.title);
        body.get("data").and_then(Value::as_array)?.iter().find_map(|entry| {
            let id = entry.get("movieId").and_then(|v| v.as_u64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string)))?;
            let title = text(entry, "title").unwrap_or_default();
            let alternate = text(entry, "alternateTitle").unwrap_or_default();
            let year = entry.get("releaseYear").and_then(Value::as_i64).map(|y| y as i32);
            let same_title = normalise(&title) == wanted || (!alternate.is_empty() && normalise(&alternate) == wanted);
            let same_year = query.year.is_none_or(|wanted_year| year == Some(wanted_year)) || query.is_episode();
            // An ID match is trusted as it is; a text match must be the same title and year.
            (!exact || (same_title && same_year)).then_some((id, title, year))
        })
    };
    if let Some(imdb) = &query.imdb_id {
        let mut params = vec![("searchType", "imdb".to_string()), ("imdb", imdb.clone())];
        if let Some(season) = query.season {
            params.push(("season", season.to_string()));
        }
        let response = http.send(request("movies/search", &params, key)?).await?;
        if let Some(problem) = refusal(&response) {
            return if problem == ProviderProblem::NotFound { Ok(None) } else { Err(problem) };
        }
        if let Some(found) = pick(&response.json()?, false) {
            return Ok(Some(found));
        }
    }
    if query.title.is_empty() {
        return Ok(None);
    }
    let mut params = vec![("searchType", "text".to_string()), ("q", query.title.to_lowercase())];
    if let Some(season) = query.season {
        params.push(("season", season.to_string()));
    }
    let response = http.send(request("movies/search", &params, key)?).await?;
    if let Some(problem) = refusal(&response) {
        return if problem == ProviderProblem::NotFound { Ok(None) } else { Err(problem) };
    }
    Ok(pick(&response.json()?, true))
}

impl SubtitleProvider for SubSource {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "subsource",
            name: "SubSource",
            description: "Community subtitles, listed per title and language. Needs your own API key.",
            site: "https://subsource.net",
            fields: vec![Field { name: "apiKey", label: "API key", secret: true, required: true, help: "Create one on the API tab of your SubSource account." }],
            movies: true,
            episodes: true,
        }
    }

    fn connect<'a>(&'a self, http: &'a dyn Transport, fields: &'a Credentials) -> BoxFuture<'a, Result<Credentials, ProviderProblem>> {
        Box::pin(async move {
            let key = fields.get("apiKey").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).ok_or(ProviderProblem::NotConnected)?;
            let response = http.send(request("movies/search", &[("searchType", "text".to_string()), ("q", "inception".to_string())], &key)?).await?;
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            response.json()?;
            Ok(Credentials::from([("apiKey".to_string(), key)]))
        })
    }

    fn search<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, query: &'a SubtitleQuery) -> BoxFuture<'a, Result<Vec<Candidate>, ProviderProblem>> {
        Box::pin(async move {
            let key = credentials.get("apiKey").filter(|k| !k.is_empty()).cloned().ok_or(ProviderProblem::NotConnected)?;
            let query = query.cleaned();
            let languages: Vec<(String, &'static str)> = query.languages.iter().filter_map(|tag| name_for(tag).map(|name| (tag.clone(), name))).collect();
            if !query.languages.is_empty() && languages.is_empty() {
                return Err(ProviderProblem::NotSupported);
            }
            let Some((movie_id, title, year)) = find_title(http, &key, &query).await? else { return Ok(Vec::new()) };
            let mut candidates = Vec::new();
            let wanted: Vec<&'static str> = if languages.is_empty() { vec!["english"] } else { languages.iter().map(|(_, n)| *n).collect() };
            for language in wanted {
                let mut params = vec![("movieId", movie_id.clone()), ("language", language.to_string()), ("limit", "100".to_string())];
                if let Some(season) = query.season {
                    params.push(("seasonNumber", season.to_string()));
                }
                if let Some(episode) = query.episode {
                    params.push(("episodeNumber", episode.to_string()));
                }
                let response = http.send(request("subtitles", &params, &key)?).await?;
                if let Some(problem) = refusal(&response) {
                    if problem == ProviderProblem::NotFound {
                        continue;
                    }
                    return Err(problem);
                }
                let body = response.json()?;
                if body.get("success").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                for item in body.get("data").and_then(Value::as_array).ok_or(ProviderProblem::BadResponse)? {
                    let Some(id) = item.get("subtitleId").and_then(|v| v.as_u64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string))) else { continue };
                    let release = text(item, "releaseInfo").unwrap_or_default();
                    let (season, episode) = season_episode_from_release(&release);
                    let commentary = item.get("commentary").and_then(Value::as_str).unwrap_or("").to_lowercase();
                    let flag = |key: &str| item.get(key).and_then(Value::as_bool).unwrap_or(false);
                    let not_hi = ["non hi", "nonhi", "non-hi", "non-sdh", "non sdh", "hi remove", "sdh remove"].iter().any(|m| commentary.contains(m));
                    let hi = flag("hearingImpaired") || (!not_hi && ["sdh", "closed caption", "_cc_"].iter().any(|m| commentary.contains(m)));
                    let uploader = item.get("contributors").and_then(Value::as_array).and_then(|list| {
                        let uploader_id = item.get("uploaderId")?;
                        list.iter().find(|c| c.get("id") == Some(uploader_id)).and_then(|c| text(c, "displayname"))
                    });
                    candidates.push(Candidate {
                        provider: "subsource".into(),
                        id,
                        language: item.get("language").and_then(Value::as_str).map(tag_for).unwrap_or_else(|| tag_for(language)),
                        release,
                        hearing_impaired: hi,
                        forced: flag("foreignParts") || commentary.contains("forced"),
                        ai_translated: false,
                        season,
                        episode,
                        pack: query.is_episode() && episode.is_none() && season.is_some(),
                        downloads: None,
                        uploader,
                        matched_title: Some(title.clone()),
                        matched_year: year,
                        locator: BTreeMap::new(),
                    });
                }
            }
            let mut candidates = keep_hearing_impaired(&query, keep_requested_episode(&query, candidates));
            rank(&query, &mut candidates);
            Ok(candidates)
        })
    }

    fn download<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded, ProviderProblem>> {
        Box::pin(async move {
            let key = credentials.get("apiKey").filter(|k| !k.is_empty()).cloned().ok_or(ProviderProblem::NotConnected)?;
            if candidate.id.is_empty() || !candidate.id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ProviderProblem::BadResponse);
            }
            let response = http.send(request(&format!("subtitles/{}/download", candidate.id), &[], &key)?.limit(MAX_DOWNLOAD_BYTES)).await?;
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            if !looks_like_subtitle_payload(&response.body) {
                return Err(ProviderProblem::BadResponse);
            }
            Ok(Downloaded { file_name: format!("subsource-{}.zip", candidate.id), bytes: response.body, remaining: None })
        })
    }
}
