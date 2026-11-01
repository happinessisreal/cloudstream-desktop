use cloudstream_core::{models::*, repository::*, store::Store};
use serde_json::json;
use std::io::{Cursor, Write};

fn package(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn entry() -> PluginEntry {
    serde_json::from_value(json!({"url":"https://example.com/test.cs3","name":"Test","internalName":"../../untrusted/name","version":1,"authors":[],"status":1})).unwrap()
}
#[test]
fn android_packages_are_never_classified_as_jvm() {
    let bytes = package(&[
        ("classes.dex", b"dex\n035\0"),
        (
            "manifest.json",
            br#"{"pluginClassName":"example.Plugin","requiresResources":true}"#,
        ),
    ]);
    let info = inspect_package(&bytes).unwrap();
    assert_eq!(info.runtime, RuntimeKind::Android);
    assert_eq!(info.plugin_class.as_deref(), Some("example.Plugin"));
}
#[test]
fn cross_platform_jar_does_not_need_manifest_json() {
    assert_eq!(
        inspect_package(&package(&[("example/Plugin.class", b"bytecode")]))
            .unwrap()
            .runtime,
        RuntimeKind::Jvm
    );
}
#[test]
fn invalid_packages_and_mismatched_hashes_are_rejected() {
    assert!(inspect_package(b"not a zip").is_err());
    assert!(inspect_package(&package(&[("readme", b"nothing")])).is_err());
    assert!(verify_hash(b"changed", Some(&format!("sha256-{}", digest(b"original")))).is_err());
    assert!(verify_hash(b"package", Some("md5-nope")).is_err());
    assert!(!verify_hash(b"package", None).unwrap());
}
#[test]
fn extension_identity_is_repository_scoped_and_path_safe() {
    let a = plugin_id("https://a.example/repo.json", "../../x");
    let b = plugin_id("https://b.example/repo.json", "../../x");
    assert_ne!(a, b);
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}
#[test]
fn package_format_mismatch_and_corrupt_updates_leave_installed_record_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("app.db")).unwrap();
    let manager = RepositoryManager::new(dir.path().join("extensions")).unwrap();
    let bytes = package(&[("example/Plugin.class", b"class")]);
    let hash = format!("sha256-{}", digest(&bytes));
    let plugin = manager
        .prepare_bytes(
            "https://repo.example",
            entry(),
            &bytes,
            Some(&hash),
            RuntimeKind::Jvm,
        )
        .unwrap();
    store.put("plugin", &plugin.id, &plugin).unwrap();
    assert!(manager
        .prepare_bytes(
            "https://repo.example",
            entry(),
            b"broken",
            Some(&hash),
            RuntimeKind::Jvm
        )
        .is_err());
    assert!(manager
        .prepare_bytes(
            "https://repo.example",
            entry(),
            &bytes,
            Some(&hash),
            RuntimeKind::Android
        )
        .is_err());
    assert_eq!(store.installed().unwrap()[0].sha256, plugin.sha256);
    assert_eq!(std::fs::read(&plugin.path).unwrap(), bytes);
    assert!(std::path::Path::new(&plugin.path).starts_with(manager.directory));
}
#[test]
fn database_preserves_opaque_episode_data_and_bookmarks_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let value = json!({"episodeData":"{\"headers\":{\"x\":\"a\\nb\"},\"id\":\"/?=&\"}","title":"A title 日本語"});
    {
        let store = Store::open(&path).unwrap();
        store.put("opaque", "one", &value).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.list::<serde_json::Value>("opaque").unwrap(),
        vec![value]
    );
    store.remove("opaque", "one").unwrap();
    assert!(store
        .list::<serde_json::Value>("opaque")
        .unwrap()
        .is_empty());
}
#[test]
fn repository_links_are_normalized_without_accepting_other_schemes() {
    assert_eq!(
        normalize_repository("cloudstreamrepo://example.com/repo.json").unwrap(),
        "https://example.com/repo.json"
    );
    assert_eq!(
        normalize_repository("https://cs.repo/?https://example.com/repo.json").unwrap(),
        "https://example.com/repo.json"
    );
    assert!(normalize_repository("file:///etc/passwd").is_err());
    assert!(network_url("https://user:password@example.com").is_err());
}
#[tokio::test]
async fn existing_repository_manifests_and_jar_metadata_are_supported() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let n = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            let body = if request.starts_with("GET /repo.json ") {
                json!({"name":"Existing repo","manifestVersion":1,"pluginLists":["plugins.json"]})
                    .to_string()
            } else {
                json!([{"url":"test.cs3","name":"Test","internalName":"test","version":7,"authors":["author"],"apiVersion":1,"status":1,"jarUrl":"test.jar","jarHash":"sha256-a","unknownFutureField":true}]).to_string()
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        }
    });
    let repo = fetch_repository(
        &reqwest::Client::new(),
        &format!("http://{address}/repo.json"),
    )
    .await
    .unwrap();
    assert_eq!(repo.plugins[0].version, 7);
    assert_eq!(
        repo.plugins[0].jar_url.as_deref(),
        Some(format!("http://{address}/test.jar").as_str())
    );
    assert_eq!(repo.plugins[0].url, format!("http://{address}/test.cs3"));
    server.await.unwrap();
}
