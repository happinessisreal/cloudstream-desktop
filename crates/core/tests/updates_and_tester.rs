use cloudstream_core::{models::*, App};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};
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
async fn check_for_updates_and_bulk_update_mixed_batch_preserves_old_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let sample_jar_bytes = std::fs::read(resources.join("worker/desktop-sample.jar")).unwrap();

    let repo_url = "https://example.com/repo.json";

    // Create 2 installed plugins:
    // Plugin 1: v1 (repo will have v2) -> update will succeed
    // Plugin 2: v1 (repo will have v2) -> update will fail (server 500)
    let p1_entry = PluginEntry {
        url: "https://example.com/p1.jar".into(),
        name: "Blender Open Movies".into(),
        internal_name: "desktop-sample".into(),
        version: 1,
        status: 1,
        api_version: 1,
        authors: vec!["Cloudstream Desktop contributors".into()],
        description: Some("Blender sample plugin".into()),
        language: Some("en".into()),
        icon_url: None,
        repository_url: Some(repo_url.into()),
        tv_types: Some(vec!["Movie".into()]),
        file_size: None,
        file_hash: None,
        jar_url: Some("https://example.com/p1.jar".into()),
        jar_hash: None,
        jar_file_size: None,
        source_url: None,
        extra: std::collections::BTreeMap::new(),
    };

    let p2_entry = PluginEntry {
        url: "https://example.com/p2.jar".into(),
        name: "Plugin Two".into(),
        internal_name: "plugin_two".into(),
        version: 1,
        status: 1,
        api_version: 1,
        authors: vec!["Author 2".into()],
        description: Some("Second plugin".into()),
        language: Some("en".into()),
        icon_url: None,
        repository_url: Some(repo_url.into()),
        tv_types: Some(vec!["Movie".into()]),
        file_size: None,
        file_hash: None,
        jar_url: Some("https://example.com/p2.jar".into()),
        jar_hash: None,
        jar_file_size: None,
        source_url: None,
        extra: std::collections::BTreeMap::new(),
    };

    let p2_jar = make_test_jar(&[
        ("manifest.json", br#"{"name":"Plugin Two","internalName":"plugin_two","version":1}"#),
        ("p2/Test.class", b"bytecode"),
    ]);

    let inst1 = app.repositories.prepare_bytes(
        repo_url,
        p1_entry.clone(),
        &sample_jar_bytes,
        None,
        RuntimeKind::Jvm,
    ).unwrap();
    let inst2 = app.repositories.prepare_bytes(
        repo_url,
        p2_entry.clone(),
        &p2_jar,
        None,
        RuntimeKind::Jvm,
    ).unwrap();

    app.store.put("plugin", &inst1.id, &inst1).unwrap();
    app.store.put("plugin", &inst2.id, &inst2).unwrap();

    // Now set up repository in store where both have version 2 available!
    let mut p1_v2 = p1_entry.clone();
    p1_v2.version = 2;
    let mut p2_v2 = p2_entry.clone();
    p2_v2.version = 2;

    let repo = Repository {
        url: repo_url.into(),
        manifest: RepositoryManifest {
            name: "Main Repo".into(),
            description: None,
            icon_url: None,
            manifest_version: 1,
            plugin_lists: vec![],
        },
        plugins: vec![p1_v2, p2_v2],
        custom_name: None,
        updated_at: None,
        extra: std::collections::BTreeMap::new(),
    };
    app.store.put("repository", repo_url, &repo).unwrap();

    // 1. Check for updates
    let updates = app.check_for_updates().await.unwrap();
    assert_eq!(updates.len(), 2);
    assert!(updates.iter().all(|u| u.has_update));
    assert!(updates.iter().all(|u| u.available_version == 2));
    assert!(updates.iter().all(|u| u.installed_version == 1));

    // 2. Mock network download server for updates:
    // P1 download succeeds with valid sample JAR v2.
    // P2 download fails with 500 server error.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Update the URLs in the repo entry to point to our test listener
    let mut updated_repo = repo.clone();
    updated_repo.plugins[0].url = format!("http://{addr}/p1.jar");
    updated_repo.plugins[0].jar_url = Some(format!("http://{addr}/p1.jar"));
    updated_repo.plugins[1].url = format!("http://{addr}/p2.jar");
    updated_repo.plugins[1].jar_url = Some(format!("http://{addr}/p2.jar"));
    app.store.put("repository", repo_url, &updated_repo).unwrap();

    let p1_v2_bytes = sample_jar_bytes.clone();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = vec![0u8; 2048];
            let n = stream.read(&mut req_buf).await.unwrap();
            let req = String::from_utf8_lossy(&req_buf[..n]);

            if req.contains("GET /p1.jar") {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/java-archive\r\nConnection: close\r\n\r\n",
                    p1_v2_bytes.len()
                );
                stream.write_all(resp.as_bytes()).await.unwrap();
                stream.write_all(&p1_v2_bytes).await.unwrap();
            } else {
                // P2 fails with server error
                let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(resp.as_bytes()).await.unwrap();
            }
        }
    });

    // 3. Run bulk update
    let results = app.update_all_extensions().await.unwrap();
    assert_eq!(results.len(), 2);

    let r1 = results.iter().find(|r| r.name == "Blender Open Movies").unwrap();
    let r2 = results.iter().find(|r| r.name == "Plugin Two").unwrap();

    // Plugin 1 succeeded and now has version 2
    assert!(r1.success);
    assert_eq!(r1.updated_version, Some(2));

    // Plugin 2 failed, error reported
    assert!(!r2.success);
    assert!(r2.error.is_some());

    // CRITICAL: Plugin 2 in the database is still version 1 (old version kept!)
    let installed_after = app.store.installed().unwrap();
    let p2_after = installed_after.iter().find(|p| p.entry.internal_name == "plugin_two").unwrap();
    assert_eq!(p2_after.entry.version, 1);

    server.await.unwrap();
}

