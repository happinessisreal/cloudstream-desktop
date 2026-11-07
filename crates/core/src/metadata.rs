//! Metadata matching and enrichment (T24).
//!
//! Principles, each enforced and tested here:
//!
//! * **Identity is by ID, not by name.** A title is tied to a metadata service only by an ID a
//!   provider supplied (`syncData`) or one the user chose. Search results are *suggestions*; no
//!   code path links a title because its display name looks similar.
//! * **Provenance is kept.** Every link records where it came from (provider, user choice) and
//!   when. Every piece of merged data records which service it came from.
//! * **Enrichment never replaces provider data.** The provider's episode payloads, URLs and
//!   identity are untouched; metadata only fills optional display fields, side by side.
//! * **No network without opt-in, and only to known hosts over HTTPS.**
//! * **Never synthesise a source address from a metadata ID.** Finding a source goes through a
//!   provider's own search or its own `getLoadUrl`, and the user picks the result.
use crate::models::*;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

pub const CACHE_TTL_MS: u64 = 7 * 24 * 3_600_000;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const ALLOWED_HOSTS: [&str; 3] = ["graphql.anilist.co", "kitsu.io", "api.jikan.moe"];

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/// Metadata is opt-in: it sends titles and IDs to third-party services.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub anilist: bool,
    #[serde(default = "yes")]
    pub kitsu: bool,
    /// Filler flags (from Jikan, via a MyAnimeList id).
    #[serde(default = "yes")]
    pub filler: bool,
    #[serde(default = "yes")]
    pub trailer: bool,
    #[serde(default = "yes")]
    pub cast: bool,
    #[serde(default = "yes")]
    pub recommendations: bool,
    /// Prefer the metadata poster over the provider's. Off by default: the provider's own art wins.
    #[serde(default)]
    pub prefer_poster: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn yes() -> bool {
    true
}

impl Default for MetadataSettings {
    fn default() -> Self {
        Self { enabled: false, anilist: true, kitsu: true, filler: true, trailer: true, cast: true, recommendations: true, prefer_poster: false, extra: BTreeMap::new() }
    }
}

// ---------------------------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------------------------

/// Services this app can look up. IDs are strings so no service's numbering is assumed.
pub const SERVICES: [&str; 6] = ["anilist", "mal", "kitsu", "imdb", "tmdb", "tvdb"];

fn valid_id(service: &str, id: &str) -> bool {
    match service {
        "imdb" => id.len() >= 3 && id.len() <= 12 && id.starts_with("tt") && id[2..].bytes().all(|b| b.is_ascii_digit()),
        "anilist" | "mal" | "kitsu" | "tmdb" | "tvdb" => !id.is_empty() && id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit()) && id != "0",
        _ => false,
    }
}

