//! Android backup import (T25) with a synthetic fixture: no real user data, tokens or ids.
use cloudstream_core::{backup::*, backup_android::*, models::*, App};
use serde_json::{json, Value};

fn bookmark(id: i64, api: &str, url: &str, name: &str) -> String {
    json!({ "bookmarkedTime": 1_600_000_000_000i64, "id": id, "latestUpdatedTime": 1_700_000_000_000i64, "name": name, "url": url,
            "apiName": api, "type": "Movie", "posterUrl": "https://img.example/p.jpg", "year": 2019 }).to_string()
}

fn fixture() -> Value {
    json!({
        "datastore": {
            "_String": {
                "data_store_helper/account": json!([{ "keyIndex": 1, "name": "Kids", "defaultImageIndex": 2, "lockPin": "1234" }]).to_string(),
                "REPOSITORIES_KEY": json!([{ "name": "My repo", "url": "https://example.test/repo.json" }]).to_string(),
                // default account, scoped
                "0/result_watch_state_data/101": bookmark(101, "Blender Open Movies", "https://www.blender.org/movie-a", "Movie A"),
                "0/result_watch_state_data/102": bookmark(102, "Blender Open Movies", "https://www.blender.org/movie-b", "Movie B"),
                "0/result_favorites_state_data/101": bookmark(101, "Blender Open Movies", "https://www.blender.org/movie-a", "Movie A"),
                "0/result_subscribed_state_data/102": json!({ "subscribedTime": 1, "lastSeenEpisodeCount": {}, "id": 102, "latestUpdatedTime": 2, "name": "Movie B",
                    "url": "https://www.blender.org/movie-b", "apiName": "Blender Open Movies", "type": "TvSeries" }).to_string(),
                // old, unscoped key: belongs to the default account
                "result_watch_state_data/103": bookmark(103, "Blender Open Movies", "https://www.blender.org/movie-c", "Movie C"),
                // unresolved and ambiguous sources
                "0/result_watch_state_data/104": bookmark(104, "Gone Source", "https://gone.example/x", "Lost Movie"),
                "0/result_watch_state_data/105": bookmark(105, "Twin", "https://second.example/t", "Twin Movie"),
                // the Kids account
                "1/result_watch_state_data/201": bookmark(201, "Blender Open Movies", "https://www.blender.org/kids", "Kids Movie"),
                // statuses (Android WatchType ids), one without a saved title
                "0/result_watch_state/101": "1",
                "0/result_watch_state/102": "0",
                "result_watch_state/103": "4",
                "0/result_watch_state/104": "2",
                "0/result_watch_state/105": "3",
                "1/result_watch_state/201": "4",
                "0/result_watch_state/777": "1",
                // continue watching: 10 minutes into a 20 minute episode
                "0/result_resume_watching_2/101": json!({ "name": "Movie A", "url": "https://www.blender.org/movie-a", "apiName": "Blender Open Movies",
                    "watchPos": { "position": 600_000, "duration": 1_200_000 }, "id": 555, "parentId": 101, "episode": 3, "season": 1, "isFromDownload": false }).to_string(),
                // per-episode markers keyed by Android hash ids cannot be mapped
                "0/video_pos_dur/9999": json!({ "position": 1, "duration": 2 }).to_string(),
                "0/video_watch_state/9999": "\"Watched\"",
                // credentials must never be read
                "0/anilist_token": "SUPER-SECRET-TOKEN"
            },
            "_Int": {}, "_Bool": {}, "_Float": {}, "_Long": {}, "_StringSet": {}
        },
        "settings": {
            "_Bool": { "autoplay_next_key": false, "start_paused_key": true, "compressor_enabled_key": true },
            "_Int": { "video_buffer_size_key": 128, "video_buffer_length_key": 30 },
            "_String": { "mal_token": "ANOTHER-SECRET", "nginx_user": "x" },
            "_Float": {}, "_Long": {}, "_StringSet": {}
        }
    })
}