#[test]
fn interrupted_update_recovery_cleans_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let ext_dir = dir.path().join("extensions").join("plugin_123");
    std::fs::create_dir_all(&ext_dir).unwrap();

    let tmp_file1 = ext_dir.join("temp_download.tmp");
    let tmp_file2 = ext_dir.join("partial_hash.tmp.123");
    let valid_file = ext_dir.join("valid_hash.jar");

    std::fs::write(&tmp_file1, b"corrupted partial download").unwrap();
    std::fs::write(&tmp_file2, b"leftover temp file").unwrap();
    std::fs::write(&valid_file, b"intact package").unwrap();

    assert!(tmp_file1.is_file());
    assert!(tmp_file2.is_file());
    assert!(valid_file.is_file());

    // Run cleanup
    cloudstream_core::clean_interrupted_updates(&dir.path().join("extensions"));

    // Tmp files are deleted, valid file is preserved
    assert!(!tmp_file1.is_file());
    assert!(!tmp_file2.is_file());
    assert!(valid_file.is_file());
}

#[tokio::test]
async fn provider_self_tester_runs_stages_and_reports_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources).unwrap();

    // Install the compatibility sample
    let _installed = app.install_sample().await.unwrap();
    let providers = app.providers().await.unwrap();
    assert!(!providers.is_empty());

    let provider = &providers[0];

    // Run tester
    let report = app.test_provider(&provider.id, None).await.unwrap();
    assert_eq!(report.provider_id, provider.id);
    assert_eq!(report.provider_name, provider.name);
    assert!(report.passed);

    // All 4 stages must be recorded
    let stage_names: Vec<&str> = report.stages.iter().map(|s| s.stage.as_str()).collect();
    assert_eq!(stage_names, vec!["home", "search", "details", "links"]);

    for stage in &report.stages {
        assert_eq!(stage.status, "passed");
        assert!(stage.message.is_some());
    }
}