/// External IDs a provider supplied in `syncData`. Android stores AniList/MAL/Kitsu as plain
/// numbers under their own keys and packs IMDb/TMDb/TVDB into one JSON object under `simkl`.
/// Anything that is not a valid ID for its service is dropped, never repaired.
pub fn external_ids(sync_data: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for (key, value) in sync_data {
        let key = key.trim().to_lowercase();
        let value = value.trim();
        if key == "simkl" {
            if let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(value) {
                for (service, id) in map {
                    let service = service.to_lowercase();
                    if SERVICES.contains(&service.as_str()) && valid_id(&service, id.trim()) {
                        ids.entry(service).or_insert_with(|| id.trim().to_string());
                    }
                }
            }
        } else if SERVICES.contains(&key.as_str()) && valid_id(&key, value) {
            ids.insert(key, value.to_string());
        }
    }
    ids
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchSource {
    /// The extension supplied the ID itself.
    Provider,
    /// The person picked it from search results or typed it in.
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataLink {
    pub media_id: String,
    pub service: String,
    pub external_id: String,
    pub source: MatchSource,
    pub linked_at_ms: u64,
    /// The service's title and year when linked, so the link can be reviewed later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
}

/// The links that apply to a title. A user's choice overrides what a provider supplied for the
/// same service, and an explicit "unlink" (stored as a blocked ID) survives later re-supplying.
pub fn effective_links(media_id: &str, supplied: &BTreeMap<String, String>, stored: &[MetadataLink], blocked: &[(String, String)], now_ms: u64) -> Vec<MetadataLink> {
    let mut links: BTreeMap<String, MetadataLink> = BTreeMap::new();
    for (service, id) in supplied {
        if blocked.iter().any(|(s, i)| s == service && i == id) {
            continue;
        }
        links.insert(service.clone(), MetadataLink { media_id: media_id.into(), service: service.clone(), external_id: id.clone(), source: MatchSource::Provider, linked_at_ms: now_ms, label: None, year: None });
    }
    for link in stored.iter().filter(|l| l.media_id == media_id && SERVICES.contains(&l.service.as_str())) {
        links.insert(link.service.clone(), link.clone());
    }
    links.into_values().collect()
}

// ---------------------------------------------------------------------------------------------
// Candidates (search results are suggestions, never links)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub service: String,
    pub external_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    /// Closeness of the title to the one searched for, 0 to 100. Only used to order suggestions.
    #[serde(default)]
    pub similarity: u32,
    /// Plain-language cautions, such as a different year (often a remake).
    #[serde(default)]
    pub warnings: Vec<String>,
}

fn normalise(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn title_similarity(a: &str, b: &str) -> u32 {
    let (a, b) = (normalise(a), normalise(b));
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    if a == b {
        return 100;
    }
    let words_a: std::collections::BTreeSet<&str> = a.split(' ').collect();
    let words_b: std::collections::BTreeSet<&str> = b.split(' ').collect();
    let shared = words_a.intersection(&words_b).count() as f64;
    let total = words_a.union(&words_b).count() as f64;
    ((shared / total) * 90.0) as u32
}

/// Order suggestions and attach cautions. Never selects one: even a perfect title match is only a
/// suggestion the person has to confirm.
pub fn rank_candidates(wanted_title: &str, wanted_year: Option<i32>, mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    for candidate in &mut candidates {
        candidate.similarity = title_similarity(wanted_title, &candidate.title);
        if let (Some(want), Some(have)) = (wanted_year, candidate.year) {
            if want != have {
                candidate.warnings.push(format!("Year differs ({have} here, {want} in your source). It may be a remake or a different season."));
                candidate.similarity = candidate.similarity.saturating_sub(10);
            }
        }
        if candidate.similarity < 40 {
            candidate.warnings.push("The title is quite different.".into());
        }
    }
    candidates.sort_by(|a, b| b.similarity.cmp(&a.similarity).then_with(|| a.title.cmp(&b.title)).then_with(|| a.external_id.cmp(&b.external_id)));
    candidates
}

// ---------------------------------------------------------------------------------------------
// Enrichment
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CastMember {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub title: String,
    pub service: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Enrichment {
    /// Where this data came from, shown to the person next to it.
    pub service: String,
    pub external_id: String,
    pub locale: String,
    pub fetched_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    /// 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailer_url: Option<String>,
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub recommendations: Vec<Recommendation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_airing_episode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_airing_at_ms: Option<u64>,
    /// Episode numbers flagged as filler (absolute numbering, as the service counts them).
    #[serde(default)]
    pub filler_episodes: Vec<i32>,
}

pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#039;", "'").replace("&nbsp;", " ")
        .split_whitespace().collect::<Vec<_>>().join(" ")
}

fn https_url(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty()).then(|| value.to_string())
}

fn s(value: &Value, path: &str) -> Option<String> {
    value.pointer(path).and_then(Value::as_str).map(str::to_string).filter(|t| !t.is_empty())
}

fn first_title(value: &Value, base: &str) -> Option<String> {
    ["english", "romaji", "native"].iter().find_map(|k| s(value, &format!("{base}/{k}")))
}

// ---- AniList (GraphQL, public queries) ----

pub const ANILIST_URL: &str = "https://graphql.anilist.co";

pub fn anilist_search_request(title: &str) -> Value {
    json!({
        "query": "query($q:String){Page(perPage:8){media(search:$q,type:ANIME,sort:SEARCH_MATCH){id idMal title{romaji english native} startDate{year} format coverImage{large}}}}",
        "variables": { "q": title }
    })
}

pub fn anilist_details_request(id: &str) -> Value {
    json!({
        "query": "query($id:Int){Media(id:$id,type:ANIME){id idMal title{romaji english} description(asHtml:false) genres averageScore coverImage{extraLarge large} bannerImage trailer{id site} nextAiringEpisode{episode airingAt} characters(perPage:12,sort:[ROLE,RELEVANCE]){edges{role node{name{full} image{large}}}} recommendations(perPage:8,sort:RATING_DESC){nodes{mediaRecommendation{id title{romaji english} coverImage{large}}}}}}",
        "variables": { "id": id.parse::<i64>().unwrap_or(0) }
    })
}

pub fn parse_anilist_search(response: &Value) -> Vec<Candidate> {
    response.pointer("/data/Page/media").and_then(Value::as_array).into_iter().flatten().filter_map(|media| {
        let id = media.get("id")?.as_i64()?.to_string();
        Some(Candidate {
            service: "anilist".into(),
            external_id: id,
            title: first_title(media, "/title")?,
            year: media.pointer("/startDate/year").and_then(Value::as_i64).map(|y| y as i32),
            format: s(media, "/format"),
            poster_url: https_url(s(media, "/coverImage/large").as_deref()),
            similarity: 0,
            warnings: Vec::new(),
        })
    }).collect()
}

pub fn parse_anilist_details(response: &Value, locale: &str, now_ms: u64) -> Result<Enrichment> {
    let media = response.pointer("/data/Media").filter(|m| m.is_object()).context("AniList returned no title for that id")?;
    let id = media.get("id").and_then(Value::as_i64).context("AniList returned a title without an id")?.to_string();
    let trailer = match (s(media, "/trailer/site").as_deref(), s(media, "/trailer/id")) {
        (Some("youtube"), Some(video)) if video.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') => Some(format!("https://www.youtube.com/watch?v={video}")),
        _ => None,
    };
    let cast = media.pointer("/characters/edges").and_then(Value::as_array).into_iter().flatten().filter_map(|edge| {
        Some(CastMember { name: s(edge, "/node/name/full")?, role: s(edge, "/role").map(|r| r.to_lowercase()), image_url: https_url(s(edge, "/node/image/large").as_deref()) })
    }).collect();
    let recommendations = media.pointer("/recommendations/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|node| {
        let rec = node.get("mediaRecommendation")?;
        Some(Recommendation { title: first_title(rec, "/title")?, service: "anilist".into(), external_id: rec.get("id")?.as_i64()?.to_string(), poster_url: https_url(s(rec, "/coverImage/large").as_deref()) })
    }).collect();
    Ok(Enrichment {
        service: "anilist".into(),
        external_id: id,
        locale: locale.into(),
        fetched_at_ms: now_ms,
        poster_url: https_url(s(media, "/coverImage/extraLarge").or_else(|| s(media, "/coverImage/large")).as_deref()),
        banner_url: https_url(s(media, "/bannerImage").as_deref()),
        synopsis: s(media, "/description").map(|d| strip_markup(&d)).filter(|d| !d.is_empty()),
        genres: media.get("genres").and_then(Value::as_array).into_iter().flatten().filter_map(|g| g.as_str().map(str::to_string)).collect(),
        score: media.get("averageScore").and_then(Value::as_f64),
        trailer_url: trailer,
        cast,
        recommendations,
        next_airing_episode: media.pointer("/nextAiringEpisode/episode").and_then(Value::as_i64).map(|e| e as i32),
        next_airing_at_ms: media.pointer("/nextAiringEpisode/airingAt").and_then(Value::as_u64).map(|secs| secs * 1000),
        filler_episodes: Vec::new(),
    })
}

