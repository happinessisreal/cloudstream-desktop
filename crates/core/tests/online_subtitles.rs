//! Online subtitle search through the real app and a real (headless) mpv, with scripted service
//! replies and an in-memory keyring.
mod support;
use cloudstream_core::{
    models::{compute_media_id, Episode, MediaItem, Stream},
    subtitle_online::SearchOptions,
    subtitle_providers::{
        secrets::{MemorySecrets, SecretStore},
        BoxFuture, HttpRequest, HttpResponse, ProviderProblem, Transport,
    },
    App, SubtitleFileSelection,
};
use serde_json::json;
use std::{
    io::Write,
    sync::{Arc, Mutex},
};

type Handler = Box<dyn Fn(&HttpRequest) -> HttpResponse + Send + Sync>;

#[derive(Default)]
struct Services {
    routes: Mutex<Vec<(String, Handler)>>,
    log: Mutex<Vec<HttpRequest>>,
}

impl Services {
    fn on(&self, needle: &str, handler: impl Fn(&HttpRequest) -> HttpResponse + Send + Sync + 'static) {
        self.routes.lock().unwrap().insert(0, (needle.to_string(), Box::new(handler)));
    }
    fn reply(&self, needle: &str, response: HttpResponse) {
        self.on(needle, move |_| response.clone());
    }
    fn count(&self, needle: &str) -> usize {
        self.log.lock().unwrap().iter().filter(|r| r.url.contains(needle)).count()
    }
    fn total(&self) -> usize {
        self.log.lock().unwrap().len()
    }
    fn last(&self, needle: &str) -> HttpRequest {
        self.log.lock().unwrap().iter().rev().find(|r| r.url.contains(needle)).cloned().unwrap_or_else(|| panic!("no request to {needle}"))
    }
}

impl Transport for Services {
    fn send<'a>(&'a self, request: HttpRequest) -> BoxFuture<'a, Result<HttpResponse, ProviderProblem>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(request.clone());
            let routes = self.routes.lock().unwrap();
            Ok(match routes.iter().find(|(needle, _)| request.url.contains(needle.as_str())) {
                Some((_, handler)) => handler(&request),
                None => HttpResponse::new(404, b"{}".to_vec()),
            })
        })
    }
}

fn json_reply(status: u16, body: serde_json::Value) -> HttpResponse {
    HttpResponse::new(status, body.to_string().into_bytes()).with_header("content-type", "application/json")
}

fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        for (name, text) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer.into_inner()
}

const SRT: &str = "1\n00:00:01,000 --> 00:00:03,000\nHello from the service\n";

fn subdl_listing() -> serde_json::Value {
    let sub = |url: &str, language: &str, episode: i64| json!({"release_name": format!("The.Show.S02E0{episode}.720p"), "name": "x", "url": url, "language": language, "releases": [format!("The.Show.S02E0{episode}.720p")], "author": "bob", "hi": false, "season": 2, "episode": episode, "full_season": false});
    json!({"status": true, "results": [{"sd_id": 1, "type": "tv", "name": "The Show", "year": 2011}], "subtitles": [sub("/subtitle/1-1.zip", "EN", 5), sub("/subtitle/1-2.zip", "EN", 6), sub("/subtitle/1-3.zip", "FR", 5)]})
}

struct Rig {
    app: Arc<App>,
    services: Arc<Services>,
    keyring: Arc<MemorySecrets>,
    _dir: tempfile::TempDir,
    _server: support::Fixture,
    url: String,
}

async fn rig() -> Option<Rig> {
    if !support::mpv_installed() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(30.0))]).await;
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), support::headless_player(dir.path())).unwrap();
    app.player.set_policy(cloudstream_core::models::PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let services = Arc::new(Services::default());
    let keyring = Arc::new(MemorySecrets::default());
    app.set_subtitle_transport(services.clone());
    app.set_subtitle_secrets(keyring.clone());
    let url = server.url("/v.wav");
    Some(Rig { app, services, keyring, _dir: dir, _server: server, url })
}

