mod support;
use cloudstream_core::downloads::{transfer_direct, transfer_with_retry, DestinationIdentity, TransferResult};
use std::{path::Path, sync::{Arc, atomic::{AtomicU8, Ordering}}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::{watch, Mutex}};

struct Server { address: std::net::SocketAddr, mode: Arc<AtomicU8>, requests: Arc<Mutex<Vec<String>>>, body: Arc<Vec<u8>>, task: tokio::task::JoinHandle<()> }
impl Drop for Server { fn drop(&mut self) { self.task.abort(); } }
impl Server { fn url(&self) -> String { format!("http://{}/video.mp4", self.address) } }
async fn server() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = Arc::new((0..1_048_576).map(|i| ((i * 31 + 7) % 251) as u8).collect::<Vec<_>>());
    let data = body.clone();
    let mode = Arc::new(AtomicU8::new(0));
    let mode_task = mode.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let logs = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = data.clone(); let mode = mode_task.clone(); let logs = logs.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                logs.lock().await.push(request.clone());
                let range = request.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").and_then(|v| v.split('-').next()?.parse::<usize>().ok()));
                let current = mode.load(Ordering::SeqCst);
                let etag = if current == 2 { "\"v2\"" } else { "\"v1\"" };
                let mut start = range.unwrap_or(0);
                if current == 1 || range.is_none() { start = 0; }
                if range.is_some() && (start >= body.len() || current == 3) {
                    let head = format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nETag: {etag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(head.as_bytes()).await; return;
                }
                let partial = range.is_some() && current != 1;
                let status = if partial { "206 Partial Content" } else { "200 OK" };
                let cr = if partial { let reported = if current == 6 { start + 1 } else { start }; format!("Content-Range: bytes {reported}-{}/{}\r\n", body.len()-1, body.len()) } else { String::new() };
                let validator = if current == 4 { String::new() } else if current == 7 { "Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\nDate: Wed, 21 Oct 2015 07:30:00 GMT\r\n".into() } else { format!("ETag: {etag}\r\n") };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\n{validator}{cr}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()-start);
                if socket.write_all(head.as_bytes()).await.is_err() { return; }
                for (i, chunk) in body[start..].chunks(16_384).enumerate() {
                    if socket.write_all(chunk).await.is_err() { break; }
                    if current == 5 && i == 5 { mode.store(0, Ordering::SeqCst); break; }
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
            });
        }
    });
    Server { address, mode, requests, body, task }
}

async fn paused(root: &Path, id: &str, server: &Server) -> u64 {
    let (tx, rx) = watch::channel(false);
    let result = transfer_direct(root, DestinationIdentity::from_path(root).unwrap(), id, "mp4", "logical-video", &server.url(), false, rx, move |p| {
        if p.bytes >= 65_536 { let _ = tx.send(true); }
    }).await.unwrap();
    let TransferResult::Paused { bytes } = result else { panic!("expected paused transfer") };
    assert!(bytes >= 65_536 && bytes < server.body.len() as u64);
    bytes
}
async fn finish(root: &Path, id: &str, server: &Server) -> std::path::PathBuf {
    let (_tx, rx) = watch::channel(false);
    let result = transfer_direct(root, DestinationIdentity::from_path(root).unwrap(), id, "mp4", "logical-video", &server.url(), false, rx, |_| {}).await.unwrap();
    let TransferResult::Completed { path, bytes } = result else { panic!("expected completed transfer") };
    assert_eq!(bytes, server.body.len() as u64);
    assert_eq!(std::fs::read(&path).unwrap(), *server.body);
    path
}

#[tokio::test]
async fn range_resume_survives_reopen_and_preserves_exact_bytes() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    let offset = paused(dir.path(), &id, &server).await;
    let path = finish(dir.path(), &id, &server).await;
    assert!(path.starts_with(dir.path()));
    let requests = server.requests.lock().await;
    assert!(requests[1].to_ascii_lowercase().contains(&format!("range: bytes={offset}-")));
    assert!(requests[1].to_ascii_lowercase().contains("if-range: \"v1\""));
    assert!(!dir.path().join(format!(".cloudstream-{id}/video.part")).exists());
}

