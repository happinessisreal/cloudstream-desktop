//! Metadata matching and enrichment (T24): fixtures shaped like the services' documented
//! responses, a scriptable fake network, and checks that provider data is never touched.
use cloudstream_core::{metadata::*, models::*, App};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::{Arc, Mutex}};

type Handler = Box<dyn Fn(&Request) -> anyhow::Result<Value> + Send + Sync>;

/// A fake network that records every request and answers from a closure.
struct Fake {
    seen: Mutex<Vec<String>>,
    handler: Handler,
}

impl Fake {
    fn new(handler: impl Fn(&Request) -> anyhow::Result<Value> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { seen: Mutex::new(Vec::new()), handler: Box::new(handler) })
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl Transport for Fake {
    fn send(&self, request: Request) -> BoxFuture<Value> {
        let label = match &request {
            Request::Get(url) => format!("GET {url}"),
            Request::Post(url, body) => format!("POST {url} {}", body["variables"]),
        };
        self.seen.lock().unwrap().push(label);
        let result = (self.handler)(&request);
        Box::pin(async move { result })
    }
}

fn anilist_search() -> Value {
    json!({ "data": { "Page": { "media": [
        { "id": 25, "idMal": 25, "title": { "romaji": "Hagane no Renkinjutsushi", "english": "Fullmetal Alchemist", "native": "鋼の錬金術師" }, "startDate": { "year": 2003 }, "format": "TV", "coverImage": { "large": "https://s4.anilist.co/a.jpg" } },
        { "id": 121, "idMal": 5114, "title": { "romaji": "Hagane no Renkinjutsushi: FULLMETAL ALCHEMIST", "english": "Fullmetal Alchemist: Brotherhood", "native": null }, "startDate": { "year": 2009 }, "format": "TV", "coverImage": { "large": "http://insecure.example/b.jpg" } },
    ] } } })
}

fn anilist_details() -> Value {
    json!({ "data": { "Media": {
        "id": 121, "idMal": 5114, "title": { "romaji": "Hagane no Renkinjutsushi", "english": "Fullmetal Alchemist: Brotherhood" },
        "description": "<p>Two brothers search for the <b>Philosopher's Stone</b>.<br>Story continues &amp; ends.</p>",
        "genres": ["Action", "Adventure"], "averageScore": 90,
        "coverImage": { "extraLarge": "https://s4.anilist.co/xl.jpg", "large": "https://s4.anilist.co/l.jpg" }, "bannerImage": "https://s4.anilist.co/banner.jpg",
        "trailer": { "id": "--PZ4xcrJL8", "site": "youtube" },
        "nextAiringEpisode": { "episode": 65, "airingAt": 1_800_000_000 },
        "characters": { "edges": [ { "role": "MAIN", "node": { "name": { "full": "Edward Elric" }, "image": { "large": "https://s4.anilist.co/e.jpg" } } }, { "role": "SUPPORTING", "node": { "name": { "full": "Alphonse Elric" }, "image": { "large": null } } } ] },
        "recommendations": { "nodes": [ { "mediaRecommendation": { "id": 1, "title": { "romaji": "Cowboy Bebop", "english": "Cowboy Bebop" }, "coverImage": { "large": "https://s4.anilist.co/cb.jpg" } } }, { "mediaRecommendation": null } ] },
    } } })
}

fn kitsu_search() -> Value {
    json!({ "data": [ { "id": "3936", "type": "anime", "attributes": { "canonicalTitle": "Fullmetal Alchemist: Brotherhood", "titles": { "en": "Fullmetal Alchemist: Brotherhood", "ja_jp": "鋼の錬金術師" }, "startDate": "2009-04-05", "subtype": "TV", "posterImage": { "small": "https://media.kitsu.io/s.jpg", "original": "https://media.kitsu.io/o.jpg" } } } ] })
}

fn kitsu_details() -> Value {
    json!({ "data": { "id": "3936", "type": "anime", "attributes": { "synopsis": "A Kitsu synopsis.", "posterImage": { "large": "https://media.kitsu.io/l.jpg" }, "coverImage": { "original": "https://media.kitsu.io/cover.jpg" }, "youtubeVideoId": "abc_DEF-123", "averageRating": "84.2" } } })
}

fn jikan(page: u32) -> Value {
    if page == 1 {
        json!({ "pagination": { "has_next_page": true }, "data": [ { "mal_id": 1, "filler": false }, { "mal_id": 2, "filler": true }, { "mal_id": 3, "filler": false } ] })
    } else {
        json!({ "pagination": { "has_next_page": false }, "data": [ { "mal_id": 4, "filler": true } ] })
    }
}

fn answers(request: &Request) -> anyhow::Result<Value> {
    match request {
        Request::Post(_, body) if body["query"].as_str().is_some_and(|q| q.contains("Page(")) => Ok(anilist_search()),
        Request::Post(..) => Ok(anilist_details()),
        Request::Get(url) if url.contains("kitsu.io/api/edge/anime?") => Ok(kitsu_search()),
        Request::Get(url) if url.contains("kitsu.io/api/edge/anime/") => Ok(kitsu_details()),
        Request::Get(url) if url.contains("page=2") => Ok(jikan(2)),
        Request::Get(_) => Ok(jikan(1)),
    }
}

fn enabled(app: &App) {
    app.store.update_settings(json!({ "metadata": { "enabled": true } }), Some("global"), None).unwrap();
}

fn details(sync: &[(&str, &str)], plot: Option<&str>) -> MediaDetails {
    let mut episodes = Vec::new();
    for n in 1..=4 {
        episodes.push(Episode { name: format!("Episode {n}"), data: format!("{{\"opaque\":\"ep{n}\"}}"), season: Some(1), episode: Some(n), id: Some(format!("ep-{n}")), ..Default::default() });
    }
    serde_json::from_value(json!({
        "item": { "providerId": "p:Source", "name": "Fullmetal Alchemist: Brotherhood", "url": "https://source.example/fmab", "posterUrl": "https://source.example/poster.jpg", "year": 2009 },
        "plot": plot,
        "episodes": episodes,
        "syncData": sync.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    }))
    .unwrap()
}

fn app(dir: &std::path::Path) -> Arc<App> {
    App::open(dir.join("data"), dir.join("res")).unwrap()
}

#[test]
fn ids_the_worker_supplies_are_read_and_bad_ones_are_dropped() {
    // The shape the worker emits: plain numbers for AniList/MAL/Kitsu, packed JSON under simkl.
    let supplied: BTreeMap<String, String> = [("mal", "5114"), ("anilist", "121"), ("kitsu", "3936"), ("simkl", r#"{"Imdb":"tt1355642","Tmdb":"31911"}"#), ("", "9")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let ids = external_ids(&supplied);
    assert_eq!(ids.get("anilist").map(String::as_str), Some("121"));
    assert_eq!(ids.get("imdb").map(String::as_str), Some("tt1355642"));
    assert_eq!(ids.len(), 5, "{ids:?}");
}

#[test]
fn remakes_with_the_same_name_are_suggestions_with_cautions_and_never_a_link() {
    let candidates = parse_anilist_search(&anilist_search());
    assert_eq!(candidates.len(), 2);
    assert!(candidates[1].poster_url.is_none(), "a non-HTTPS image address is dropped");
    let ranked = rank_candidates("Fullmetal Alchemist: Brotherhood", Some(2009), candidates.clone());
    assert_eq!(ranked[0].external_id, "121", "the same title and year ranks first");
    let remake = ranked.iter().find(|c| c.external_id == "25").unwrap();
    assert!(remake.warnings.iter().any(|w| w.contains("Year differs (2003 here, 2009 in your source)")), "{:?}", remake.warnings);
    // Even a perfect title and year match carries no "selected" flag: it is only an ordering.
    let exact = rank_candidates("Fullmetal Alchemist", Some(2003), candidates);
    assert_eq!(exact[0].external_id, "25");
    assert_eq!(title_similarity("Fullmetal Alchemist", "FULLMETAL  alchemist!"), 100);
    assert!(title_similarity("Naruto", "Bleach") < 40);
}

#[tokio::test]
async fn nothing_is_linked_or_fetched_without_an_id_even_when_a_title_matches_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    enabled(&app);
    let fake = Fake::new(answers);
    app.set_metadata_transport(fake.clone());
    let d = details(&[], Some("Provider plot."));
    // Opening a title with no known ids makes no network request at all.
    let result = app.metadata_for("m1", &d, "en").await.unwrap();
    assert!(result.links.is_empty());
    assert_eq!(fake.count(), 0);
    assert_eq!(result.view.synopsis.as_deref(), Some("Provider plot."));
    // Searching returns suggestions, and links nothing.
    let (found, problems) = app.metadata_search("Fullmetal Alchemist: Brotherhood", Some(2009)).await.unwrap();
    assert!(problems.is_empty() && found.len() >= 2);
    assert!(app.metadata_links("m1", &d.sync_data).unwrap().is_empty(), "a search result is never a link");
    assert!(app.store.list::<Value>("metadata_link").unwrap().is_empty());
}

#[tokio::test]
async fn metadata_is_off_by_default_and_asks_for_nothing_until_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let fake = Fake::new(answers);
    app.set_metadata_transport(fake.clone());
    let d = details(&[("anilist", "121")], None);
    let result = app.metadata_for("m1", &d, "en").await.unwrap();
    assert_eq!(fake.count(), 0, "no request before the person opts in");
    assert_eq!(result.links.len(), 1, "the supplied id is still known");
    assert!(app.metadata_search("x", None).await.unwrap_err().to_string().contains("turned off"));
}

#[tokio::test]
async fn enrichment_fills_gaps_with_provenance_and_never_touches_provider_data() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    enabled(&app);
    let fake = Fake::new(answers);
    app.set_metadata_transport(fake.clone());
    let d = details(&[("anilist", "121"), ("mal", "5114")], None);
    let before = serde_json::to_value(&d).unwrap();
    let result = app.metadata_for("m1", &d, "en").await.unwrap();
    // The provider's own record is exactly as it was, including opaque episode data.
    assert_eq!(serde_json::to_value(&d).unwrap(), before);
    assert_eq!(d.episodes[2].data, "{\"opaque\":\"ep3\"}");

