//! Online subtitle services (T22).
//!
//! One adapter per service behind a common trait, all talking through `http::Transport` so the
//! same contract tests run against every adapter with scripted replies. Nothing here picks a
//! subtitle for the person: searches return *candidates* with their labels (language, release,
//! hearing-impaired, season pack, matched title and year), and a candidate is downloaded only when
//! the person chooses it. When a service finds nothing, the answer is "no results"; there is no
//! fallback to a differently named title.
pub mod addic7ed;
pub mod http;
pub mod opensubtitles;
pub mod secrets;
pub mod subdl;
pub mod subsource;

#[cfg(test)]
mod contract_tests;

pub use http::{BoxFuture, HttpRequest, HttpResponse, Method, ReqwestTransport, Transport};

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

pub const PROVIDER_IDS: [&str; 4] = ["opensubtitles", "subdl", "subsource", "addic7ed"];

/// Saved credentials for one service and profile, as plain named fields. They live in the OS
/// keyring only; see `secrets`.
pub type Credentials = BTreeMap<String, String>;

/// Why a request to a service did not give results. Messages are fixed text: a service's own
/// words, addresses and keys are never repeated back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderProblem {
    /// The service needs saved details (an API key or an account) and none are saved.
    NotConnected,
    /// The service refused the saved key or account.
    SignInRejected,
    RateLimited { retry_after_secs: Option<u64> },
    /// A daily allowance of downloads or requests is used up.
    QuotaExhausted { retry_after_secs: Option<u64> },
    /// The service now wants a paid plan for this.
    PaidPlanRequired,
    /// The service blocked the request (a bot check or a ban).
    Blocked,
    /// No connection, a timeout, or a server error.
    Unavailable,
    /// The reply was not what the service documents.
    BadResponse,
    /// The service does not cover this request (for example a movie on a TV-only service).
    NotSupported,
    /// The chosen subtitle is no longer offered by the service.
    NotFound,
}

impl std::fmt::Display for ProviderProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => f.write_str("Add your details for this service in Settings first."),
            Self::SignInRejected => f.write_str("The service rejected the saved key or account. Check it in Settings and connect again."),
            Self::RateLimited { retry_after_secs: Some(s) } => write!(f, "The service is limiting requests. Try again in about {s} seconds."),
            Self::RateLimited { retry_after_secs: None } => f.write_str("The service is limiting requests. Try again in a little while."),
            Self::QuotaExhausted { .. } => f.write_str("The daily allowance for this service is used up. It resets on its own; try again later."),
            Self::PaidPlanRequired => f.write_str("The service now requires a paid plan for this."),
            Self::Blocked => f.write_str("The service blocked the request (for example a bot check). Try again later or use another service."),
            Self::Unavailable => f.write_str("The service could not be reached."),
            Self::BadResponse => f.write_str("The service answered in a way this app does not understand."),
            Self::NotSupported => f.write_str("This service does not cover this kind of title."),
            Self::NotFound => f.write_str("That subtitle is no longer offered by the service. Search again."),
        }
    }
}
impl std::error::Error for ProviderProblem {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HearingImpaired {
    /// Show both kinds, with the label on each.
    #[default]
    Include,
    Exclude,
    Only,
}

/// What to look for. IDs are used when known; a title is only ever a search *text*, and the
/// candidates come back labelled with the title and year the service matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleQuery {
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    /// For an episode this is the *series* ID.
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    /// Language tags, best first: `en`, `pt-br`.
    pub languages: Vec<String>,
    pub hearing_impaired: HearingImpaired,
}

