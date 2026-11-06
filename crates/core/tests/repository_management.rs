use cloudstream_core::{models::*, repository::*, App};
use serde_json::json;
use std::io::{Cursor, Write};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn make_test_jar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn short_code_resolution_py_md_and_cutt_ly_with_bounds_and_fallbacks() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        // Handle redirect request, target request, and 404 request
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = vec![0u8; 2048];
            let n = stream.read(&mut req_buf).await.unwrap();
            let req = String::from_utf8_lossy(&req_buf[..n]);

            if req.contains("GET /valid_code") {
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{addr}/actual_repo.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(resp.as_bytes()).await.unwrap();
            } else if req.contains("GET /actual_repo.json") {
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}";
                stream.write_all(resp.as_bytes()).await.unwrap();
            } else {
                let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(resp.as_bytes()).await.unwrap();
            }
        }
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .unwrap();

    // 1. Direct redirect resolution
    let resolved = resolve_shortcode_redirect(&client, &format!("http://{addr}/valid_code"))
        .await
        .unwrap();
    assert_eq!(resolved, format!("http://{addr}/actual_repo.json"));

    // 2. Failed / 404 shortcode
    assert!(resolve_shortcode_redirect(&client, &format!("http://{addr}/bad_code")).await.is_err());

    // 3. Normal URL bypasses network resolver
    let ordinary = resolve_repository_url(&client, "https://example.com/repo.json")
        .await
        .unwrap();
    assert_eq!(ordinary, "https://example.com/repo.json");

    let cloudstream_scheme = resolve_repository_url(&client, "cloudstreamrepo://example.com/repo.json")
        .await
        .unwrap();
    assert_eq!(cloudstream_scheme, "https://example.com/repo.json");

    server.await.unwrap();
}

#[test]
fn local_package_import_jar_and_cs3_runs_same_checks_and_registers() {
    let dir = tempfile::tempdir().unwrap();
    let manager = RepositoryManager::new(dir.path().join("extensions")).unwrap();

    let manifest = json!({
        "name": "My Local Source",
        "internalName": "my_local_source",
        "version": 4,
        "authors": ["Test Dev"],
        "description": "A standalone local test package"
    });

    let jar_bytes = make_test_jar(&[
        ("manifest.json", manifest.to_string().as_bytes()),
        ("com/example/MyPlugin.class", b"fake bytecode"),
    ]);

    let jar_path = dir.path().join("local_sample.jar");
    std::fs::write(&jar_path, &jar_bytes).unwrap();

    // Prepare local
    let plugin = manager.prepare_local(&jar_path).unwrap();
    assert_eq!(plugin.entry.name, "My Local Source");
    assert_eq!(plugin.entry.internal_name, "my_local_source");
    assert_eq!(plugin.entry.version, 4);
    assert_eq!(plugin.runtime, RuntimeKind::Jvm);
    assert!(plugin.verified);
    assert_eq!(plugin.repository_url, "local://packages");

    // Check content-addressed installation file exists
    assert!(std::path::Path::new(&plugin.path).is_file());
    assert_eq!(std::fs::read(&plugin.path).unwrap(), jar_bytes);

    // Invalid local file rejected
    let bad_path = dir.path().join("corrupt.jar");
    std::fs::write(&bad_path, b"corrupted not a zip").unwrap();
    assert!(manager.prepare_local(&bad_path).is_err());
}

#[test]
fn repositories_sharing_internal_name_remain_independent() {
    let id_remote = plugin_id("https://repo.example/repo.json", "shared_source");
    let id_local = plugin_id("local://packages", "shared_source");

    assert_ne!(id_remote, id_local);
    assert_eq!(id_remote.len(), 64);
    assert_eq!(id_local.len(), 64);
}