// ---- Kitsu (JSON:API, public) ----

pub fn kitsu_search_url(title: &str) -> String {
    format!("https://kitsu.io/api/edge/anime?filter%5Btext%5D={}&page%5Blimit%5D=8", percent_encode(title))
}

pub fn kitsu_details_url(id: &str) -> String {
    format!("https://kitsu.io/api/edge/anime/{}", percent_encode(id))
}

fn percent_encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn kitsu_title(attributes: &Value) -> Option<String> {
    ["/titles/en", "/canonicalTitle", "/titles/en_jp", "/titles/ja_jp"].iter().find_map(|p| s(attributes, p))
}

pub fn parse_kitsu_search(response: &Value) -> Vec<Candidate> {
    response.get("data").and_then(Value::as_array).into_iter().flatten().filter_map(|item| {
        let attributes = item.get("attributes")?;
        Some(Candidate {
            service: "kitsu".into(),
            external_id: item.get("id")?.as_str()?.to_string(),
            title: kitsu_title(attributes)?,
            year: s(attributes, "/startDate").and_then(|d| d.get(..4).and_then(|y| y.parse().ok())),
            format: s(attributes, "/subtype"),
            poster_url: https_url(s(attributes, "/posterImage/small").or_else(|| s(attributes, "/posterImage/original")).as_deref()),
            similarity: 0,
            warnings: Vec::new(),
        })
    }).collect()
}