impl SubtitleQuery {
    pub fn is_episode(&self) -> bool {
        self.episode.is_some()
    }
    /// The query with its text and tags reduced to what services can safely be sent.
    pub fn cleaned(&self) -> Self {
        let mut q = self.clone();
        q.title = clean_text(&q.title, 200);
        q.languages = q.languages.iter().map(|l| l.trim().to_ascii_lowercase()).filter(|l| l.len() <= 12 && !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_')).collect();
        q.imdb_id = q.imdb_id.as_deref().and_then(imdb_digits).map(|d| format!("tt{d:0>7}"));
        q.tmdb_id = q.tmdb_id.filter(|t| !t.is_empty() && t.len() <= 12 && t.bytes().all(|b| b.is_ascii_digit()));
        q
    }
}

/// Digits of an IMDb ID given as `tt0133093` or `133093`.
pub fn imdb_digits(id: &str) -> Option<String> {
    let digits = id.trim().trim_start_matches("tt");
    (!digits.is_empty() && digits.len() <= 10 && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.to_string())
}

pub fn clean_text(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub provider: String,
    /// Opaque to everything except the adapter that made it.
    pub id: String,
    pub language: String,
    pub release: String,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub ai_translated: bool,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    /// A whole season (or several episodes) in one archive.
    pub pack: bool,
    pub downloads: Option<u64>,
    pub uploader: Option<String>,
    /// The title and year the service matched, so a wrong title is visible before downloading.
    pub matched_title: Option<String>,
    pub matched_year: Option<i32>,
    /// Adapter-private details needed to download (addresses, page links). Never sent to the UI.
    #[serde(skip)]
    pub locator: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Downloaded {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub remaining: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub required: bool,
    pub help: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub site: &'static str,
    /// What the person must give to connect; empty when the service works without an account.
    pub fields: Vec<Field>,
    pub movies: bool,
    pub episodes: bool,
}

pub trait SubtitleProvider: Send + Sync {
    fn info(&self) -> ProviderInfo;

    /// Check the details with the service and return what to keep. Nothing is kept when this
    /// fails, and a service that cannot be reached is not "connected".
    fn connect<'a>(&'a self, http: &'a dyn Transport, fields: &'a Credentials) -> BoxFuture<'a, Result<Credentials, ProviderProblem>>;

    /// `credentials` may be updated (a refreshed sign-in); the caller saves it when it changed.
    fn search<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, query: &'a SubtitleQuery) -> BoxFuture<'a, Result<Vec<Candidate>, ProviderProblem>>;

    fn download<'a>(&'a self, http: &'a dyn Transport, credentials: &'a mut Credentials, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded, ProviderProblem>>;

    /// A service with no required fields needs no saved details to search.
    fn needs_connection(&self) -> bool {
        self.info().fields.iter().any(|f| f.required)
    }
}

pub fn all() -> Vec<std::sync::Arc<dyn SubtitleProvider>> {
    vec![std::sync::Arc::new(opensubtitles::OpenSubtitles), std::sync::Arc::new(subdl::Subdl), std::sync::Arc::new(subsource::SubSource), std::sync::Arc::new(addic7ed::Addic7ed)]
}

pub fn by_id(id: &str) -> Option<std::sync::Arc<dyn SubtitleProvider>> {
    all().into_iter().find(|p| p.info().id == id)
}

/// Global choices about online subtitle services. Off until the person turns it on, because
/// searching sends the title being played (and its IDs) to the services they have connected.
/// Credentials are per profile and live in the keyring, not here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServiceSettings {
    pub enabled: bool,
    /// Per service switches; a service that is not listed is on.
    pub providers: BTreeMap<String, bool>,
    pub hearing_impaired: HearingImpaired,
    /// Languages to look for, best first. Empty means the player's subtitle language, then English.
    pub languages: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for ServiceSettings {
    fn default() -> Self {
        Self { enabled: false, providers: BTreeMap::new(), hearing_impaired: HearingImpaired::Include, languages: Vec::new(), extra: BTreeMap::new() }
    }
}

impl ServiceSettings {
    pub fn provider_enabled(&self, id: &str) -> bool {
        self.providers.get(id).copied().unwrap_or(true)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(unknown) = self.providers.keys().find(|id| !PROVIDER_IDS.contains(&id.as_str())) {
            anyhow::bail!("Unknown subtitle service '{}'", clean_text(unknown, 24));
        }
        if self.languages.len() > 8 {
            anyhow::bail!("Choose at most eight subtitle languages");
        }
        for tag in &self.languages {
            if tag.is_empty() || tag.len() > 12 || !tag.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-') {
                anyhow::bail!("'{}' is not a language tag such as en or pt-br", clean_text(tag, 16));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

fn season_episode_patterns() -> &'static (Regex, Regex, Regex) {
    static PATTERNS: OnceLock<(Regex, Regex, Regex)> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        (
            Regex::new(r"(?i)\bs(\d{1,2})[ ._-]?e(\d{1,3})(?:[ ._-]?e?(\d{1,3}))?").expect("valid"),
            Regex::new(r"(?i)\b(\d{1,2})x(\d{2,3})\b").expect("valid"),
            Regex::new(r"(?i)\b(?:s(\d{1,2})|season[ ._-]?(\d{1,2}))\b").expect("valid"),
        )
    })
}

/// Season and episode named in a release label: `S02E05`, `2x05`, or a bare season (`S02`,
/// `Season 2`), which is a pack. Returns `(season, episode)`.
pub fn season_episode_from_release(release: &str) -> (Option<i32>, Option<i32>) {
    let (sxe, nxn, season_only) = season_episode_patterns();
    if let Some(c) = sxe.captures(release) {
        return (c[1].parse().ok(), c[2].parse().ok());
    }
    if let Some(c) = nxn.captures(release) {
        return (c[1].parse().ok(), c[2].parse().ok());
    }
    if let Some(c) = season_only.captures(release) {
        return (c.get(1).or_else(|| c.get(2)).and_then(|m| m.as_str().parse().ok()), None);
    }
    (None, None)
}

/// Drop candidates that are for a different episode, keeping season packs (flagged) for the
/// requested season. Movies are not filtered.
pub fn keep_requested_episode(query: &SubtitleQuery, candidates: Vec<Candidate>) -> Vec<Candidate> {
    let Some(episode) = query.episode else { return candidates };
    candidates
        .into_iter()
        .filter(|c| {
            let season_ok = match (query.season, c.season) {
                (Some(wanted), Some(found)) => wanted == found,
                _ => true,
            };
            let episode_ok = match c.episode {
                Some(found) => found == episode,
                // No episode number: only a pack of the right season is acceptable.
                None => c.pack && c.season.is_none_or(|found| query.season.is_none_or(|wanted| wanted == found)),
            };
            season_ok && episode_ok
        })
        .collect()
}

/// Apply the hearing-impaired preference.
pub fn keep_hearing_impaired(query: &SubtitleQuery, candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates
        .into_iter()
        .filter(|c| match query.hearing_impaired {
            HearingImpaired::Include => true,
            HearingImpaired::Exclude => !c.hearing_impaired,
            HearingImpaired::Only => c.hearing_impaired,
        })
        .collect()
}

/// Order candidates: the person's language order, then exact episodes before packs, then the
/// preference for hearing-impaired labels, then popularity, then a stable tiebreak.
pub fn rank(query: &SubtitleQuery, candidates: &mut [Candidate]) {
    let language_rank = |c: &Candidate| query.languages.iter().position(|l| l.split('-').next() == c.language.split('-').next()).unwrap_or(usize::MAX);
    candidates.sort_by(|a, b| {
        (language_rank(a), a.pack, a.hearing_impaired != (query.hearing_impaired == HearingImpaired::Only), std::cmp::Reverse(a.downloads.unwrap_or(0)), &a.provider, &a.id)
            .cmp(&(language_rank(b), b.pack, b.hearing_impaired != (query.hearing_impaired == HearingImpaired::Only), std::cmp::Reverse(b.downloads.unwrap_or(0)), &b.provider, &b.id))
    });
}

/// What a download must look like: a ZIP archive or a plain text subtitle. Anything else (most
/// often an HTML page returned with a success status) is refused.
pub fn looks_like_subtitle_payload(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return true;
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<!doctype") || lower.starts_with("<html") || lower.starts_with('{') {
        return false;
    }
    trimmed.starts_with("WEBVTT") || trimmed.starts_with("[Script Info]") || trimmed.contains("-->") || trimmed.starts_with("<sami") || lower.starts_with("<tt") || lower.starts_with("<?xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_and_episode_are_read_from_release_labels() {
        assert_eq!(season_episode_from_release("Show.Name.S02E05.720p.WEB"), (Some(2), Some(5)));
        assert_eq!(season_episode_from_release("Show 2x05 HDTV"), (Some(2), Some(5)));
        assert_eq!(season_episode_from_release("Show.S03.COMPLETE.720p"), (Some(3), None));
        assert_eq!(season_episode_from_release("Show Season 4 Pack"), (Some(4), None));
        assert_eq!(season_episode_from_release("Film.2019.1080p.BluRay"), (None, None));
        assert_eq!(season_episode_from_release("x264.1080p"), (None, None), "resolutions and codecs are not episodes");
    }

