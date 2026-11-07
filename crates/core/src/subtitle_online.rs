//! Online subtitle search for the playing title (T22): connecting services per profile, searching
//! the ones the person has switched on, and loading a chosen result into the player.
//!
//! The rules this layer keeps:
//!
//! * nothing is sent anywhere until the person turns the feature on, and only to the services they
//!   have connected (or that need no account);
//! * the query comes from the title that is actually playing, with IDs only when the source or the
//!   person supplied them;
//! * results are *offered*; one is downloaded only when chosen, by position in the list the app
//!   itself produced, so the interface can never name an address to fetch;
//! * credentials live in the OS keyring, per profile and per service, and disconnecting deletes
//!   them; with no keyring, connecting is refused rather than falling back to plain text.
use crate::subtitle_providers::{
    self, secrets::{SecretStore, SecretTool}, Candidate, Credentials, HearingImpaired, ProviderInfo, ProviderProblem, ReqwestTransport, SubtitleQuery, Transport,
};
use crate::{subtitles, App, SubtitleFileSelection};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(40);
const RESULT_LIMIT: usize = 100;
const SEARCH_LIFETIME: Duration = Duration::from_secs(900);

/// The last search, kept so that a result can be chosen by position.
pub struct OnlineSearch {
    id: String,
    session_id: String,
    profile_id: String,
    query: SubtitleQuery,
    candidates: Vec<Candidate>,
    created: std::time::Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ServiceState {
    Connected,
    Disconnected,
    /// The service needs no account; there is nothing to connect.
    Open,
    /// Whether details are saved cannot be read because there is no usable keyring.
    KeyringUnavailable { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    #[serde(flatten)]
    pub info: ProviderInfo,
    pub enabled: bool,
    pub state: ServiceState,
    /// The account name when the service has one (never a password, key or token).
    pub account: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesView {
    pub enabled: bool,
    pub hearing_impaired: HearingImpaired,
    pub languages: Vec<String>,
    pub keyring: KeyringView,
    pub services: Vec<ServiceView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KeyringView {
    Ready,
    Unavailable { message: String },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchOptions {
    pub providers: Option<Vec<String>>,
    pub languages: Option<Vec<String>>,
    pub hearing_impaired: Option<HearingImpaired>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub provider: String,
    pub name: String,
    /// `ok`, `no_results`, or the kind of problem.
    pub status: String,
    pub count: usize,
    pub message: Option<String>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView {
    pub index: usize,
    #[serde(flatten)]
    pub candidate: Candidate,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchView {
    pub search_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub languages: Vec<String>,
    pub results: Vec<ResultView>,
    pub outcomes: Vec<Outcome>,
}

fn problem_status(problem: &ProviderProblem) -> (&'static str, Option<u64>) {
    match problem {
        ProviderProblem::NotConnected => ("not_connected", None),
        ProviderProblem::SignInRejected => ("sign_in_rejected", None),
        ProviderProblem::RateLimited { retry_after_secs } => ("rate_limited", *retry_after_secs),
        ProviderProblem::QuotaExhausted { retry_after_secs } => ("quota_exhausted", *retry_after_secs),
        ProviderProblem::PaidPlanRequired => ("paid_plan_required", None),
        ProviderProblem::Blocked => ("blocked", None),
        ProviderProblem::Unavailable => ("unavailable", None),
        ProviderProblem::BadResponse => ("bad_response", None),
        ProviderProblem::NotSupported => ("not_supported", None),
        ProviderProblem::NotFound => ("not_found", None),
    }
}

fn problem_outcome(info: &ProviderInfo, problem: &ProviderProblem) -> Outcome {
    let (status, retry_after_secs) = problem_status(problem);
    Outcome { provider: info.id.into(), name: info.name.into(), status: status.into(), count: 0, message: Some(problem.to_string()), retry_after_secs }
}

/// The subtitle format of downloaded text, from its content first and its name second.
fn subtitle_extension(file_name: &str, bytes: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        return Some("vtt");
    }
    if head.starts_with("[Script Info]") {
        return Some(if file_name.to_ascii_lowercase().ends_with(".ssa") { "ssa" } else { "ass" });
    }
    if head.to_ascii_lowercase().starts_with("<sami") {
        return Some("smi");
    }
    if head.contains("-->") {
        return Some("srt");
    }
    None
}

impl App {
    pub fn set_subtitle_transport(&self, transport: Arc<dyn Transport>) {
        *self.subtitle_http.lock().unwrap_or_else(|e| e.into_inner()) = Some(transport);
    }

    pub fn set_subtitle_secrets(&self, secrets: Arc<dyn SecretStore>) {
        *self.subtitle_secrets.lock().unwrap_or_else(|e| e.into_inner()) = Some(secrets);
    }

    fn subtitle_transport(&self) -> Result<Arc<dyn Transport>> {
        let mut guard = self.subtitle_http.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(Arc::new(ReqwestTransport::new()?));
        }
        Ok(guard.clone().expect("set above"))
    }

    fn secrets(&self) -> Arc<dyn SecretStore> {
        let mut guard = self.subtitle_secrets.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(|| Arc::new(SecretTool::new())).clone()
    }

    fn require_subtitle_services_enabled(&self) -> Result<subtitle_providers::ServiceSettings> {
        let settings = self.store.get_settings()?.global.subtitle_services;
        if !settings.enabled {
            bail!("Online subtitle services are off. Turn them on in Settings first.");
        }
        Ok(settings)
    }

    /// Every service with its switch and whether details are saved for this profile.
    pub async fn subtitle_services(&self, profile_id: &str) -> Result<ServicesView> {
        let settings = self.store.get_settings()?.global.subtitle_services;
        let secrets = self.secrets();
        let keyring_problem = secrets.available().await.err();
        let mut services = Vec::new();
        for provider in subtitle_providers::all() {
            let info = provider.info();
            let (state, account) = if info.fields.is_empty() {
                (ServiceState::Open, None)
            } else if let Some(problem) = &keyring_problem {
                (ServiceState::KeyringUnavailable { message: problem.to_string() }, None)
            } else {
                match secrets.get(info.id, profile_id).await {
                    Ok(Some(saved)) if info.fields.iter().filter(|f| f.required).all(|f| saved.get(f.name).is_some_and(|v| !v.is_empty())) => (ServiceState::Connected, saved.get("username").cloned()),
                    Ok(_) => (ServiceState::Disconnected, None),
                    Err(problem) => (ServiceState::KeyringUnavailable { message: problem.to_string() }, None),
                }
            };
            services.push(ServiceView { enabled: settings.provider_enabled(info.id), info, state, account });
        }
        Ok(ServicesView {
            enabled: settings.enabled,
            hearing_impaired: settings.hearing_impaired,
            languages: settings.languages,
            keyring: match keyring_problem {
                None => KeyringView::Ready,
                Some(problem) => KeyringView::Unavailable { message: problem.to_string() },
            },
            services,
        })
    }

    /// Check the details with the service and, only if it accepts them, save them for this profile.
    pub async fn connect_subtitle_service(&self, profile_id: &str, provider_id: &str, fields: Credentials) -> Result<ServiceView> {
        self.require_subtitle_services_enabled()?;
        let provider = subtitle_providers::by_id(provider_id).context("Unknown subtitle service")?;
        let info = provider.info();
        if info.fields.is_empty() {
            bail!("{} needs no account, so there is nothing to connect.", info.name);
        }
        let secrets = self.secrets();
        secrets.available().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        for field in info.fields.iter().filter(|f| f.required) {
            if fields.get(field.name).is_none_or(|v| v.trim().is_empty()) {
                bail!("Enter your {}.", field.label.to_lowercase());
            }
        }
        if fields.values().any(|v| v.len() > 512 || v.chars().any(char::is_control)) {
            bail!("One of the entries is not valid.");
        }
        let http = self.subtitle_transport()?;
        let saved = match tokio::time::timeout(SEARCH_TIMEOUT, provider.connect(&*http, &fields)).await {
            Ok(Ok(saved)) => saved,
            Ok(Err(problem)) => {
                self.diagnostics.record("subtitles", &format!("Connecting {} failed: {}", info.id, problem));
                bail!("{problem}")
            }
            Err(_) => bail!("{}", ProviderProblem::Unavailable),
        };
        secrets.set(info.id, profile_id, &saved).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let settings = self.store.get_settings()?.global.subtitle_services;
        Ok(ServiceView { enabled: settings.provider_enabled(info.id), info, state: ServiceState::Connected, account: saved.get("username").cloned() })
    }

    /// Delete the saved details for this profile. Local access is gone immediately; a sign-in the
    /// service issued simply expires on its side.
    pub async fn disconnect_subtitle_service(&self, profile_id: &str, provider_id: &str) -> Result<()> {
        let provider = subtitle_providers::by_id(provider_id).context("Unknown subtitle service")?;
        self.secrets().delete(provider.info().id, profile_id).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        // A result list fetched with those details must not outlive them.
        let mut search = self.online_subtitles.lock().await;
        if search.as_ref().is_some_and(|s| s.profile_id == profile_id && s.candidates.iter().any(|c| c.provider == provider_id)) {
            *search = None;
        }
        Ok(())
    }

    /// Everything a query needs, taken from what is playing right now.
    async fn playback_query(&self, session_id: &str, settings: &subtitle_providers::ServiceSettings, options: &SearchOptions) -> Result<(SubtitleQuery, String)> {
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id) {
            bail!("Playback changed. Start the video again before searching for subtitles.");
        }
        let (watch, profile) = self.player.current_watch().await.context("Nothing is playing")?;
        let media_id = watch.item.media_id.clone();
        let supplied = self.cached_subscription_details(&profile, &watch.item.provider_id, &watch.item.url).map(|d| d.sync_data).unwrap_or_default();
        let (mut imdb_id, mut tmdb_id) = (None, None);
        if let Some(media_id) = media_id.as_deref() {
            for link in self.metadata_links(media_id, &supplied)? {
                match link.service.as_str() {
                    "imdb" => imdb_id = Some(link.external_id.clone()),
                    "tmdb" => tmdb_id = Some(link.external_id.clone()),
                    _ => {}
                }
            }
        }
        let is_movie = watch.item.media_type.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("movie"));
        let player_language = self.player.preferences().await.subtitle_language;
        let languages: Vec<String> = options
            .languages
            .clone()
            .filter(|l| !l.is_empty())
            .or_else(|| Some(settings.languages.clone()).filter(|l| !l.is_empty()))
            .or_else(|| player_language.map(|l| vec![crate::hls::language_code(&l)]).filter(|l| l.iter().all(|c| !c.is_empty())))
            .unwrap_or_else(|| vec!["en".into()]);
        let query = SubtitleQuery {
            title: watch.item.name.clone(),
            year: watch.item.year,
            season: if is_movie { None } else { watch.episode.season },
            episode: if is_movie { None } else { watch.episode.episode },
            imdb_id,
            tmdb_id,
            languages,
            hearing_impaired: options.hearing_impaired.unwrap_or(settings.hearing_impaired),
        }
        .cleaned();
        Ok((query, profile))
    }

    /// Ask every switched-on service about the title that is playing.
    pub async fn search_online_subtitles(&self, session_id: &str, options: SearchOptions) -> Result<SearchView> {
        let settings = self.require_subtitle_services_enabled()?;
        let (query, profile) = self.playback_query(session_id, &settings, &options).await?;
        let http = self.subtitle_transport()?;
        let secrets = self.secrets();
        let wanted: Vec<_> = subtitle_providers::all()
            .into_iter()
            .filter(|p| settings.provider_enabled(p.info().id) && options.providers.as_ref().is_none_or(|ids| ids.iter().any(|id| id == p.info().id)))
            .collect();
        if wanted.is_empty() {
            bail!("Every subtitle service is switched off. Turn one on in Settings.");
        }
        let mut outcomes = Vec::new();
        let mut running = tokio::task::JoinSet::new();
        for provider in wanted {
            let info = provider.info();
            let mut credentials = Credentials::new();
            if provider.needs_connection() {
                match secrets.get(info.id, &profile).await {
                    Ok(Some(saved)) => credentials = saved,
                    Ok(None) => {
                        outcomes.push(problem_outcome(&info, &ProviderProblem::NotConnected));
                        continue;
                    }
                    Err(problem) => {
                        outcomes.push(Outcome { provider: info.id.into(), name: info.name.into(), status: "keyring_unavailable".into(), count: 0, message: Some(problem.to_string()), retry_after_secs: None });
                        continue;
                    }
                }
            }
            let (http, query) = (http.clone(), query.clone());
            running.spawn(async move {
                let before = credentials.clone();
                let result = tokio::time::timeout(SEARCH_TIMEOUT, provider.search(&*http, &mut credentials, &query)).await.unwrap_or(Err(ProviderProblem::Unavailable));
                (info, result, (before != credentials).then_some(credentials))
            });
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        while let Some(joined) = running.join_next().await {
            let Ok((info, result, updated)) = joined else { continue };
            if let Some(updated) = updated {
                let _ = secrets.set(info.id, &profile, &updated).await;
            }
            match result {
                Ok(found) => {
                    outcomes.push(Outcome { provider: info.id.into(), name: info.name.into(), status: if found.is_empty() { "no_results" } else { "ok" }.into(), count: found.len(), message: None, retry_after_secs: None });
                    candidates.extend(found);
                }
                Err(problem) => {
                    self.diagnostics.record("subtitles", &format!("Searching {} failed: {}", info.id, problem));
                    outcomes.push(problem_outcome(&info, &problem));
                }
            }
        }
        outcomes.sort_by(|a, b| a.provider.cmp(&b.provider));
        subtitle_providers::rank(&query, &mut candidates);
        candidates.truncate(RESULT_LIMIT);

        // A result for a video that is no longer playing is useless; say so rather than show it.
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id) {
            bail!("Playback changed while searching. Search again.");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let results = candidates.iter().cloned().enumerate().map(|(index, candidate)| ResultView { index, candidate }).collect();
        let view = SearchView { search_id: id.clone(), title: query.title.clone(), year: query.year, season: query.season, episode: query.episode, languages: query.languages.clone(), results, outcomes };
        *self.online_subtitles.lock().await = Some(OnlineSearch { id, session_id: session_id.into(), profile_id: profile, query, candidates, created: std::time::Instant::now() });
        Ok(view)
    }

    /// Download one offered result and load it into the player. A ZIP with several subtitles
    /// becomes a choice, unless exactly one of them is named for the requested episode.
    pub async fn use_online_subtitle(&self, search_id: &str, index: usize) -> Result<SubtitleFileSelection> {
        let settings = self.require_subtitle_services_enabled()?;
        let (candidate, session_id, profile, query) = {
            let guard = self.online_subtitles.lock().await;
            let search = guard.as_ref().filter(|s| s.id == search_id).context("Those results are no longer available. Search again.")?;
            if search.created.elapsed() > SEARCH_LIFETIME {
                bail!("Those results have expired. Search again.");
            }
            (search.candidates.get(index).cloned().context("That result is no longer in the list. Search again.")?, search.session_id.clone(), search.profile_id.clone(), search.query.clone())
        };
        let state = self.player.state().await;
        if !state.active || state.session_id.as_deref() != Some(session_id.as_str()) {
            bail!("Playback changed. Search for subtitles again.");
        }
        if !settings.provider_enabled(&candidate.provider) {
            bail!("That subtitle service is switched off.");
        }
        let provider = subtitle_providers::by_id(&candidate.provider).context("Unknown subtitle service")?;
        let info = provider.info();
        let secrets = self.secrets();
        let mut credentials = Credentials::new();
        if provider.needs_connection() {
            credentials = secrets.get(info.id, &profile).await.map_err(|e| anyhow::anyhow!("{e}"))?.ok_or_else(|| anyhow::anyhow!("{}", ProviderProblem::NotConnected))?;
        }
        let before = credentials.clone();
        let http = self.subtitle_transport()?;
        let downloaded = tokio::time::timeout(DOWNLOAD_TIMEOUT, provider.download(&*http, &mut credentials, &candidate)).await.unwrap_or(Err(ProviderProblem::Unavailable));
        if before != credentials {
            let _ = secrets.set(info.id, &profile, &credentials).await;
        }
        let downloaded = match downloaded {
            Ok(downloaded) => downloaded,
            Err(problem) => {
                self.diagnostics.record("subtitles", &format!("Downloading from {} failed: {}", info.id, problem));
                bail!("{problem}")
            }
        };
        let current = self.player.state().await;
        if !current.active || current.session_id.as_deref() != Some(session_id.as_str()) {
            bail!("Playback changed while downloading. Search again.");
        }
        let label = format!("{} ({})", info.name, candidate.language);
        if downloaded.bytes.starts_with(b"PK\x03\x04") {
            let entries = subtitles::list_archive(&downloaded.bytes)?;
            // In a pack, the file named for the requested episode is the one asked for.
            let named: Vec<_> = match (query.season, query.episode) {
                (_, Some(episode)) if entries.len() > 1 => entries
                    .iter()
                    .filter(|e| {
                        let (season, ep) = subtitle_providers::season_episode_from_release(&e.name);
                        ep == Some(episode) && query.season.is_none_or(|s| season.is_none_or(|found| found == s))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let pick = if entries.len() == 1 { Some(entries[0].index) } else if named.len() == 1 { Some(named[0].index) } else { None };
            return match pick {
                Some(entry) => {
                    self.player.add_archived_subtitle(&downloaded.bytes, entry, &session_id).await?;
                    Ok(SubtitleFileSelection::Added)
                }
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    *self.pending_subtitle_archive.lock().await = Some(crate::PendingSubtitleArchive { id: id.clone(), session_id, bytes: downloaded.bytes, created: std::time::Instant::now() });
                    Ok(SubtitleFileSelection::Archive { id, entries })
                }
            };
        }
        let extension = subtitle_extension(&downloaded.file_name, &downloaded.bytes).context("The service returned something that is not a subtitle")?;
        self.player.add_subtitle_bytes(&downloaded.bytes, extension, &label, &session_id).await?;
        Ok(SubtitleFileSelection::Added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_comes_from_content_not_from_a_name_the_service_chose() {
        assert_eq!(subtitle_extension("evil.exe", b"WEBVTT\n\n"), Some("vtt"));
        assert_eq!(subtitle_extension("x.srt", b"1\n00:00:01,000 --> 00:00:02,000\nHi"), Some("srt"));
        assert_eq!(subtitle_extension("x.ssa", b"[Script Info]\nTitle: x"), Some("ssa"));
        assert_eq!(subtitle_extension("x.ass", b"[Script Info]\nTitle: x"), Some("ass"));
        assert_eq!(subtitle_extension("x.srt", b"<html>nope</html>"), None);
    }

    #[test]
    fn problems_map_to_stable_statuses() {
        assert_eq!(problem_status(&ProviderProblem::QuotaExhausted { retry_after_secs: Some(5) }), ("quota_exhausted", Some(5)));
        assert_eq!(problem_status(&ProviderProblem::PaidPlanRequired).0, "paid_plan_required");
        assert_eq!(problem_status(&ProviderProblem::SignInRejected).0, "sign_in_rejected");
    }
}