#[tokio::test]
async fn ignored_ranges_changed_validators_and_416_restart_without_corruption() {
    for mode in [1, 2, 3, 4, 6] {
        let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
        if mode == 4 { server.mode.store(4, Ordering::SeqCst); }
        let _ = paused(dir.path(), &id, &server).await;
        server.mode.store(mode, Ordering::SeqCst);
        finish(dir.path(), &id, &server).await;
        let requests = server.requests.lock().await;
        if mode == 2 || mode == 3 || mode == 6 {
            assert!(requests.len() >= 3);
            assert!(!requests.last().unwrap().to_ascii_lowercase().contains("range: bytes="));
        }
        if mode == 4 { assert!(!requests[1].to_ascii_lowercase().contains("range: bytes=")); }
    }
}

#[tokio::test]
async fn completed_partial_416_and_unsafe_paths_are_handled_without_overwrite() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    paused(dir.path(), &id, &server).await;
    let partial = dir.path().join(format!(".cloudstream-{id}/video.part"));
    std::fs::write(&partial, &*server.body).unwrap();
    finish(dir.path(), &id, &server).await;
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), "../../escape", "mp4", "x", &server.url(), false, rx, |_| {}).await.is_err());
    let target = dir.path().join("keep"); std::fs::write(&target, b"do not change").unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    symlink(&target, dir.path().join(format!("cloudstream-{other}.mp4"))).unwrap();
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &other, "mp4", "x", &server.url(), false, rx, |_| {}).await.is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"do not change");
}

#[tokio::test]
async fn interrupted_connection_retries_with_range_and_completes() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    server.mode.store(5, Ordering::SeqCst);
    let (_tx, rx) = watch::channel(false);
    let result = transfer_with_retry(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "stable-source", &server.url(), false, rx, 3, |_| {}).await.unwrap();
    let TransferResult::Completed { path, bytes } = result else { panic!("expected completion") };
    assert_eq!(bytes, server.body.len() as u64);
    assert_eq!(std::fs::read(path).unwrap(), *server.body);
    let requests = server.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[1].to_ascii_lowercase().contains("range: bytes="));
}

#[tokio::test]
async fn simultaneous_attempts_for_one_job_cannot_write_the_same_partial() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    let (pause_tx, pause_rx) = watch::channel(false);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let mut started_tx = Some(started_tx);
    let root = dir.path().to_owned(); let first_id = id.clone(); let url = server.url();
    let first = tokio::spawn(async move {
        transfer_direct(&root, DestinationIdentity::from_path(&root).unwrap(), &first_id, "mp4", "logical-video", &url, false, pause_rx, move |p| {
            if p.bytes > 0 { if let Some(tx) = started_tx.take() { let _ = tx.send(()); } }
        }).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started_rx).await.unwrap().unwrap();
    let (_other_tx, other_rx) = watch::channel(false);
    let error = transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "logical-video", &server.url(), false, other_rx, |_| {}).await.unwrap_err();
    assert!(error.to_string().contains("already running"));
    pause_tx.send(true).unwrap();
    assert!(matches!(first.await.unwrap().unwrap(), TransferResult::Paused { .. }));
    finish(dir.path(), &id, &server).await;
}

#[tokio::test]
async fn strong_last_modified_allows_resume_and_source_change_forces_restart() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    server.mode.store(7, Ordering::SeqCst);
    let offset = paused(dir.path(), &id, &server).await;
    finish(dir.path(), &id, &server).await;
    let requests = server.requests.lock().await;
    assert!(requests[1].to_ascii_lowercase().contains(&format!("range: bytes={offset}-")));
    assert!(requests[1].to_ascii_lowercase().contains("if-range: wed, 21 oct 2015 07:28:00 gmt"));
    drop(requests);
    let next = uuid::Uuid::new_v4().to_string();
    paused(dir.path(), &next, &server).await;
    let (_tx, rx) = watch::channel(false);
    let result = transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &next, "mp4", "different-source", &server.url(), false, rx, |_| {}).await.unwrap();
    assert!(matches!(result, TransferResult::Completed { .. }));
    let requests = server.requests.lock().await;
    assert!(!requests.last().unwrap().to_ascii_lowercase().contains("range: bytes="));
}

