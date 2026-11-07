use cloudstream_core::{downloads::DestinationIdentity, hls::SelectionPolicy, hls_download::{check_folder, download_hls, read_manifest, remove_folder, ExternalSubtitle, HlsRequest, HlsResult}};
use std::{path::Path, sync::{Arc, Mutex, atomic::{AtomicU8, Ordering}}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::watch};

struct Server { address: std::net::SocketAddr, requests: Arc<Mutex<Vec<String>>>, mode: Arc<AtomicU8>, task: tokio::task::JoinHandle<()> }
impl Drop for Server { fn drop(&mut self) { self.task.abort(); } }
impl Server { fn url(&self, path: &str) -> String { format!("http://{}/{path}", self.address) } }
async fn server() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mode = Arc::new(AtomicU8::new(0));
    let server_mode = mode.clone();
    let log = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let log = log.clone();
            let mode = server_mode.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("").trim_start_matches('/').split('?').next().unwrap_or("");
                log.lock().unwrap().push(path.to_string());
                if path.contains("..") { return; }
                let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls").join(path);
                let Ok(mut data) = std::fs::read(file) else {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                if path.starts_with("ts/seg") {
                    match mode.load(Ordering::SeqCst) {
                        1 => { if let Some(last) = data.last_mut() { *last ^= 1; } },
                        2 => { let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await; return; },
                        _ => {}
                    }
                }
                let range = request.lines().find_map(|line| {
                    let line = line.to_ascii_lowercase();
                    let (a, b) = line.strip_prefix("range: bytes=")?.split_once('-')?;
                    Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                });
                let (status, start, end, extra) = if let Some((start, end)) = range {
                    if start > end || end >= data.len() { return; }
                    ("206 Partial Content", start, end + 1, format!("Content-Range: bytes {start}-{end}/{}\r\n", data.len()))
                } else { ("200 OK", 0, data.len(), String::new()) };
                let head = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", end - start);
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&data[start..end]).await;
            });
        }
    });
    Server { address, requests, mode, task }
}

async fn fetch(root: &Path, id: &str, url: &str) -> std::path::PathBuf {
    let (_tx, stop) = watch::channel(false);
    let request = HlsRequest { root, identity: DestinationIdentity::from_path(root).unwrap(), job_id: id, source_identity: "fixture", url, no_proxy: true, policy: SelectionPolicy::default(), subtitles: vec![], skipped_subtitles: 0 };
    let HlsResult::Completed { path, bytes } = download_hls(request, stop, |_| {}).await.unwrap() else { panic!("download did not finish") };
    assert!(bytes > 0);
    path
}

#[tokio::test]
async fn downloads_separate_audio_and_plays_from_local_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let server = server().await;
    let id = uuid::Uuid::new_v4().to_string();
    let path = fetch(dir.path(), &id, &server.url("fmp4/master.m3u8")).await;
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let manifest = read_manifest(dir.path(), identity, &id).unwrap().unwrap();
    assert!(manifest.audio.is_some());
    assert!(check_folder(dir.path(), identity, &id, true).unwrap().healthy());
    assert!(manifest.files.iter().all(|file| !file.path.contains("http")));
    drop(server);
    let output = std::process::Command::new("ffprobe").args(["-v", "error", "-protocol_whitelist", "file,crypto,data", "-show_entries", "stream=codec_type", "-of", "csv=p=0"]).arg(path.join("index.m3u8")).output().expect("ffprobe is required to verify local playback");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let streams = String::from_utf8_lossy(&output.stdout);
    assert!(streams.contains("video") && streams.contains("audio"), "{streams}");
}