fn providers() -> Vec<ProviderRef> {
    vec![
        ProviderRef { id: "pk:Blender Open Movies".into(), name: "Blender Open Movies".into(), main_url: "https://www.blender.org".into() },
        ProviderRef { id: "repo1:Twin".into(), name: "Twin".into(), main_url: "https://first.example".into() },
        ProviderRef { id: "repo2:Twin".into(), name: "Twin".into(), main_url: "https://third.example".into() },
    ]
}

fn plan(options: &ImportOptions) -> (AndroidParsed, ImportSet) {
    let parsed = parse_android(&fixture()).unwrap();
    let set = plan_android(&parsed, &providers(), &[("default".into(), "Default Profile".into())], options);
    (parsed, set)
}

#[test]
fn accounts_statuses_and_old_unscoped_keys_are_read_from_the_android_layout() {
    let parsed = parse_android(&fixture()).unwrap();
    assert_eq!(parsed.accounts.len(), 1);
    assert_eq!((parsed.accounts[0].index.as_str(), parsed.accounts[0].name.as_str(), parsed.accounts[0].has_pin), ("1", "Kids", true));
    let title = |url: &str| parsed.titles.iter().find(|t| t.url == url).unwrap_or_else(|| panic!("no title {url}"));
    // Same Android id 101 in two folders still produces one title; the unscoped key is account 0.
    let a = title("https://www.blender.org/movie-a");
    assert_eq!((a.account.as_str(), a.status, a.favorite, a.subscribed), ("0", Some(1), true, false));
    assert!(title("https://www.blender.org/movie-b").subscribed);
    assert_eq!(title("https://www.blender.org/movie-c").account, "0");
    assert_eq!(title("https://www.blender.org/movie-c").status, Some(4));
    assert_eq!(title("https://www.blender.org/kids").account, "1");
    let resume = a.resume.as_ref().unwrap();
    assert_eq!((resume.season, resume.episode, resume.position_seconds, resume.duration_seconds), (Some(1), Some(3), 600.0, Some(1200.0)));
    assert_eq!(parsed.unmappable_entries, 2, "per-episode hash-id markers are counted, not guessed");
    assert_eq!(parsed.orphan_status_entries, 1);
    assert_eq!(parsed.repositories, vec![BackupRepository { url: "https://example.test/repo.json".into(), name: "My repo".into() }]);
    let settings = Value::Object(parsed.settings_patch.clone());
    assert_eq!(settings, json!({ "playback": { "autoNext": false, "startPaused": true }, "player": { "audioCompressor": true, "bufferRamMb": 128, "bufferSeconds": 30 } }));
}

#[test]
fn every_android_watch_status_maps_to_the_matching_desktop_status() {
    let expected = [(0, WatchStatus::Watching), (1, WatchStatus::Completed), (2, WatchStatus::OnHold), (3, WatchStatus::Dropped), (4, WatchStatus::PlanToWatch), (5, WatchStatus::None), (99, WatchStatus::None)];
    for (android, desktop) in expected {
        let title = AndroidTitle { account: "0".into(), api_name: "A".into(), url: "https://a.example/x".into(), name: "T".into(), media_type: None, poster_url: None, year: None,
            status: Some(android), favorite: false, subscribed: false, in_library: true, saved_at_ms: 1, updated_at_ms: 2, resume: None, android_ids: vec![] };
        let (library, _) = title_records("default", &title, "pk:A");
        let item: LibraryItem = serde_json::from_value(library.unwrap().value).unwrap();
        assert_eq!(item.watch_status, desktop, "android status {android}");
    }
}

#[test]
fn titles_are_tied_to_sources_only_by_evidence() {
    let p = providers();
    assert_eq!(resolve_provider("blender open movies", "https://x/y", &p), Resolution::Resolved("pk:Blender Open Movies".into()), "names compare case-insensitively");
    assert_eq!(resolve_provider("Gone Source", "https://gone.example/x", &p), Resolution::Missing);
    // Two sources share a name: the title's website host decides, otherwise nothing is guessed.
    assert_eq!(resolve_provider("Twin", "https://www.first.example/t", &p), Resolution::Resolved("repo1:Twin".into()));
    assert_eq!(resolve_provider("Twin", "https://elsewhere.example/t", &p), Resolution::Ambiguous(vec!["repo1:Twin".into(), "repo2:Twin".into()]));
}