impl Rig {
    fn enable(&self) {
        self.app.store.update_settings(json!({"subtitleServices": {"enabled": true}}), Some("global"), None).unwrap();
    }

    async fn play(&self, profile: &str) -> String {
        let item = MediaItem { provider_id: "fixture".into(), name: "The Show".into(), url: "https://example.test/show".into(), year: Some(2011), media_type: Some("TvSeries".into()), ..Default::default() };
        let media_id = compute_media_id(&item.provider_id, &item.url, None);
        self.app.set_metadata_link(&media_id, "imdb", "tt0944947", None, None).unwrap();
        let stream = Stream { name: "Fixture".into(), url: self.url.clone(), ..Default::default() };
        let episode = Episode { name: "The Fifth".into(), data: "opaque".into(), season: Some(2), episode: Some(5), ..Default::default() };
        let session = self.app.play(stream, vec![], Some(item), Some(episode), 0.0, Some(profile), None).await.unwrap();
        support::wait_for_state(&self.app.player, 10, "playback", |s| s.duration > 29.0).await;
        session
    }

    async fn connect_subdl(&self, profile: &str) {
        self.services.reply("/api/v1/subtitles", json_reply(200, subdl_listing()));
        self.app.connect_subtitle_service(profile, "subdl", [("apiKey".to_string(), "SUBDL-SECRET-KEY".to_string())].into()).await.unwrap();
    }

    async fn subtitle_tracks(&self) -> usize {
        self.app.player.state().await.tracks.iter().filter(|t| t.kind == "sub").count()
    }
}

#[tokio::test]
async fn nothing_is_sent_anywhere_until_the_person_turns_the_feature_on() {
    let Some(rig) = rig().await else { return };
    let defaults = rig.app.store.get_settings().unwrap().global.subtitle_services;
    assert!(!defaults.enabled);
    let session = rig.play("default").await;
    assert!(rig.app.search_online_subtitles(&session, SearchOptions::default()).await.unwrap_err().to_string().contains("off"));
    assert!(rig.app.connect_subtitle_service("default", "subdl", [("apiKey".to_string(), "K".to_string())].into()).await.unwrap_err().to_string().contains("off"));
    assert_eq!(rig.services.total(), 0);
    assert!(rig.keyring.snapshot().is_empty());
}

#[tokio::test]
async fn connected_means_the_service_accepted_the_details_and_the_keyring_holds_them_for_that_profile_only() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    let view = rig.app.subtitle_services("default").await.unwrap();
    let state = |view: &cloudstream_core::subtitle_online::ServicesView, id: &str| serde_json::to_value(view.services.iter().find(|s| s.info.id == id).unwrap().state.clone()).unwrap()["status"].clone();
    assert_eq!((state(&view, "subdl"), state(&view, "opensubtitles"), state(&view, "addic7ed")), (json!("disconnected"), json!("disconnected"), json!("open")), "an unconnected service is not shown as connected, and one without accounts has nothing to connect");

    // A rejected key is refused and nothing is saved.
    rig.services.reply("/api/v1/subtitles", json_reply(403, json!({"error": "Invalid API key"})));
    let error = rig.app.connect_subtitle_service("default", "subdl", [("apiKey".to_string(), "WRONG".to_string())].into()).await.unwrap_err();
    assert!(error.to_string().contains("rejected"), "{error}");
    assert!(rig.keyring.snapshot().is_empty());

    // Missing fields are caught before any request.
    let before = rig.services.total();
    assert!(rig.app.connect_subtitle_service("default", "opensubtitles", [("apiKey".to_string(), "K".to_string())].into()).await.is_err());
    assert_eq!(rig.services.total(), before);
    assert!(rig.app.connect_subtitle_service("default", "addic7ed", Default::default()).await.is_err(), "nothing to connect for a service without accounts");

    rig.connect_subdl("default").await;
    let saved = rig.keyring.get("subdl", "default").await.unwrap().unwrap();
    assert_eq!(saved["apiKey"], "SUBDL-SECRET-KEY");
    for entry in std::fs::read_dir(rig._dir.path().join("data")).unwrap().flatten().filter(|e| e.path().is_file()) {
        let bytes = std::fs::read(entry.path()).unwrap();
        assert!(!bytes.windows(16).any(|w| w == b"SUBDL-SECRET-KEY"), "the key is not in {}", entry.path().display());
    }
    let after = rig.app.subtitle_services("default").await.unwrap();
    assert_eq!(state(&after, "subdl"), json!("connected"));
    let other = rig.app.subtitle_services("someone-else").await.unwrap();
    assert_eq!(state(&other, "subdl"), json!("disconnected"), "credentials belong to one profile");

    // Logging out removes local access.
    rig.app.disconnect_subtitle_service("default", "subdl").await.unwrap();
    assert!(rig.keyring.get("subdl", "default").await.unwrap().is_none());
    assert_eq!(state(&rig.app.subtitle_services("default").await.unwrap(), "subdl"), json!("disconnected"));
}