    fn candidate(season: Option<i32>, episode: Option<i32>, pack: bool) -> Candidate {
        Candidate { provider: "p".into(), id: format!("{season:?}{episode:?}{pack}"), language: "en".into(), season, episode, pack, ..Default::default() }
    }

    #[test]
    fn only_the_requested_episode_or_a_matching_season_pack_is_kept() {
        let query = SubtitleQuery { season: Some(2), episode: Some(5), ..Default::default() };
        let kept = keep_requested_episode(&query, vec![candidate(Some(2), Some(5), false), candidate(Some(2), Some(6), false), candidate(Some(1), Some(5), false), candidate(Some(2), None, true), candidate(Some(3), None, true), candidate(None, None, false), candidate(None, Some(5), false)]);
        let ids: Vec<_> = kept.iter().map(|c| (c.season, c.episode, c.pack)).collect();
        assert_eq!(ids, [(Some(2), Some(5), false), (Some(2), None, true), (None, Some(5), false)]);
        let movie = SubtitleQuery::default();
        assert_eq!(keep_requested_episode(&movie, vec![candidate(None, None, false)]).len(), 1, "movies are never filtered by episode");
    }

    #[test]
    fn hearing_impaired_preference_filters_and_ranking_follows_language_order() {
        let mut a = candidate(None, None, false);
        a.hearing_impaired = true;
        a.language = "fr".into();
        let mut b = candidate(None, None, false);
        b.language = "en".into();
        b.id = "b".into();
        b.downloads = Some(5);
        let mut c = b.clone();
        c.id = "c".into();
        c.downloads = Some(50);
        let mut query = SubtitleQuery { languages: vec!["en".into(), "fr".into()], ..Default::default() };
        let mut all = vec![a.clone(), b.clone(), c.clone()];
        rank(&query, &mut all);
        assert_eq!(all.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["c", "b", &a.id], "language order first, then popularity");
        query.hearing_impaired = HearingImpaired::Exclude;
        assert_eq!(keep_hearing_impaired(&query, vec![a.clone(), b.clone()]).len(), 1);
        query.hearing_impaired = HearingImpaired::Only;
        assert_eq!(keep_hearing_impaired(&query, vec![a, b]).len(), 1);
    }

