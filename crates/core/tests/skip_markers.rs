mod support;
use cloudstream_core::{metadata::{BoxFuture, MatchSource, MetadataLink}, models::{Episode, MediaItem, Stream, PlaybackPolicy, compute_media_id}, skip::{self, Client, SkipSettings, Transport}, App};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

struct Fixture { requests: Arc<Mutex<Vec<String>>>, response: Value }
impl Transport for Fixture {
    fn get(&self, url: String) -> BoxFuture<Value> {
        self.requests.lock().unwrap().push(url);
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
    }
}
struct Delayed { started: Arc<tokio::sync::Notify>, release: Arc<tokio::sync::Notify> }
impl Transport for Delayed {
    fn get(&self, _url: String) -> BoxFuture<Value> {
        let started = self.started.clone(); let release = self.release.clone();
        Box::pin(async move {
            started.notify_one();
            release.notified().await;
            Ok(json!({"found":true,"results":[{"skipType":"op","interval":{"startTime":0,"endTime":5},"episodeLength":30}]}))
        })
    }
}
fn link(media: &str, service: &str, id: &str) -> MetadataLink {
    MetadataLink { media_id: media.into(), service: service.into(), external_id: id.into(), source: MatchSource::User, linked_at_ms: 1, label: None, year: None }
}
fn episode(season: Option<i32>, number: Option<i32>) -> Episode { Episode { season, episode: number, ..Default::default() } }

#[tokio::test]
async fn disabled_unmatched_or_ambiguous_episodes_never_contact_service() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let client = Client::new(Arc::new(Fixture { requests: requests.clone(), response: json!({"found":false}) }));
    let off = SkipSettings::default();
    let on = SkipSettings { aniskip: true, ..Default::default() };
    let links = [link("media-1", "mal", "42")];
    assert!(client.lookup(&off, "media-1", &links, &episode(Some(1), Some(1)), 1440.0).await.unwrap().is_empty());
    assert!(client.lookup(&on, "different-media", &links, &episode(Some(1), Some(1)), 1440.0).await.unwrap().is_empty());
    assert!(client.lookup(&on, "media-1", &[link("media-1", "anilist", "42")], &episode(Some(1), Some(1)), 1440.0).await.unwrap().is_empty());
    assert!(client.lookup(&on, "media-1", &links, &episode(Some(2), Some(1)), 1440.0).await.unwrap().is_empty());
    assert!(client.lookup(&on, "media-1", &links, &episode(None, None), 1440.0).await.unwrap().is_empty());
    assert!(client.lookup(&on, "media-1", &links, &episode(None, Some(1)), 0.0).await.unwrap().is_empty());
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_mal_id_uses_only_official_endpoint_and_documented_query() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let response = json!({"found": true, "results": [{"skipType":"op", "interval":{"startTime":70.0,"endTime":160.0}, "episodeLength":1440.0}]});
    let client = Client::new(Arc::new(Fixture { requests: requests.clone(), response }));
    let result = client.lookup(&SkipSettings { aniskip: true, ..Default::default() }, "media-1", &[link("media-1", "mal", "42")], &episode(Some(1), Some(3)), 1440.0).await.unwrap();
    assert_eq!(result.len(), 1); assert_eq!(result[0].kind, "op"); assert_eq!(result[0].end, 160.0);
    let requests = requests.lock().unwrap();
    let url = url::Url::parse(&requests[0]).unwrap();
    assert_eq!(url.scheme(), "https"); assert_eq!(url.host_str(), Some("api.aniskip.com")); assert_eq!(url.path(), "/v2/skip-times/42/3");
    let query: Vec<_> = url.query_pairs().collect();
    assert!(query.iter().any(|(k, v)| k == "episodeLength" && v == "1440.000"));
    assert_eq!(query.iter().filter(|(k, _)| k == "types").count(), 5);
}

#[test]
fn malformed_or_wrong_cut_intervals_cannot_produce_unsafe_seek_targets() {
    let response = json!({"found":true,"results":[
        {"skipType":"op","interval":{"startTime":10,"endTime":90},"episodeLength":1440},
        {"skipType":"op","interval":{"startTime":10,"endTime":90},"episodeLength":1440},
        {"skipType":"ed","interval":{"startTime":1400,"endTime":1440.5},"episodeLength":1440},
        {"skipType":"ed","interval":{"startTime":100,"endTime":1500},"episodeLength":1440},
        {"skipType":"op","interval":{"startTime":-1,"endTime":100},"episodeLength":1440},
        {"skipType":"op","interval":{"startTime":50,"endTime":49},"episodeLength":1440},
        {"skipType":"op","interval":{"startTime":20,"endTime":80},"episodeLength":1200},
        {"skipType":"other","interval":{"startTime":20,"endTime":80},"episodeLength":1440}
    ]});
    let markers = skip::parse_response(&response, 1440.0).unwrap();
    assert_eq!(markers.len(), 2); assert_eq!(markers[0].end, 90.0); assert_eq!(markers[1].end, 1440.0);
    assert!(skip::parse_response(&json!({"found":true,"results":vec![json!({}); 65]}), 1440.0).is_err());
    assert!(skip::parse_response(&json!({"found":false}), 1440.0).unwrap().is_empty());
}

#[test]
fn invalid_ids_are_never_normalized_into_a_match() {
    for id in ["", "0", "-1", " 42", "12/3", "4294967296"] {
        assert_eq!(skip::matching_mal_id("m", &[link("m", "mal", id)]), None, "{id}");
    }
}