#[tokio::test]
async fn without_a_keyring_connecting_is_refused_and_no_request_is_made() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.keyring.down.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = rig.app.connect_subtitle_service("default", "subdl", [("apiKey".to_string(), "K".to_string())].into()).await.unwrap_err();
    assert!(error.to_string().contains("keyring"), "{error}");
    assert_eq!(rig.services.total(), 0, "details are never sent to a service that cannot be kept safely");
    let view = rig.app.subtitle_services("default").await.unwrap();
    assert_eq!(serde_json::to_value(&view.keyring).unwrap()["status"], "unavailable");
    let subdl = view.services.iter().find(|s| s.info.id == "subdl").unwrap();
    assert_eq!(serde_json::to_value(&subdl.state).unwrap()["status"], "keyring_unavailable");
    assert_eq!(serde_json::to_value(&view.services.iter().find(|s| s.info.id == "addic7ed").unwrap().state).unwrap()["status"], "open");
}

#[tokio::test]
async fn a_search_uses_the_playing_title_offers_results_and_selects_nothing() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    let session = rig.play("default").await;
    rig.services.reply("/api/v1/subtitles", json_reply(200, subdl_listing()));
    let found = rig.app.search_online_subtitles(&session, SearchOptions { providers: Some(vec!["subdl".into()]), languages: Some(vec!["en".into(), "fr".into()]), ..Default::default() }).await.unwrap();
    let request = url::Url::parse(&rig.services.last("/api/v1/subtitles").url).unwrap();
    let get = |k: &str| request.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    assert_eq!((get("imdb_id").as_deref(), get("season_number").as_deref(), get("episode_number").as_deref(), get("type").as_deref(), get("languages").as_deref()), (Some("tt0944947"), Some("2"), Some("5"), Some("tv"), Some("EN,FR")), "the query comes from the title that is playing");
    assert_eq!((found.title.as_str(), found.season, found.episode), ("The Show", Some(2), Some(5)));
    assert_eq!(found.results.iter().map(|r| (r.candidate.language.as_str(), r.candidate.episode)).collect::<Vec<_>>(), [("en", Some(5)), ("fr", Some(5))], "the other episode is not offered");
    assert!(found.results.iter().all(|r| r.candidate.matched_title.as_deref() == Some("The Show")));
    assert_eq!(found.outcomes.iter().map(|o| (o.provider.as_str(), o.status.as_str(), o.count)).collect::<Vec<_>>(), [("subdl", "ok", 2)]);
    assert_eq!(rig.subtitle_tracks().await, 0, "searching never loads a subtitle by itself");
    let json = serde_json::to_string(&found).unwrap();
    assert!(!json.contains("locator") && !json.contains("SUBDL-SECRET-KEY"), "nothing private reaches the interface");
    rig.app.player.stop().await;
}