#[test]
fn the_plan_creates_profiles_resolves_titles_and_keeps_the_rest_without_secrets() {
    let (_, set) = plan(&ImportOptions::default());
    assert_eq!(set.new_profiles.len(), 1);
    assert_eq!(set.new_profiles[0].name, "Kids");
    let kids = placeholder("android:1");
    // Default account titles go to the default profile, Kids titles to the new profile.
    let ids: Vec<&str> = set.library.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(set.library.len(), 4, "A, B, C and the Kids movie: {ids:?}");
    assert_eq!(ids.iter().filter(|id| id.starts_with("default::")).count(), 3);
    assert_eq!(ids.iter().filter(|id| id.starts_with(&format!("{kids}::"))).count(), 1);
    // Two titles have no usable source: kept, with the reason visible.
    assert_eq!(set.unresolved_titles.len(), 2);
    let lost = set.unresolved_titles.iter().find(|t| t.name == "Lost Movie").unwrap();
    assert!(lost.reason.contains("No installed source is called 'Gone Source'"), "{}", lost.reason);
    let twin = set.unresolved_titles.iter().find(|t| t.name == "Twin Movie").unwrap();
    assert!(twin.reason.contains("2 installed sources"), "{}", twin.reason);
    // Progress uses the desktop episode identity built from season and episode.
    assert_eq!(set.progress.len(), 1);
    let media_id = compute_media_id("pk:Blender Open Movies", "https://www.blender.org/movie-a", None);
    let episode_id = compute_episode_id(&media_id, &Episode { season: Some(1), episode: Some(3), ..Default::default() }, None, None);
    assert_eq!(set.progress[0]["episode_id"], json!(episode_id));
    assert_eq!(set.progress[0]["position_seconds"], json!(600.0));
    assert_eq!(set.progress[0]["is_watched"], json!(0));
    // Notes explain what was not imported; no secret leaks into anything that gets written.
    let notes = set.notes.join("\n");
    assert!(notes.contains("PINs were not imported") && notes.contains("per-episode watch markers") && notes.contains("never imported"), "{notes}");
    let everything = format!("{:?} {:?} {:?} {:?}", set.library, set.unresolved, set.progress, set.settings);
    for secret in ["SUPER-SECRET-TOKEN", "ANOTHER-SECRET", "1234", "nginx_user"] {
        assert!(!everything.contains(secret), "{secret} must never be imported");
    }
    assert!(set.settings_is_patch);
}

#[test]
fn account_targets_and_name_collisions_are_respected() {
    let parsed = parse_android(&fixture()).unwrap();
    let mut options = ImportOptions::default();
    options.account_targets.insert("1".into(), "default".into());
    let set = plan_android(&parsed, &providers(), &[("default".into(), "Default Profile".into())], &options);
    assert!(set.new_profiles.is_empty());
    assert_eq!(set.library.iter().filter(|r| r.id.starts_with("default::")).count(), 4);
    let colliding = plan_android(&parsed, &providers(), &[("default".into(), "Default Profile".into()), ("p1".into(), "kids".into())], &ImportOptions::default());
    assert_eq!(colliding.new_profiles[0].name, "Kids (imported)");
}