pub fn parse_kitsu_details(response: &Value, locale: &str, now_ms: u64) -> Result<Enrichment> {
    let item = response.get("data").filter(|d| d.is_object()).context("Kitsu returned no title for that id")?;
    let attributes = item.get("attributes").context("Kitsu returned a title without details")?;
    let video = s(attributes, "/youtubeVideoId").filter(|v| v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    Ok(Enrichment {
        service: "kitsu".into(),
        external_id: item.get("id").and_then(Value::as_str).context("Kitsu returned a title without an id")?.to_string(),
        locale: locale.into(),
        fetched_at_ms: now_ms,
        poster_url: https_url(s(attributes, "/posterImage/large").or_else(|| s(attributes, "/posterImage/original")).as_deref()),
        banner_url: https_url(s(attributes, "/coverImage/original").as_deref()),
        synopsis: s(attributes, "/synopsis").map(|d| strip_markup(&d)).filter(|d| !d.is_empty()),
        genres: Vec::new(),
        // Kitsu rates 0 to 100 as a string.
        score: s(attributes, "/averageRating").and_then(|r| r.parse::<f64>().ok()),
        trailer_url: video.map(|v| format!("https://www.youtube.com/watch?v={v}")),
        ..Default::default()
    })
}

// ---- Jikan (filler flags by MyAnimeList id) ----

pub fn jikan_episodes_url(mal_id: &str, page: u32) -> String {
    format!("https://api.jikan.moe/v4/anime/{}/episodes?page={page}", percent_encode(mal_id))
}

/// `(filler episode numbers, has next page)`.
pub fn parse_jikan_episodes(response: &Value) -> (Vec<i32>, bool) {
    let fillers = response.get("data").and_then(Value::as_array).into_iter().flatten().filter(|e| e.get("filler").and_then(Value::as_bool) == Some(true)).filter_map(|e| e.get("mal_id").and_then(Value::as_i64).map(|n| n as i32)).collect();
    (fillers, response.pointer("/pagination/has_next_page").and_then(Value::as_bool).unwrap_or(false))
}

// ---------------------------------------------------------------------------------------------
// Transport and client
// ---------------------------------------------------------------------------------------------

pub enum Request {
    Get(String),
    Post(String, Value),
}

/// How the client talks to the network. The app uses `Reqwest`; tests use recorded responses.
pub trait Transport: Send + Sync {
    fn send(&self, request: Request) -> BoxFuture<Value>;
}

fn check_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).context("Invalid metadata address")?;
    if parsed.scheme() != "https" || !parsed.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h)) {
        bail!("Metadata requests are only made to known services over HTTPS");
    }
    Ok(())
}

pub struct Reqwest {
    client: reqwest::Client,
}

impl Reqwest {
    pub fn new() -> Result<Self> {
        Ok(Self { client: reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).redirect(reqwest::redirect::Policy::none()).user_agent("CloudstreamDesktop").build()? })
    }
}

impl Transport for Reqwest {
    fn send(&self, request: Request) -> BoxFuture<Value> {
        let client = self.client.clone();
        Box::pin(async move {
            let builder = match &request {
                Request::Get(url) => {
                    check_url(url)?;
                    client.get(url)
                }
                Request::Post(url, body) => {
                    check_url(url)?;
                    client.post(url).json(body)
                }
            };
            let mut response = builder.send().await.context("Could not reach the metadata service")?;
            if !response.status().is_success() {
                bail!("The metadata service answered with an error ({})", response.status().as_u16());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    bail!("The metadata service sent more data than expected");
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&body).context("The metadata service sent something unreadable")
        })
    }
}

pub struct Client {
    transport: Arc<dyn Transport>,
    settings: MetadataSettings,
}