#[test]
fn skip_service_is_off_by_default_and_the_choice_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("app.db");
    let store = cloudstream_core::store::Store::open(&database).unwrap();
    assert!(!store.get_settings().unwrap().global.skip.aniskip);
    store.update_settings(json!({"skip":{"aniskip":true}}), Some("global"), None).unwrap();
    drop(store);
    assert!(cloudstream_core::store::Store::open(&database).unwrap().get_settings().unwrap().global.skip.aniskip);
}

#[tokio::test]
async fn active_player_uses_linked_id_obeys_preference_and_skips_only_its_own_session() {
    if !support::mpv_installed() { return; }
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(30.0))]).await;
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), support::headless_player(dir.path())).unwrap();
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let requests = Arc::new(Mutex::new(Vec::new()));
    app.set_skip_transport(Arc::new(Fixture { requests: requests.clone(), response: json!({"found":true,"results":[{"skipType":"op","interval":{"startTime":0,"endTime":5},"episodeLength":30}]}) }));
    let item = MediaItem { provider_id: "fixture".into(), name: "A show".into(), url: "https://example.test/show".into(), ..Default::default() };
    let media_id = compute_media_id(&item.provider_id, &item.url, None);
    app.set_metadata_link(&media_id, "mal", "42", None, None).unwrap();
    let stream = Stream { name: "Fixture".into(), url: server.url("/v.wav"), ..Default::default() };
    let episode = Episode { name: "Episode 1".into(), data: "opaque".into(), episode: Some(1), season: Some(1), ..Default::default() };
    let session = app.play(stream, vec![], Some(item), Some(episode), 0.0, Some("default"), None).await.unwrap();
    support::wait_for_state(&app.player, 10, "fixture duration", |state| state.duration > 29.0).await;
    assert!(app.player_skip_markers(&session).await.unwrap().is_empty());
    assert!(requests.lock().unwrap().is_empty());
    app.store.update_settings(json!({"skip":{"aniskip":true}}), Some("global"), None).unwrap();
    assert!(app.player_skip_markers("wrong-session").await.is_err());
    let markers = app.player_skip_markers(&session).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(app.player.skip_interval("wrong-session", 0.0, 5.0).await.is_err());
    assert!(app.player.skip_interval(&session, 0.0, 50.0).await.is_err());
    app.player.skip_interval(&session, markers[0].start, markers[0].end).await.unwrap();
    support::wait_for_state(&app.player, 5, "skip seek", |state| state.position >= 4.9).await;
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    app.set_skip_transport(Arc::new(Delayed { started: started.clone(), release: release.clone() }));
    let clone = app.clone(); let delayed_session = session.clone();
    let pending = tokio::spawn(async move { clone.player_skip_markers(&delayed_session).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified()).await.unwrap();
    app.store.update_settings(json!({"skip":{"aniskip":false}}), Some("global"), None).unwrap();
    release.notify_one();
    assert!(pending.await.unwrap().unwrap().is_empty(), "disabling while a request is in flight discards its result");
    assert!(app.player_skip_markers(&session).await.unwrap().is_empty());
    assert_eq!(requests.lock().unwrap().len(), 1);
    app.player.stop().await;
}

#[tokio::test]
async fn animeskip_and_theintrodb_adapters_parse_and_validate_correctly() {
    let anime_json = json!([
        {"type": "opening", "startTime": 12.5, "endTime": 102.5},
        {"type": "ending", "startTime": 1300.0, "endTime": 1390.0},
        {"type": "recap", "startTime": 0.0, "endTime": 12.0},
        {"type": "unsupported", "startTime": 200.0, "endTime": 300.0}
    ]);
    let anime_markers = skip::parse_animeskip_response(&anime_json, 1440.0).unwrap();
    assert_eq!(anime_markers.len(), 3);
    assert_eq!(anime_markers[0].kind, "recap");
    assert_eq!(anime_markers[0].source, "AnimeSkip");
    assert_eq!(anime_markers[1].kind, "op");
    assert_eq!(anime_markers[2].kind, "ed");

    let introdb_json = json!({
        "timestamps": [
            {"type": "intro", "start": 30.0, "end": 90.0},
            {"type": "outro", "start": 1200.0, "end": 1260.0}
        ]
    });
    let introdb_markers = skip::parse_theintrodb_response(&introdb_json, 1300.0).unwrap();
    assert_eq!(introdb_markers.len(), 2);
    assert_eq!(introdb_markers[0].kind, "op");
    assert_eq!(introdb_markers[0].source, "TheIntroDB");
    assert_eq!(introdb_markers[1].kind, "ed");

    // Test client lookup with multiple adapters enabled
    let requests = Arc::new(Mutex::new(Vec::new()));
    let client = Client::new(Arc::new(Fixture {
        requests: requests.clone(),
        response: json!([{"type": "intro", "startTime": 10.0, "endTime": 70.0}]),
    }));
    let settings = SkipSettings { animeskip: true, ..Default::default() };
    let links = [link("media-1", "kitsu", "1234")];
    let markers = client.lookup(&settings, "media-1", &links, &episode(Some(1), Some(1)), 1440.0).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].start, 10.0);
    assert_eq!(markers[0].end, 70.0);
    assert!(requests.lock().unwrap()[0].contains("api.animeskip.com"));
}