    let view = result.view;
    assert_eq!(view.poster_url.as_deref(), Some("https://source.example/poster.jpg"), "the provider's poster wins by default");
    assert_eq!(view.provenance["poster"], "provider");
    assert_eq!(view.synopsis.as_deref(), Some("Two brothers search for the Philosopher's Stone . Story continues & ends."), "markup stripped, filled because the provider has none");
    assert_eq!(view.provenance["synopsis"], "anilist");
    assert_eq!(view.genres, ["Action", "Adventure"]);
    assert_eq!(view.score, Some(90.0));
    assert_eq!(view.trailer_url.as_deref(), Some("https://www.youtube.com/watch?v=--PZ4xcrJL8"));
    assert_eq!(view.cast.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Edward Elric", "Alphonse Elric"]);
    assert!(view.cast[1].image_url.is_none());
    assert_eq!(view.recommendations.len(), 1, "an empty recommendation entry is skipped");
    assert_eq!((view.next_airing_episode, view.next_airing_at_ms), (Some(65), Some(1_800_000_000_000)));
    // Filler comes from Jikan by MAL id, across pages, and is attached to the app's episode ids.
    assert_eq!(view.filler_episode_ids, ["ep-2", "ep-4"]);
    assert_eq!(view.provenance["filler"], "anilist");

    // With the preference on, the metadata poster replaces the provider's in the view only.
    app.store.update_settings(json!({ "metadata": { "preferPoster": true } }), Some("global"), None).unwrap();
    let preferred = app.metadata_for("m1", &d, "en").await.unwrap().view;
    assert_eq!(preferred.poster_url.as_deref(), Some("https://s4.anilist.co/xl.jpg"));
    assert_eq!(preferred.provenance["poster"], "anilist");
    assert_eq!(serde_json::to_value(&d).unwrap(), before);
}

#[tokio::test]
async fn provider_values_win_over_metadata_and_multi_season_filler_is_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    enabled(&app);
    app.set_metadata_transport(Fake::new(answers));
    let mut d = details(&[("anilist", "121"), ("mal", "5114")], Some("The provider's own synopsis."));
    d.tags = vec!["Provider tag".into()];
    d.actors = serde_json::from_value(json!([{ "actor": { "name": "Provider actor" } }])).unwrap();
    d.item.rating = Some(7.0);
    d.background_poster_url = Some("https://source.example/bg.jpg".into());
    for (i, ep) in d.episodes.iter_mut().enumerate() {
        ep.season = Some(if i < 2 { 1 } else { 2 });
    }
    let view = app.metadata_for("m1", &d, "en").await.unwrap().view;
    assert_eq!(view.synopsis.as_deref(), Some("The provider's own synopsis."));
    assert!(view.genres.is_empty() && view.cast.is_empty() && view.score.is_none() && view.backdrop_url.is_none(), "provider data present, so metadata adds none of it");
    assert!(view.filler_episode_ids.is_empty());
    assert!(view.notes.iter().any(|n| n.contains("several seasons")), "{:?}", view.notes);
}