    #[test]
    fn queries_are_cleaned_before_they_reach_a_service() {
        let q = SubtitleQuery { title: "  The\u{0}  Matrix\n".into(), imdb_id: Some("tt0133093".into()), tmdb_id: Some("603; drop".into()), languages: vec!["EN".into(), "pt-BR".into(), "e n".into(), "".into()], ..Default::default() }.cleaned();
        assert_eq!(q.title, "The Matrix");
        assert_eq!(q.imdb_id.as_deref(), Some("tt0133093"));
        assert_eq!(q.tmdb_id, None);
        assert_eq!(q.languages, ["en", "pt-br"]);
        assert_eq!(imdb_digits("133093").as_deref(), Some("133093"));
        assert_eq!(imdb_digits("ttabc"), None);
    }

    #[test]
    fn payloads_must_be_archives_or_subtitle_text() {
        assert!(looks_like_subtitle_payload(b"PK\x03\x04rest"));
        assert!(looks_like_subtitle_payload(b"1\n00:00:01,000 --> 00:00:02,000\nHi\n"));
        assert!(looks_like_subtitle_payload(b"\xEF\xBB\xBFWEBVTT\n\n"));
        assert!(looks_like_subtitle_payload(b"[Script Info]\nTitle: x"));
        for bad in [&b""[..], b"<!DOCTYPE html><html>Daily limit reached</html>", b"{\"error\":\"nope\"}", b"just some words"] {
            assert!(!looks_like_subtitle_payload(bad), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn problems_have_fixed_messages_without_service_text() {
        for problem in [ProviderProblem::NotConnected, ProviderProblem::SignInRejected, ProviderProblem::RateLimited { retry_after_secs: Some(9) }, ProviderProblem::QuotaExhausted { retry_after_secs: None }, ProviderProblem::PaidPlanRequired, ProviderProblem::Blocked, ProviderProblem::Unavailable, ProviderProblem::BadResponse, ProviderProblem::NotSupported, ProviderProblem::NotFound] {
            let message = problem.to_string();
            assert!(!message.contains("://") && message.ends_with('.'), "{message}");
        }
        assert!(ProviderProblem::RateLimited { retry_after_secs: Some(9) }.to_string().contains("9 seconds"));
    }
}