#[tokio::test]
async fn replaced_folder_and_symlinked_partial_are_rejected() {
    use std::os::unix::fs::{symlink, DirBuilderExt};
    let parent = tempfile::tempdir().unwrap(); let server = server().await;
    let root = parent.path().join("downloads"); std::fs::create_dir(&root).unwrap();
    let identity = DestinationIdentity::from_path(&root).unwrap();
    std::fs::rename(&root, parent.path().join("moved")).unwrap();
    std::fs::create_dir(&root).unwrap();
    let id = uuid::Uuid::new_v4().to_string(); let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(&root, identity, &id, "mp4", "video", &server.url(), false, rx, |_| {}).await.unwrap_err().to_string().contains("folder has changed"));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    let target = parent.path().join("original"); std::fs::write(&target, b"untouched").unwrap();
    let id = uuid::Uuid::new_v4().to_string(); let job = root.join(format!(".cloudstream-{id}"));
    std::fs::DirBuilder::new().mode(0o700).create(&job).unwrap(); symlink(&target, job.join("video.part")).unwrap();
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(&root, DestinationIdentity::from_path(&root).unwrap(), &id, "mp4", "video", &server.url(), false, rx, |_| {}).await.is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"untouched");
}

#[tokio::test]
async fn interrupted_publication_is_recovered_only_for_the_same_source() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    paused(dir.path(), &id, &server).await;
    let part = dir.path().join(format!(".cloudstream-{id}/video.part"));
    std::fs::write(&part, &*server.body).unwrap();
    let final_path = dir.path().join(format!("cloudstream-{id}.mp4"));
    std::fs::hard_link(&part, &final_path).unwrap();
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "different", &server.url(), false, rx, |_| {}).await.is_err());
    let count = server.requests.lock().await.len();
    finish(dir.path(), &id, &server).await;
    assert_eq!(server.requests.lock().await.len(), count, "publication recovery should not re-download");
    assert!(!part.exists());
}

#[tokio::test]
async fn completed_marker_recovers_restart_without_replacing_an_unrelated_file() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    let path = finish(dir.path(), &id, &server).await;
    let requests = server.requests.lock().await.len();
    finish(dir.path(), &id, &server).await;
    assert_eq!(server.requests.lock().await.len(), requests);
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "different-source", &server.url(), false, rx, |_| {}).await.is_err());
    std::fs::remove_file(&path).unwrap(); std::fs::write(&path, b"my file").unwrap();
    let (_tx, rx) = watch::channel(false);
    assert!(transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "logical-video", &server.url(), false, rx, |_| {}).await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"my file");
}

#[tokio::test]
async fn durable_completion_marker_recovers_if_partial_deletion_was_not_persisted() {
    let dir = tempfile::tempdir().unwrap(); let server = server().await; let id = uuid::Uuid::new_v4().to_string();
    let output = finish(dir.path(), &id, &server).await;
    let partial = dir.path().join(format!(".cloudstream-{id}/video.part"));
    assert!(!dir.path().join(format!(".cloudstream-{id}/resume.json")).exists());
    std::fs::hard_link(&output, &partial).unwrap();
    let before = server.requests.lock().await.len();
    finish(dir.path(), &id, &server).await;
    assert_eq!(server.requests.lock().await.len(), before);
    assert!(!partial.exists());
}

#[tokio::test]
async fn disguised_manifests_and_html_are_never_published_as_video_files() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/video.wav", b"#EXTM3U\n#EXTINF:10,\nsegment.ts\n".to_vec()), ("/error.wav", b"<!DOCTYPE html><html>expired</html>".to_vec())]).await;
    for path in ["/video.wav", "/error.wav"] {
        let id = uuid::Uuid::new_v4().to_string(); let (_tx, rx) = watch::channel(false);
        let result = transfer_direct(dir.path(), DestinationIdentity::from_path(dir.path()).unwrap(), &id, "mp4", "video", &server.url(path), false, rx, |_| {}).await;
        assert!(result.unwrap_err().to_string().contains("not a direct video"));
        assert!(!dir.path().join(format!("cloudstream-{id}.mp4")).exists());
    }
}