impl Client {
    pub fn new(transport: Arc<dyn Transport>, settings: MetadataSettings) -> Self {
        Self { transport, settings }
    }

    pub fn enabled_services(&self) -> Vec<&'static str> {
        let mut services = Vec::new();
        if self.settings.anilist {
            services.push("anilist");
        }
        if self.settings.kitsu {
            services.push("kitsu");
        }
        services
    }

    /// Search enabled services. One service failing does not hide the others' results.
    pub async fn search(&self, title: &str, year: Option<i32>) -> Result<(Vec<Candidate>, Vec<String>)> {
        if !self.settings.enabled {
            bail!("Metadata lookups are turned off. Turn them on in Settings to search");
        }
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            bail!("Enter a title between 1 and 200 characters");
        }
        let mut all = Vec::new();
        let mut problems = Vec::new();
        if self.settings.anilist {
            match self.transport.send(Request::Post(ANILIST_URL.into(), anilist_search_request(title))).await {
                Ok(value) => all.extend(parse_anilist_search(&value)),
                Err(error) => problems.push(format!("AniList: {error:#}")),
            }
        }
        if self.settings.kitsu {
            match self.transport.send(Request::Get(kitsu_search_url(title))).await {
                Ok(value) => all.extend(parse_kitsu_search(&value)),
                Err(error) => problems.push(format!("Kitsu: {error:#}")),
            }
        }
        if all.is_empty() && !problems.is_empty() {
            bail!("No metadata service answered. {}", problems.join(" "));
        }
        Ok((rank_candidates(title, year, all), problems))
    }

    /// Fetch details for one linked ID. Filler flags come from Jikan when a MyAnimeList id is
    /// known; their failure never fails the rest.
    pub async fn details(&self, service: &str, id: &str, mal_id: Option<&str>, locale: &str, now_ms: u64) -> Result<Enrichment> {
        if !self.settings.enabled {
            bail!("Metadata lookups are turned off");
        }
        if !valid_id(service, id) {
            bail!("That is not a valid {service} id");
        }
        let mut enrichment = match service {
            "anilist" if self.settings.anilist => parse_anilist_details(&self.transport.send(Request::Post(ANILIST_URL.into(), anilist_details_request(id))).await?, locale, now_ms)?,
            "kitsu" if self.settings.kitsu => parse_kitsu_details(&self.transport.send(Request::Get(kitsu_details_url(id))).await?, locale, now_ms)?,
            other => bail!("{other} lookups are not available or are turned off"),
        };
        if self.settings.filler {
            if let Some(mal) = mal_id.filter(|m| valid_id("mal", m)) {
                let mut fillers = Vec::new();
                for page in 1..=10 {
                    match self.transport.send(Request::Get(jikan_episodes_url(mal, page))).await {
                        Ok(value) => {
                            let (found, more) = parse_jikan_episodes(&value);
                            fillers.extend(found);
                            if !more {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                fillers.sort_unstable();
                fillers.dedup();
                enrichment.filler_episodes = fillers;
            }
        }
        Ok(enrichment)
    }
}

// ---------------------------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnrichedView {
    /// Where each shown field came from: `provider` or a service name.
    pub provenance: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailer_url: Option<String>,
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub recommendations: Vec<Recommendation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_airing_episode: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_airing_at_ms: Option<u64>,
    /// Episode ids (the app's stable ids) that the service marks as filler.
    #[serde(default)]
    pub filler_episode_ids: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Combine what the provider supplied with what the services know. The provider's values always
/// win unless the person prefers the metadata poster; metadata only fills gaps and adds extras.
/// Nothing in `details` is modified: this builds a separate view.
pub fn merge(details: &MediaDetails, enrichments: &[Enrichment], settings: &MetadataSettings) -> EnrichedView {
    let mut view = EnrichedView::default();
    let provider_poster = details.item.poster_url.clone();
    let provider_plot = details.plot.clone().filter(|p| !p.trim().is_empty());
    view.poster_url = provider_poster.clone();
    if provider_poster.is_some() {
        view.provenance.insert("poster".into(), "provider".into());
    }
    view.synopsis = provider_plot.clone();
    if provider_plot.is_some() {
        view.provenance.insert("synopsis".into(), "provider".into());
    }
    for e in enrichments {
        if e.poster_url.is_some() && (view.poster_url.is_none() || (settings.prefer_poster && view.provenance.get("poster").map_or(true, |p| p == "provider"))) {
            view.poster_url = e.poster_url.clone();
            view.provenance.insert("poster".into(), e.service.clone());
        }
        if view.backdrop_url.is_none() && e.banner_url.is_some() && details.background_poster_url.is_none() {
            view.backdrop_url = e.banner_url.clone();
            view.provenance.insert("backdrop".into(), e.service.clone());
        }
        if view.synopsis.is_none() && e.synopsis.is_some() {
            view.synopsis = e.synopsis.clone();
            view.provenance.insert("synopsis".into(), e.service.clone());
        }
        if view.genres.is_empty() && !e.genres.is_empty() && details.tags.is_empty() {
            view.genres = e.genres.clone();
            view.provenance.insert("genres".into(), e.service.clone());
        }
        if view.score.is_none() && e.score.is_some() && details.item.rating.is_none() && details.item.score.is_none() {
            view.score = e.score;
            view.provenance.insert("score".into(), e.service.clone());
        }
        if settings.trailer && view.trailer_url.is_none() && e.trailer_url.is_some() && details.trailers.is_empty() {
            view.trailer_url = e.trailer_url.clone();
            view.provenance.insert("trailer".into(), e.service.clone());
        }
        if settings.cast && view.cast.is_empty() && !e.cast.is_empty() && details.actors.is_empty() {
            view.cast = e.cast.clone();
            view.provenance.insert("cast".into(), e.service.clone());
        }
        if settings.recommendations && view.recommendations.is_empty() && !e.recommendations.is_empty() && details.recommendations.is_empty() {
            view.recommendations = e.recommendations.clone();
            view.provenance.insert("recommendations".into(), e.service.clone());
        }
        if view.next_airing_episode.is_none() && e.next_airing_episode.is_some() {
            view.next_airing_episode = e.next_airing_episode;
            view.next_airing_at_ms = e.next_airing_at_ms;
            view.provenance.insert("airing".into(), e.service.clone());
        }
    }
    // Filler flags are by absolute episode number, so they are only applied when the numbering is
    // unambiguous: a single season whose episodes carry their own numbers.
    let fillers: std::collections::BTreeSet<i32> = enrichments.iter().flat_map(|e| e.filler_episodes.iter().copied()).collect();
    if !fillers.is_empty() && settings.filler {
        let seasons: std::collections::BTreeSet<Option<i32>> = details.episodes.iter().map(|e| e.season).collect();
        let single_season = seasons.len() <= 1 || (seasons.len() == 2 && seasons.contains(&None) && seasons.contains(&Some(1)));
        if single_season {
            view.filler_episode_ids = details.episodes.iter().filter(|e| e.episode.is_some_and(|n| fillers.contains(&n))).filter_map(|e| e.id.clone()).collect();
            view.provenance.insert("filler".into(), enrichments.iter().find(|e| !e.filler_episodes.is_empty()).map(|e| e.service.clone()).unwrap_or_default());
        } else {
            view.notes.push("Filler flags were not applied: this title has several seasons, so episode numbers do not line up with the service's numbering.".into());
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_validated_per_service_and_simkl_is_unpacked() {
        let mut sync = BTreeMap::new();
        sync.insert("MAL".into(), "21".into());
        sync.insert("anilist".into(), "0".into());
        sync.insert("kitsu".into(), "12abc".into());
        sync.insert("simkl".into(), r#"{"Imdb":"tt0388629","Tmdb":"37854","Tvdb":"x"}"#.into());
        sync.insert("".into(), "999".into());
        sync.insert("unknown".into(), "5".into());
        let ids = external_ids(&sync);
        assert_eq!(ids.get("mal").map(String::as_str), Some("21"));
        assert_eq!(ids.get("imdb").map(String::as_str), Some("tt0388629"));
        assert_eq!(ids.get("tmdb").map(String::as_str), Some("37854"));
        assert_eq!(ids.len(), 3, "zero ids, malformed ids, empty and unknown keys are dropped: {ids:?}");
    }

    #[test]
    fn markup_is_stripped_to_text() {
        assert_eq!(strip_markup("<p>A <b>bold</b> &amp; <i>fine</i> tale.<br>Next</p>"), "A bold & fine tale. Next");
    }
}