#[tokio::test]
async fn importing_writes_everything_once_and_a_second_import_adds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();
    let options = ImportOptions::default();
    let (_, set) = plan(&options);

    let preview = app.store.run_import(&set, &options, true).unwrap();
    assert!(app.store.library(Some("default")).unwrap().is_empty(), "a preview writes nothing");
    let report = app.store.run_import(&set, &options, false).unwrap();
    assert_eq!(report.created_profiles.len(), 1);
    assert_eq!(preview.sections.iter().map(|s| (s.section.clone(), s.new)).collect::<Vec<_>>().len(), report.sections.len());

    let library = app.store.library(Some("default")).unwrap();
    assert_eq!(library.len(), 3);
    let by_name = |name: &str| library.iter().find(|l| l.item.name == name).unwrap();
    assert_eq!((by_name("Movie A").watch_status, by_name("Movie A").is_favorite), (WatchStatus::Completed, true));
    assert_eq!((by_name("Movie B").watch_status, by_name("Movie B").is_subscribed), (WatchStatus::Watching, true));
    assert_eq!(by_name("Movie C").watch_status, WatchStatus::PlanToWatch);
    assert_eq!(by_name("Movie A").item.provider_id, "pk:Blender Open Movies");
    assert_eq!(by_name("Movie A").item.url, "https://www.blender.org/movie-a", "provider URLs are copied byte for byte");
    let kids = app.store.list_profiles().unwrap().into_iter().find(|p| p.name == "Kids").unwrap();
    assert!(!kids.has_pin);
    let kids_library = app.store.library(Some(&kids.id)).unwrap();
    assert_eq!(kids_library.len(), 1);
    assert_eq!(kids_library[0].profile_id.as_deref(), Some(kids.id.as_str()));
    let progress = app.store.get_media_watch_progress("default", &compute_media_id("pk:Blender Open Movies", "https://www.blender.org/movie-a", None)).unwrap();
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].position_seconds, 600.0);
    assert_eq!(app.store.list::<Value>("unresolved_import").unwrap().len(), 2, "unmatched titles stay reviewable");
    let settings = app.store.get_settings().unwrap();
    assert_eq!((settings.global.playback.auto_next, settings.global.playback.start_paused, settings.global.player.buffer_ram_mb, settings.global.player.audio_compressor), (false, true, 128, true));

    // Re-import: the second plan creates a second "Kids (imported)" profile only if asked to
    // create new profiles again; mapping the account to the profile made earlier adds nothing.
    let mut again = ImportOptions::default();
    again.account_targets.insert("1".into(), kids.id.clone());
    let (_, second) = plan(&again);
    let report = app.store.run_import(&second, &again, false).unwrap();
    // Repository references are never written by an import (adding one needs the network); they
    // are reported so the user can add them, so that section is expected to stay "new".
    assert!(report.sections.iter().filter(|s| s.section != "repositories").all(|s| s.new == 0 && s.replaced == 0), "{:?}", report.sections);
    assert_eq!(report.repositories_to_add, vec![BackupRepository { url: "https://example.test/repo.json".into(), name: "My repo".into() }]);
    assert_eq!(app.store.library(Some("default")).unwrap().len(), 3);
    assert_eq!(app.store.list_profiles().unwrap().len(), 2, "no second Kids profile");
}

#[tokio::test]
async fn kept_titles_join_the_library_once_a_matching_source_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();
    let options = ImportOptions::default();
    let (_, set) = plan(&options);
    app.store.run_import(&set, &options, false).unwrap();
    let kept: Vec<Value> = app.store.list("unresolved_import").unwrap();
    let lost = kept.iter().find(|v| v["name"] == "Lost Movie").unwrap();
    assert_eq!(lost["status"], json!(2));
    // Not yet installed: still unresolved.
    assert!(resolve_kept("default", lost, &providers()).is_none());
    // The source arrives.
    let mut later = providers();
    later.push(ProviderRef { id: "newrepo:Gone Source".into(), name: "Gone Source".into(), main_url: "https://gone.example".into() });
    let (library, progress) = resolve_kept("default", lost, &later).unwrap();
    let library = library.unwrap();
    let entry: LibraryItem = serde_json::from_value(library.value).unwrap();
    assert_eq!((entry.watch_status, entry.item.provider_id.as_str()), (WatchStatus::OnHold, "newrepo:Gone Source"));
    assert!(progress.is_none());
}

#[test]
fn files_that_are_not_android_backups_are_rejected_with_a_reason() {
    assert!(parse_android(&json!({ "datastore": 5, "settings": {} })).unwrap_err().to_string().contains("must be objects"));
    assert!(parse_android(&json!({ "settings": {} })).unwrap_err().to_string().contains("no datastore"));
    let empty = parse_android(&json!({ "datastore": {}, "settings": {} })).unwrap();
    assert!(empty.titles.is_empty() && empty.accounts.is_empty());
}
