//! Addic7ed (web pages; there is no public API).
//!
//! TV episodes only, and no account: the pages are read the way a browser reads them. That makes
//! this adapter the most fragile one: the site can change its layout or put up a bot check at any
//! time, and then the honest answer is "blocked" or "no results", never a guess. The show is
//! found by its exact name (the year is optional); a differently named show is never used.
use super::http::{allowed_url, HttpRequest, MAX_DOWNLOAD_BYTES};
use super::*;
use std::sync::OnceLock;

pub struct Addic7ed;

const SERVER: &str = "https://www.addic7ed.com";

fn patterns() -> &'static (Regex, Regex, Regex, Regex, Regex) {
    static PATTERNS: OnceLock<(Regex, Regex, Regex, Regex, Regex)> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        (
            Regex::new(r#"(?is)<a[^>]+href="/show/(\d+)"[^>]*>(.*?)</a>"#).expect("valid"),
            Regex::new(r#"(?is)<tr[^>]*class="[^"]*\bepeven\b[^"]*"[^>]*>(.*?)</tr>"#).expect("valid"),
            Regex::new(r"(?is)<td[^>]*>(.*?)</td>").expect("valid"),
            Regex::new(r#"(?is)<a[^>]+href="([^"]+)""#).expect("valid"),
            Regex::new(r"(?s)<[^>]*>").expect("valid"),
        )
    })
}

fn text_of(html: &str) -> String {
    let stripped = patterns().4.replace_all(html, " ");
    let decoded = stripped.replace("&nbsp;", " ").replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">");
    clean_text(&decoded, 200)
}