#[tokio::test]
async fn a_chosen_match_survives_restart_overrides_the_provider_and_can_be_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let supplied: BTreeMap<String, String> = [("anilist".to_string(), "25".to_string())].into();
    {
        let app = app(dir.path());
        assert_eq!(app.metadata_links("m1", &supplied).unwrap()[0].source, MatchSource::Provider);
        app.set_metadata_link("m1", "anilist", "121", Some("Fullmetal Alchemist: Brotherhood".into()), Some(2009)).unwrap();
        assert!(app.set_metadata_link("m1", "anilist", "abc", None, None).is_err());
        assert!(app.set_metadata_link("m1", "bogus", "1", None, None).is_err());
        assert!(app.set_metadata_link("m1", "imdb", "12345", None, None).is_err(), "imdb ids start with tt");
    }
    let app = app(dir.path());
    let links = app.metadata_links("m1", &supplied).unwrap();
    assert_eq!((links[0].external_id.as_str(), links[0].source, links[0].year), ("121", MatchSource::User, Some(2009)), "the person's choice overrides the supplied id after a restart");
    // Another title is not affected.
    assert_eq!(app.metadata_links("m2", &supplied).unwrap()[0].external_id, "25");
    // Rejecting the supplied id keeps it away on the next visit, until the person chooses it again.
    app.remove_metadata_link("m1", "anilist", &supplied).unwrap();
    assert!(app.metadata_links("m1", &supplied).unwrap().is_empty());
    app.set_metadata_link("m1", "anilist", "25", None, None).unwrap();
    assert_eq!(app.metadata_links("m1", &supplied).unwrap()[0].external_id, "25");
}