#[tokio::test]
async fn choosing_a_result_downloads_it_and_the_player_gets_a_real_subtitle_track() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    let session = rig.play("default").await;
    rig.services.reply("/api/v1/subtitles", json_reply(200, subdl_listing()));
    let found = rig.app.search_online_subtitles(&session, SearchOptions::default()).await.unwrap();
    rig.services.reply("dl.subdl.com/subtitle/1-1.zip", HttpResponse::new(200, zip_of(&[("The.Show.S02E05.srt", SRT)])));
    let result = rig.app.use_online_subtitle(&found.search_id, 0).await.unwrap();
    assert!(matches!(result, SubtitleFileSelection::Added));
    let download = rig.services.last("dl.subdl.com");
    assert!(download.headers.contains(&("x-api-key".into(), "SUBDL-SECRET-KEY".into())) && !download.url.contains("SUBDL-SECRET-KEY"));
    let state = support::wait_for_state(&rig.app.player, 10, "online subtitle track", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected)).await;
    assert_eq!(state.tracks.iter().filter(|t| t.kind == "sub").count(), 1);
    rig.app.player.stop().await;
}

#[tokio::test]
async fn a_pack_loads_the_file_named_for_the_episode_or_asks_when_it_cannot_tell() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    let session = rig.play("default").await;
    rig.services.reply("/api/v1/subtitles", json_reply(200, subdl_listing()));
    let found = rig.app.search_online_subtitles(&session, SearchOptions::default()).await.unwrap();

    rig.services.reply("dl.subdl.com/subtitle/1-1.zip", HttpResponse::new(200, zip_of(&[("The.Show.S02E04.srt", SRT), ("The.Show.S02E05.srt", SRT), ("The.Show.S02E06.srt", SRT)])));
    assert!(matches!(rig.app.use_online_subtitle(&found.search_id, 0).await.unwrap(), SubtitleFileSelection::Added), "exactly one file is named for episode 5");
    support::wait_for_state(&rig.app.player, 10, "subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;

    rig.services.reply("dl.subdl.com/subtitle/1-1.zip", HttpResponse::new(200, zip_of(&[("english.srt", SRT), ("english-sdh.srt", SRT)])));
    match rig.app.use_online_subtitle(&found.search_id, 0).await.unwrap() {
        SubtitleFileSelection::Archive { entries, .. } => assert_eq!(entries.len(), 2, "when it cannot tell, the person chooses"),
        other => panic!("expected a choice: {other:?}"),
    }
    rig.app.player.stop().await;
}

#[tokio::test]
async fn stale_wrong_and_replaced_results_cannot_be_used() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    let session = rig.play("default").await;
    rig.services.reply("/api/v1/subtitles", json_reply(200, subdl_listing()));
    let first = rig.app.search_online_subtitles(&session, SearchOptions::default()).await.unwrap();
    assert!(rig.app.use_online_subtitle("not-a-search", 0).await.is_err());
    assert!(rig.app.use_online_subtitle(&first.search_id, 99).await.is_err());
    assert!(rig.app.search_online_subtitles("wrong-session", SearchOptions::default()).await.is_err());
    let second = rig.app.search_online_subtitles(&session, SearchOptions::default()).await.unwrap();
    assert!(rig.app.use_online_subtitle(&first.search_id, 0).await.is_err(), "a newer search replaces the older list");
    rig.app.player.stop().await;
    rig.services.reply("dl.subdl.com", HttpResponse::new(200, zip_of(&[("x.srt", SRT)])));
    assert!(rig.app.use_online_subtitle(&second.search_id, 0).await.is_err(), "results for a video that stopped are not loaded");
    assert_eq!(rig.services.count("dl.subdl.com"), 0);
}