fn normalise(title: &str) -> String {
    title.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A bot check or a ban page rather than the site's own content.
fn blocked(response: &HttpResponse) -> bool {
    let body = String::from_utf8_lossy(&response.body[..response.body.len().min(8192)]).to_lowercase();
    matches!(response.status, 403 | 503) || body.contains("cf-chl") || body.contains("just a moment") || body.contains("attention required") || body.contains("captcha")
}

fn refusal(response: &HttpResponse) -> Option<ProviderProblem> {
    if blocked(response) {
        return Some(ProviderProblem::Blocked);
    }
    match response.status {
        200..=299 => None,
        304 | 429 => Some(ProviderProblem::RateLimited { retry_after_secs: response.retry_after() }),
        404 => Some(ProviderProblem::NotFound),
        500..=599 => Some(ProviderProblem::Unavailable),
        _ => Some(ProviderProblem::BadResponse),
    }
}

fn get(path: &str, referer: &str) -> HttpRequest {
    HttpRequest::get(format!("{SERVER}/{}", path.trim_start_matches('/'))).header("referer", referer).header("accept", "text/html,*/*")
}

fn show_for(html: &str, query: &SubtitleQuery) -> Option<(String, String)> {
    let wanted = normalise(&query.title);
    let mut exact: Vec<(String, String)> = patterns()
        .0
        .captures_iter(html)
        .filter_map(|c| {
            let name = text_of(&c[2]);
            // "Show (2019)" matches a query for "Show" in 2019, and a bare "Show" query too.
            let without_year = name.rsplit_once(" (").filter(|(_, tail)| tail.trim_end_matches(')').chars().all(|ch| ch.is_ascii_digit())).map_or(name.clone(), |(head, _)| head.to_string());
            (normalise(&name) == wanted || normalise(&without_year) == wanted).then(|| (c[1].to_string(), name))
        })
        .collect();
    if let Some(year) = query.year {
        if let Some(found) = exact.iter().find(|(_, name)| name.contains(&format!("({year})"))) {
            return Some(found.clone());
        }
    }
    exact.sort_by_key(|(_, name)| name.contains('(')); // the plain name first
    exact.into_iter().next()
}

impl SubtitleProvider for Addic7ed {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "addic7ed",
            name: "Addic7ed",
            description: "TV episode subtitles read from the website; no account needed. Daily downloads are limited by the site and it may block automated requests.",
            site: "https://www.addic7ed.com",
            fields: Vec::new(),
            movies: false,
            episodes: true,
        }
    }

    fn connect<'a>(&'a self, http: &'a dyn Transport, _fields: &'a Credentials) -> BoxFuture<'a, Result<Credentials, ProviderProblem>> {
        Box::pin(async move {
            // Nothing to save; reaching the site is the only thing to verify.
            let response = http.send(get("shows.php", SERVER)).await?;
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            Ok(Credentials::new())
        })
    }

    fn search<'a>(&'a self, http: &'a dyn Transport, _credentials: &'a mut Credentials, query: &'a SubtitleQuery) -> BoxFuture<'a, Result<Vec<Candidate>, ProviderProblem>> {
        Box::pin(async move {
            let query = query.cleaned();
            let (Some(season), true) = (query.season, query.is_episode()) else {
                return if query.is_episode() { Ok(Vec::new()) } else { Err(ProviderProblem::NotSupported) };
            };
            if query.title.is_empty() {
                return Ok(Vec::new());
            }
            let shows = http.send(get("shows.php", SERVER)).await?;
            if let Some(problem) = refusal(&shows) {
                return Err(problem);
            }
            let Some((show_id, show_name)) = show_for(&String::from_utf8_lossy(&shows.body), &query) else { return Ok(Vec::new()) };
            let list = http
                .send(get(&format!("ajax_loadShow.php?show={show_id}&season={season}&langs=%7C"), &format!("{SERVER}/show/{show_id}")).header("x-requested-with", "XMLHttpRequest"))
                .await?;
            if let Some(problem) = refusal(&list) {
                return Err(problem);
            }
            let html = String::from_utf8_lossy(&list.body).into_owned();
            if html.to_lowercase().contains("relax, slow down") {
                return Err(ProviderProblem::RateLimited { retry_after_secs: None });
            }
            let mut candidates = Vec::new();
            for row in patterns().1.captures_iter(&html) {
                let cells: Vec<String> = patterns().2.captures_iter(&row[1]).map(|c| c[1].to_string()).collect();
                if cells.len() < 10 {
                    continue;
                }
                let status = text_of(&cells[5]);
                // A subtitle still being worked on shows a percentage; only finished ones are offered.
                if status.contains('%') {
                    continue;
                }
                let (Some(row_season), Some(row_episode)) = (text_of(&cells[0]).parse::<i32>().ok(), text_of(&cells[1]).parse::<i32>().ok()) else { continue };
                let Some(download) = patterns().3.captures(&cells[9]).map(|c| c[1].to_string()).filter(|href| href.starts_with('/') && !href.starts_with("//") && !href.contains("..")) else { continue };
                let page = patterns().3.captures(&cells[2]).map(|c| c[1].trim_start_matches('/').to_string()).unwrap_or_default();
                let language_name = text_of(&cells[3]);
                let language = crate::hls::language_code(&language_name);
                if !query.languages.is_empty() && !query.languages.iter().any(|l| l.split('-').next() == language.split('-').next()) {
                    continue;
                }
                candidates.push(Candidate {
                    provider: "addic7ed".into(),
                    id: download,
                    language,
                    release: text_of(&cells[4]),
                    hearing_impaired: !text_of(&cells[6]).is_empty(),
                    forced: false,
                    ai_translated: false,
                    season: Some(row_season),
                    episode: Some(row_episode),
                    pack: false,
                    downloads: None,
                    uploader: None,
                    matched_title: Some(show_name.clone()),
                    matched_year: None,
                    locator: BTreeMap::from([("page".to_string(), page)]),
                });
            }
            let mut candidates = keep_hearing_impaired(&query, keep_requested_episode(&query, candidates));
            rank(&query, &mut candidates);
            Ok(candidates)
        })
    }

    fn download<'a>(&'a self, http: &'a dyn Transport, _credentials: &'a mut Credentials, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded, ProviderProblem>> {
        Box::pin(async move {
            if !candidate.id.starts_with('/') || candidate.id.starts_with("//") || candidate.id.contains("..") {
                return Err(ProviderProblem::BadResponse);
            }
            let page = candidate.locator.get("page").map(|p| format!("{SERVER}/{}", p.trim_start_matches('/'))).unwrap_or_else(|| SERVER.to_string());
            let url = allowed_url(&format!("{SERVER}{}", candidate.id)).ok_or(ProviderProblem::BadResponse)?;
            let response = http.send(HttpRequest::get(url.as_str()).header("referer", page).limit(MAX_DOWNLOAD_BYTES)).await?;
            // The site answers a spent daily allowance with an ordinary page instead of a file.
            if response.status == 200 && response.is_html() {
                return Err(if blocked(&response) { ProviderProblem::Blocked } else { ProviderProblem::QuotaExhausted { retry_after_secs: None } });
            }
            if let Some(problem) = refusal(&response) {
                return Err(problem);
            }
            if !looks_like_subtitle_payload(&response.body) {
                return Err(ProviderProblem::BadResponse);
            }
            Ok(Downloaded { file_name: "addic7ed.srt".into(), bytes: response.body, remaining: None })
        })
    }
}