#[tokio::test]
async fn an_outage_leaves_the_provider_data_playable_and_old_data_is_marked_stale() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    enabled(&app);
    let down = Arc::new(Mutex::new(false));
    let flag = down.clone();
    let fake = Fake::new(move |request| if *flag.lock().unwrap() { Err(anyhow::anyhow!("Could not reach the metadata service")) } else { answers(request) });
    app.set_metadata_transport(fake.clone());
    let d = details(&[("anilist", "121")], Some("Provider plot."));
    let ok = app.metadata_for("m1", &d, "en").await.unwrap();
    assert!(ok.errors.is_empty() && !ok.stale);
    let requests = fake.count();

    // A fresh cache answers without asking again.
    app.metadata_for("m1", &d, "en").await.unwrap();
    assert_eq!(fake.count(), requests, "cached for a week");
    // A different language is a different cache entry.
    app.metadata_for("m1", &d, "de").await.unwrap();
    assert!(fake.count() > requests);

    // Expire the cache and take the service down: the old data is shown, marked as out of date.
    let mut cached: Enrichment = app.store.get("metadata_cache", "anilist::121::en").unwrap().unwrap();
    cached.fetched_at_ms = 1;
    app.store.put("metadata_cache", "anilist::121::en", &cached).unwrap();
    *down.lock().unwrap() = true;
    let stale = app.metadata_for("m1", &d, "en").await.unwrap();
    assert!(stale.stale && stale.errors.len() == 1 && stale.view.synopsis.is_some());
    // With nothing cached at all, the provider's data is simply all there is.
    let fresh = app.metadata_for("other", &details(&[("anilist", "999")], Some("Only the provider's plot.")), "en").await.unwrap();
    assert_eq!(fresh.view.synopsis.as_deref(), Some("Only the provider's plot."));
    assert!(fresh.view.cast.is_empty() && !fresh.errors.is_empty());
    assert!(app.diagnostics.events().iter().any(|e| e.category == "metadata"));
}

#[test]
fn kitsu_and_jikan_responses_are_read_conservatively() {
    let found = parse_kitsu_search(&kitsu_search());
    assert_eq!((found[0].external_id.as_str(), found[0].year, found[0].title.as_str()), ("3936", Some(2009), "Fullmetal Alchemist: Brotherhood"));
    let e = parse_kitsu_details(&kitsu_details(), "en", 5).unwrap();
    assert_eq!((e.score, e.trailer_url.as_deref()), (Some(84.2), Some("https://www.youtube.com/watch?v=abc_DEF-123")));
    assert_eq!(e.poster_url.as_deref(), Some("https://media.kitsu.io/l.jpg"));
    // A hostile video id or image address is never turned into a link.
    let hostile = json!({ "data": { "id": "1", "attributes": { "youtubeVideoId": "x&redirect=http://evil", "posterImage": { "large": "javascript:alert(1)" } } } });
    let e = parse_kitsu_details(&hostile, "en", 5).unwrap();
    assert!(e.trailer_url.is_none() && e.poster_url.is_none());
    let (fillers, more) = parse_jikan_episodes(&jikan(1));
    assert_eq!((fillers, more), (vec![2], true));
    assert!(parse_anilist_details(&json!({ "data": { "Media": null } }), "en", 1).is_err());
    assert_eq!(kitsu_search_url("a b&c=d"), "https://kitsu.io/api/edge/anime?filter%5Btext%5D=a%20b%26c%3Dd&page%5Blimit%5D=8");
}

#[tokio::test]
async fn the_network_layer_only_talks_to_known_services_over_https() {
    let transport = Reqwest::new().unwrap();
    for url in ["https://evil.example/steal", "http://kitsu.io/api/edge/anime", "https://kitsu.io.evil.example/x", "file:///etc/passwd"] {
        let error = transport.send(Request::Get(url.into())).await.unwrap_err().to_string();
        assert!(error.contains("known services over HTTPS") || error.contains("Invalid"), "{url}: {error}");
    }
}