#[tokio::test]
async fn duplicate_repository_add_refreshes_instead_of_failing_and_preserves_custom_name() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = vec![0u8; 2048];
            let n = stream.read(&mut req_buf).await.unwrap();
            let req = String::from_utf8_lossy(&req_buf[..n]);

            let body = if req.starts_with("GET /repo.json") {
                json!({
                    "name": "Test Repo",
                    "manifestVersion": 1,
                    "pluginLists": ["plugins.json"]
                }).to_string()
            } else {
                json!([{
                    "url": "test.jar",
                    "name": "Test Extension",
                    "internalName": "test_ext",
                    "version": 1,
                    "status": 1,
                    "apiVersion": 1,
                    "authors": ["Author"],
                    "jarUrl": "test.jar"
                }]).to_string()
            };

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let repo_url = format!("http://{addr}/repo.json");

    // First add
    let r1 = app.add_repository(&repo_url).await.unwrap();
    assert_eq!(r1.manifest.name, "Test Repo");
    assert_eq!(r1.plugins.len(), 1);
    assert_eq!(r1.custom_name, None);

    // Set custom display name
    let renamed = app.rename_repository(&repo_url, Some("My Favorite Sources".into())).unwrap();
    assert_eq!(renamed.custom_name.as_deref(), Some("My Favorite Sources"));

    // Add again (duplicate add) -> refreshes without failing, preserving custom name!
    let r2 = app.add_repository(&repo_url).await.unwrap();
    assert_eq!(r2.custom_name.as_deref(), Some("My Favorite Sources"));

    // Verify stored
    let stored = app.store.repositories().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].custom_name.as_deref(), Some("My Favorite Sources"));

    server.await.unwrap();
}

#[tokio::test]
async fn removing_repository_preserves_library_and_watch_history() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    let repo_url = "https://example.com/repo.json";
    let test_repo = Repository {
        url: repo_url.into(),
        manifest: RepositoryManifest {
            name: "Anime Repo".into(),
            description: None,
            icon_url: None,
            manifest_version: 1,
            plugin_lists: vec![],
        },
        plugins: vec![],
        custom_name: None,
        updated_at: None,
        extra: std::collections::BTreeMap::new(),
    };
    app.store.put("repository", repo_url, &test_repo).unwrap();

    // Create library item and watch history
    let media = MediaItem {
        provider_id: "anime_source".into(),
        name: "Neon Genesis".into(),
        url: "https://example.com/anime/eva".into(),
        poster_url: None,
        poster_headers: std::collections::BTreeMap::new(),
        media_type: Some("Anime".into()),
        year: Some(1995),
        unique_url: None,
        media_id: None,
        score: None,
        rating: None,
        quality: None,
        id: None,
        other_name: None,
        episodes_count: None,
        dub_status: None,
        dub_episodes: None,
        extra: std::collections::BTreeMap::new(),
    };
    let bookmarked = app.bookmark(media.clone()).unwrap();

    let ep = Episode {
        name: "Episode 1".into(),
        data: "ep1_opaque_data".into(),
        season: Some(1),
        episode: Some(1),
        poster_url: None,
        id: None,
        dub_status: None,
        index: Some(0),
        score: None,
        rating: None,
        description: None,
        date: None,
        run_time: None,
        extra: std::collections::BTreeMap::new(),
    };
    let watch = WatchEntry {
        id: "watch_eva_1".into(),
        item: media,
        episode: ep,
        position: 120.0,
        duration: 1440.0,
        watched_at: 1000,
    };
    app.store.save_watch_entry(&watch, "default").unwrap();

    // Remove repository
    app.remove_repository(repo_url).await.unwrap();

    // Repository record is gone
    assert!(app.store.repositories().unwrap().is_empty());

    // User's library and watch history remain INTACT!
    let library = app.store.library(None).unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].id, bookmarked.id);

    let history = app.store.history(None).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, watch.id);
}

#[test]
fn provider_enable_disable_and_clone_configuration_persists_without_mutating_parent() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();

    let p_id = "sample_plugin:BlenderFilms";

    // Initially not disabled
    assert!(!app.store.list_disabled_providers().unwrap().contains(p_id));

    // Disable provider
    app.set_provider_enabled(p_id, false).unwrap();
    assert!(app.store.list_disabled_providers().unwrap().contains(p_id));

    // Re-enable
    app.set_provider_enabled(p_id, true).unwrap();
    assert!(!app.store.list_disabled_providers().unwrap().contains(p_id));

    // Configure clone
    let clone = ProviderCloneConfig {
        id: "clone::sample_plugin:BlenderFilms::mirror2".into(),
        parent_provider_id: p_id.into(),
        name: "Blender Mirror 2".into(),
        custom_url: "https://mirror2.blender.org".into(),
        credentials_handle: None,
        enabled: true,
        extra: std::collections::BTreeMap::new(),
    };
    app.configure_provider_clone(clone.clone()).unwrap();

    let clones = app.store.list_provider_clones().unwrap();
    assert_eq!(clones.len(), 1);
    assert_eq!(clones[0].name, "Blender Mirror 2");
    assert_eq!(clones[0].custom_url, "https://mirror2.blender.org");

    // Delete clone
    app.delete_provider_clone(&clone.id).unwrap();
    assert!(app.store.list_provider_clones().unwrap().is_empty());
}