#[tokio::test]
async fn one_service_failing_does_not_hide_another_and_the_reasons_stay_distinct() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    rig.services.reply("/api/v1/movies/search", json_reply(200, json!({"data": [{"movieId": 77, "title": "The Show", "releaseYear": 2011}]})));
    rig.app.connect_subtitle_service("default", "subsource", [("apiKey".to_string(), "SS-SECRET".to_string())].into()).await.unwrap();
    let session = rig.play("default").await;
    rig.services.reply("/api/v1/subtitles?", json_reply(402, json!({"error": "paid_api_required"})));
    rig.services.reply("api.subsource.net/api/v1/subtitles?", json_reply(200, json!({"success": true, "data": [{"subtitleId": 501, "language": "English", "releaseInfo": "The.Show.S02E05.720p", "hearingImpaired": false}]})));
    let found = rig.app.search_online_subtitles(&session, SearchOptions { providers: Some(vec!["subdl".into(), "subsource".into(), "opensubtitles".into()]), ..Default::default() }).await.unwrap();
    let outcome = |id: &str| found.outcomes.iter().find(|o| o.provider == id).map(|o| o.status.clone()).unwrap();
    assert_eq!((outcome("subdl").as_str(), outcome("subsource").as_str(), outcome("opensubtitles").as_str()), ("paid_plan_required", "ok", "not_connected"));
    assert_eq!(found.results.len(), 1);
    assert_eq!(found.results[0].candidate.provider, "subsource");
    assert_eq!(rig.services.count("opensubtitles.com"), 0, "a service that is not connected is not contacted");
    let diagnostics = serde_json::to_string(&rig.app.diagnostics.events()).unwrap();
    assert!(!diagnostics.contains("SS-SECRET") && !diagnostics.contains("SUBDL-SECRET-KEY"), "keys never reach the diagnostics log: {diagnostics}");
    rig.app.player.stop().await;
}

#[tokio::test]
async fn another_profile_playing_does_not_use_this_profiles_credentials() {
    let Some(rig) = rig().await else { return };
    rig.enable();
    rig.connect_subdl("default").await;
    let session = rig.play("kid").await;
    let before = rig.services.count("/api/v1/subtitles");
    let found = rig.app.search_online_subtitles(&session, SearchOptions { providers: Some(vec!["subdl".into()]), ..Default::default() }).await.unwrap();
    assert_eq!(found.outcomes[0].status, "not_connected");
    assert_eq!(rig.services.count("/api/v1/subtitles"), before);
    rig.app.player.stop().await;
}

#[tokio::test]
async fn service_switches_and_languages_are_validated_and_obeyed() {
    let Some(rig) = rig().await else { return };
    for bad in [json!({"subtitleServices": {"providers": {"nope": true}}}), json!({"subtitleServices": {"languages": ["en", "x y"]}}), json!({"subtitleServices": {"languages": ["a", "b", "c", "d", "e", "f", "g", "h", "i"]}})] {
        assert!(rig.app.store.update_settings(bad, Some("global"), None).is_err());
    }
    rig.enable();
    rig.connect_subdl("default").await;
    rig.app.store.update_settings(json!({"subtitleServices": {"providers": {"subdl": false}, "languages": ["fr"]}}), Some("global"), None).unwrap();
    let session = rig.play("default").await;
    let error = rig.app.search_online_subtitles(&session, SearchOptions { providers: Some(vec!["subdl".into()]), ..Default::default() }).await.unwrap_err();
    assert!(error.to_string().contains("switched off"), "{error}");
    rig.app.store.update_settings(json!({"subtitleServices": {"providers": {"subdl": true}}}), Some("global"), None).unwrap();
    let found = rig.app.search_online_subtitles(&session, SearchOptions { providers: Some(vec!["subdl".into()]), ..Default::default() }).await.unwrap();
    assert_eq!(found.languages, ["fr"], "the saved language choice is used when none is given");
    rig.app.player.stop().await;
}