#[tokio::test]
async fn supports_encrypted_segments_and_fragmented_mp4_ranges() {
    let server = server().await;
    for fixture in ["aes/media.m3u8", "rangemp4/media.m3u8"] {
        let dir = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = fetch(dir.path(), &id, &server.url(fixture)).await;
        let output = std::process::Command::new("ffprobe").args(["-v", "error", "-protocol_whitelist", "file,crypto,data", "-show_entries", "stream=codec_type", "-of", "csv=p=0"]).arg(path.join("index.m3u8")).output().unwrap();
        assert!(output.status.success(), "{fixture}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("video"));
        assert!(!std::fs::read_to_string(path.join("video/index.m3u8")).unwrap().contains("EXT-X-KEY"));
    }
    assert!(server.requests.lock().unwrap().iter().any(|p| p == "aes/key.bin"));
}

#[tokio::test]
async fn missing_and_skipped_subtitles_are_recorded_without_losing_video() {
    let dir = tempfile::tempdir().unwrap();
    let server = server().await;
    let id = uuid::Uuid::new_v4().to_string();
    let url = server.url("ts/media.m3u8");
    let (_tx, stop) = watch::channel(false);
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let request = HlsRequest { root: dir.path(), identity, job_id: &id, source_identity: "fixture", url: &url, no_proxy: true, policy: SelectionPolicy::default(), subtitles: vec![
        ExternalSubtitle { name: "English".into(), language: "en".into(), url: server.url("subs/provider.srt"), no_proxy: true },
        ExternalSubtitle { name: "Missing".into(), language: "de".into(), url: server.url("subs/missing.srt"), no_proxy: true },
    ], skipped_subtitles: 2 };
    assert!(matches!(download_hls(request, stop, |_| {}).await.unwrap(), HlsResult::Completed { .. }));
    let manifest = read_manifest(dir.path(), identity, &id).unwrap().unwrap();
    assert_eq!(manifest.external_subtitles.len(), 1);
    assert_eq!(manifest.missing_external_subtitles, 3);
}

#[tokio::test]
async fn replaced_track_directory_is_not_followed_when_checking_or_deleting() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let server = server().await;
    let id = uuid::Uuid::new_v4().to_string();
    let path = fetch(dir.path(), &id, &server.url("ts/media.m3u8")).await;
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    std::fs::rename(path.join("video"), outside.path().join("video")).unwrap();
    symlink(outside.path().join("video"), path.join("video")).unwrap();
    assert!(!check_folder(dir.path(), identity, &id, true).unwrap().healthy());
    let report = remove_folder(dir.path(), identity, &id, true).unwrap();
    assert!(!report.file_deleted);
    assert!(!report.notes.is_empty());
    assert!(outside.path().join("video/index.m3u8").exists());
    assert!(std::fs::read_dir(outside.path().join("video")).unwrap().count() > 1);
}

#[tokio::test]
async fn deletion_preserves_files_edited_after_download() {
    let dir = tempfile::tempdir().unwrap();
    let server = server().await;
    let id = uuid::Uuid::new_v4().to_string();
    let path = fetch(dir.path(), &id, &server.url("ts/media.m3u8")).await;
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let changed = path.join("video/seg-000000.ts");
    std::fs::write(&changed, b"user-edited data").unwrap();
    let report = remove_folder(dir.path(), identity, &id, true).unwrap();
    assert!(!report.file_deleted);
    assert!(report.notes.iter().any(|note| note.contains("changed")));
    assert_eq!(std::fs::read(changed).unwrap(), b"user-edited data");
}

#[tokio::test]
async fn resume_detects_missing_and_changed_segments_before_restarting_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let server = server().await;
    let id = uuid::Uuid::new_v4().to_string();
    let url = server.url("ts/media.m3u8");
    let identity = DestinationIdentity::from_path(dir.path()).unwrap();
    let request = || HlsRequest { root: dir.path(), identity, job_id: &id, source_identity: "fixture", url: &url, no_proxy: true, policy: SelectionPolicy::default(), subtitles: vec![], skipped_subtitles: 0 };
    let (pause, stop) = watch::channel(false);
    let result = download_hls(request(), stop, move |progress| {
        if progress.segments_done > 0 { pause.send_replace(true); }
    }).await.unwrap();
    assert!(matches!(result, HlsResult::Paused { bytes } if bytes > 0));

    server.mode.store(2, Ordering::SeqCst);
    let (_tx, stop) = watch::channel(false);
    let missing = download_hls(request(), stop, |_| {}).await.unwrap_err();
    assert!(missing.to_string().contains("no longer available"), "{missing}");

    server.mode.store(1, Ordering::SeqCst);
    let (_tx, stop) = watch::channel(false);
    let changed = download_hls(request(), stop, |_| {}).await.unwrap_err();
    assert!(changed.to_string().contains("has changed"), "{changed}");

    let path = fetch(dir.path(), &id, &url).await;
    for (index, file) in ["seg0.ts", "seg1.ts", "seg2.ts"].iter().enumerate() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/ts").join(file);
        let mut expected = std::fs::read(source).unwrap();
        *expected.last_mut().unwrap() ^= 1;
        assert_eq!(std::fs::read(path.join(format!("video/seg-{index:06}.ts"))).unwrap(), expected);
    }
}
