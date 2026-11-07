mod support;
use cloudstream_core::{download_manager::DownloadManager, download_queue::{DownloadAction, DownloadRequest, DownloadSourcePolicy, DownloadState}, models::{Episode, MediaItem}, App};
use std::{sync::Arc, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::Mutex};

async fn fixture() -> (tempfile::TempDir, Arc<App>, String, Arc<Vec<u8>>, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let src = dir.path().join("DownloadFixture.java");
    std::fs::write(&src, r#"
import com.lagradost.cloudstream3.*;
import com.lagradost.cloudstream3.plugins.*;
import com.lagradost.cloudstream3.utils.*;
import kotlin.Unit;
import kotlin.coroutines.Continuation;
import kotlin.jvm.functions.Function1;
import java.util.*;
@CloudstreamPlugin public class DownloadFixture extends BasePlugin {
  public void load() { registerMainAPI(new Source()); }
  public static class Source extends MainAPI {
    public String getName() { return "Download fixture"; }
    public String getMainUrl() { return "http://127.0.0.1"; }
    public Object loadLinks(String base, boolean cast, Function1<? super SubtitleFile,Unit> sub, Function1<? super ExtractorLink,Unit> callback, Continuation<? super Boolean> continuation) {
      SubtitleFile subtitle = new SubtitleFile("English", base + "/subs/provider.srt");
      subtitle.setHeaders(Map.of("X-Subtitle-Secret", "subtitle-fixture"));
      sub.invoke(subtitle);
      callback.invoke(new ExtractorLink("Fixture", "Direct", base + "/file.mp4", "", 720, Map.of("X-Download-Secret", "fixture"), null, ExtractorLinkType.VIDEO, List.of()));
      callback.invoke(new ExtractorLink("Fixture", "HLS", base + "/fmp4/master.m3u8", "", 720, Map.of("X-Download-Secret", "fixture"), null, ExtractorLinkType.M3U8, List.of()));
      return Boolean.TRUE;
    }
  }
}
"#).unwrap();
    let classes = dir.path().join("classes"); std::fs::create_dir(&classes).unwrap();
    let compile = std::process::Command::new("javac").args(["--release", "21"]).arg("-cp").arg(resources.join("worker/worker.jar")).arg("-d").arg(&classes).arg(&src).output().unwrap();
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let jar = dir.path().join("fixture.jar");
    assert!(std::process::Command::new("jar").arg("cf").arg(&jar).arg("-C").arg(&classes).arg(".").status().unwrap().success());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let body = Arc::new(support::wav(131.0));
    let served = body.clone();
    let logs = Arc::new(Mutex::new(Vec::<String>::new())); let captured = logs.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = served.clone(); let logs = captured.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192]; let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                logs.lock().await.push(request.clone());
                let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("");
                let expected_header = if path == "/subs/provider.srt" { "x-subtitle-secret: subtitle-fixture" } else { "x-download-secret: fixture" };
                if !request.lines().any(|line| line.trim().eq_ignore_ascii_case(expected_header)) {
                    let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await; return;
                }
                if path == "/subs/provider.srt" {
                    let data = include_bytes!("fixtures/hls/subs/provider.srt");
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-subrip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(data).await;
                    return;
                }
                if let Some(name) = path.strip_prefix("/fmp4/") {
                    if name.contains('/') || name.contains("..") { return; }
                    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls/fmp4").join(name);
                    let Ok(data) = std::fs::read(file) else {
                        let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await; return;
                    };
                    let kind = if name.ends_with(".m3u8") { "application/vnd.apple.mpegurl" } else { "video/mp4" };
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&data).await;
                    return;
                }
                let range = request.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").and_then(|v| v.split('-').next()?.parse::<usize>().ok()));
                let start = range.unwrap_or(0).min(body.len());
                let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
                let cr = if range.is_some() { format!("Content-Range: bytes {start}-{}/{}\r\n", body.len()-1, body.len()) } else { String::new() };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nETag: \"fixture-v1\"\r\n{cr}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()-start);
                if socket.write_all(head.as_bytes()).await.is_err() { return; }
                for chunk in body[start..].chunks(16_384) {
                    if socket.write_all(chunk).await.is_err() { break; }
                    tokio::time::sleep(Duration::from_millis(4)).await;
                }
            });
        }
    });
    let app = App::open_with_player(dir.path().join("data"), resources, support::headless_player(dir.path())).unwrap();
    app.install_local(&jar).await.unwrap();
    assert!(app.providers().await.unwrap().iter().any(|p| p.name == "Download fixture"));
    (dir, app, format!("http://{address}"), body, logs, task)
}

#[tokio::test]
async fn protected_hls_queue_saves_separate_audio_and_removes_only_its_files() {
    let (dir, app, base, _body, logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let mut hls = request(dir.path(), provider.id, base, 1);
    hls.extension = "hls".into();
    hls.policy.stream_name = Some("HLS".into());
    let job = manager.enqueue("default", hls).unwrap();
    let completed = wait_job(&app, &job.id, DownloadState::Completed).await;
    let output = completed.output.as_ref().unwrap();
    assert!(output.join("index.m3u8").exists());
    let manifest = cloudstream_core::hls_download::read_manifest(&completed.destination, completed.destination_identity, &job.id).unwrap().unwrap();
    assert!(manifest.audio.is_some());
    assert_eq!(manifest.external_subtitles.len(), 1);
    assert_eq!(std::fs::read(output.join(&manifest.external_subtitles[0].path)).unwrap(), include_bytes!("fixtures/hls/subs/provider.srt"));
    let saved_manifest = std::fs::read_to_string(output.join("manifest.json")).unwrap();
    assert!(!saved_manifest.contains("subtitle-fixture") && !saved_manifest.contains("127.0.0.1"));
    assert_eq!(completed.total, Some(completed.bytes));
    let requests = logs.lock().await;
    assert!(requests.iter().filter(|r| !r.contains("/subs/provider.srt")).all(|r| r.to_ascii_lowercase().contains("x-download-secret: fixture")));
    let subtitle_request = requests.iter().find(|r| r.contains("/subs/provider.srt")).expect("subtitle was fetched");
    assert!(subtitle_request.to_ascii_lowercase().contains("x-subtitle-secret: subtitle-fixture"));
    assert!(!subtitle_request.to_ascii_lowercase().contains("x-download-secret"));
    assert!(requests.iter().any(|r| r.contains("/fmp4/pEnglish.m3u8")));
    assert!(requests.iter().any(|r| r.contains("/fmp4/init_1.mp4")));
    drop(requests);
    let probe = std::process::Command::new("ffprobe").args(["-v", "error", "-protocol_whitelist", "file,crypto,data", "-show_entries", "stream=codec_type", "-of", "csv=p=0"]).arg(output.join("index.m3u8")).output().unwrap();
    assert!(probe.status.success(), "{}", String::from_utf8_lossy(&probe.stderr));
    let streams = String::from_utf8_lossy(&probe.stdout);
    assert!(streams.contains("video") && streams.contains("audio"), "{streams}");
    assert!(app.play_download(&job.id, "other-profile").await.unwrap_err().to_string().contains("not found"));
    if support::mpv_installed() {
        server.abort();
        app.player.set_policy(cloudstream_core::models::PlaybackPolicy { start_paused: true, ..Default::default() }).await;
        let session = app.play_download(&job.id, "default").await.unwrap();
        let state = support::wait_for_state(&app.player, 10, "offline HLS to load", |state| state.duration > 5.0).await;
        assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
        assert!(state.tracks.iter().any(|track| track.kind == "audio"));
        support::wait_for_state(&app.player, 10, "offline subtitle", |state| state.tracks.iter().any(|track| track.kind == "sub" && track.external)).await;
        app.player_command(cloudstream_core::models::PlayerCommand::SeekAbsolute { seconds: 2.5 }).await.unwrap();
        support::wait_for_state(&app.player, 5, "offline seek", |state| state.position >= 2.0).await;
        app.save_progress().await.unwrap();
        let saved = app.store.get_watch_progress("default", &job.episode_id).unwrap().unwrap();
        assert!(saved.position_seconds >= 2.0);
        assert!(app.store.get_watch_progress("other-profile", &job.episode_id).unwrap().is_none());
        let resumed = app.play_download(&job.id, "default").await.unwrap();
        let state = support::wait_for_state(&app.player, 10, "offline resume", |state| state.session_id.as_deref() == Some(resumed.as_str()) && state.duration > 5.0 && state.position >= 2.0).await;
        assert_eq!(state.episode_id.as_deref(), Some(job.episode_id.as_str()));
        app.player.stop().await;
    }
    let altered = output.join("video/seg-000000.m4s");
    let original = std::fs::read(&altered).unwrap();
    std::fs::write(&altered, b"changed").unwrap();
    assert!(app.play_download(&job.id, "default").await.unwrap_err().to_string().contains("missing or have changed"));
    std::fs::write(&altered, original).unwrap();
    let report = manager.remove("default", &job.id, true).await.unwrap();
    assert!(report.file_deleted, "{:?}", report.notes);
    assert!(!output.exists());
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn repair_queues_a_profile_owned_replacement_without_overwriting_edited_files() {
    let (dir, app, base, body, _logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let original = manager.enqueue("default", request(dir.path(), provider.id, base, 1)).unwrap();
    let completed = wait_job(&app, &original.id, DownloadState::Completed).await;
    let output = completed.output.unwrap();
    // Allow the runner to release its control before requesting the repair.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.repair("default", &original.id).await {
                Err(error) if error.to_string().contains("still stopping") => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(error) => { assert!(error.to_string().contains("already present")); break; },
                Ok(_) => panic!("a healthy download must not be repaired"),
            }
        }
    }).await.unwrap();
    let mut edited = (*body).clone();
    edited[1000] ^= 1;
    std::fs::write(&output, &edited).unwrap();
    assert!(manager.repair("other-profile", &original.id).await.unwrap_err().to_string().contains("not found"));
    let replacement = manager.repair("default", &original.id).await.unwrap();
    assert_eq!(replacement.repair_of.as_deref(), Some(original.id.as_str()));
    let duplicate = manager.repair("default", &original.id).await.unwrap();
    assert_eq!(duplicate.id, replacement.id);
    let repaired = wait_job(&app, &replacement.id, DownloadState::Completed).await;
    assert_eq!(std::fs::read(repaired.output.unwrap()).unwrap(), *body);
    assert_eq!(std::fs::read(&output).unwrap(), edited);
    assert_eq!(app.store.get_download_job(&original.id).unwrap().unwrap().state, DownloadState::Completed);
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn repair_replaces_a_missing_hls_segment_in_a_separate_download() {
    let (dir, app, base, _body, _logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let mut request = request(dir.path(), provider.id, base, 1);
    request.extension = "hls".into(); request.policy.stream_name = Some("HLS".into());
    let original = manager.enqueue("default", request).unwrap();
    let completed = wait_job(&app, &original.id, DownloadState::Completed).await;
    let old = completed.output.unwrap();
    let missing = old.join("video/seg-000000.m4s");
    std::fs::remove_file(&missing).unwrap();
    let replacement = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.repair("default", &original.id).await {
                Err(error) if error.to_string().contains("still stopping") => tokio::time::sleep(Duration::from_millis(20)).await,
                result => break result.unwrap(),
            }
        }
    }).await.unwrap();
    let repaired = wait_job(&app, &replacement.id, DownloadState::Completed).await;
    assert!(repaired.output.unwrap().join("video/seg-000000.m4s").exists());
    assert!(old.exists() && !missing.exists(), "the original remains untouched");
    assert!(old.join("audio/index.m3u8").exists());
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn direct_download_keeps_protected_subtitle_for_offline_playback_and_deletes_owned_sidecar() {
    let (dir, app, base, _body, logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let job = manager.enqueue("default", request(dir.path(), provider.id, base, 1)).unwrap();
    let completed = wait_job(&app, &job.id, DownloadState::Completed).await;
    let (subtitles, missing) = cloudstream_core::direct_subtitles::read(&completed.destination, completed.destination_identity, &job.id, &completed.extension).unwrap().unwrap();
    assert_eq!(missing, 0);
    assert_eq!(subtitles.len(), 1);
    assert_eq!(std::fs::read(&subtitles[0].url).unwrap(), include_bytes!("fixtures/hls/subs/provider.srt"));
    let requests = logs.lock().await;
    let subtitle = requests.iter().find(|r| r.contains("/subs/provider.srt")).unwrap().to_ascii_lowercase();
    assert!(subtitle.contains("x-subtitle-secret: subtitle-fixture") && !subtitle.contains("x-download-secret"));
    drop(requests);
    server.abort();
    if support::mpv_installed() {
        app.player.set_policy(cloudstream_core::models::PlaybackPolicy { start_paused: true, ..Default::default() }).await;
        app.play_download(&job.id, "default").await.unwrap();
        support::wait_for_state(&app.player, 10, "offline direct video and subtitle", |state| state.duration > 100.0 && state.tracks.iter().any(|track| track.kind == "sub" && track.external)).await;
        app.player.stop().await;
    }
    let sidecar = std::path::Path::new(&subtitles[0].url).parent().unwrap().to_path_buf();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.remove("default", &job.id, true).await {
                Err(error) if error.to_string().contains("still stopping") => tokio::time::sleep(Duration::from_millis(20)).await,
                result => break result.unwrap(),
            }
        }
    }).await.unwrap();
    assert!(result.file_deleted && result.temp_cleaned, "{result:?}");
    assert!(!sidecar.exists());
    manager.shutdown().await; drop(manager); app.worker.stop().await;
}

fn request(root: &std::path::Path, provider_id: String, payload: String, number: i32) -> DownloadRequest {
    let item: MediaItem = serde_json::from_value(serde_json::json!({"providerId":provider_id,"name":"Example","url":"https://example.test/show"})).unwrap();
    DownloadRequest { item, episode: Episode { name: format!("Episode {number}"), season: Some(1), episode: Some(number), data: payload, ..Default::default() }, destination: root.into(), extension: "mp4".into(), policy: DownloadSourcePolicy { source_name: Some("Fixture".into()), stream_name: Some("Direct".into()), quality: Some(720) } }
}
async fn wait_job(app: &App, id: &str, state: DownloadState) -> cloudstream_core::download_queue::DownloadJob {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(job) = app.store.get_download_job(id).unwrap() {
                if job.state == state { break job; }
                if job.state == DownloadState::Failed && state != DownloadState::Failed { panic!("download failed: {:?}; diagnostics: {:?}", job.error, app.diagnostics.events()); }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await.expect("timed out waiting for download state")
}

#[tokio::test]
async fn protected_provider_download_pauses_and_resumes_via_its_session() {
    let (dir, app, base, body, logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let mut events = manager.subscribe();
    let job = manager.enqueue("default", request(dir.path(), provider.id, base, 1)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(cloudstream_core::download_queue::DownloadEvent::JobChanged { job: updated }) = events.recv().await {
                if updated.id == job.id && updated.state == DownloadState::Downloading && updated.bytes > 0 { break; }
                if updated.id == job.id && updated.state == DownloadState::Failed { panic!("download failed: {:?}", updated.error); }
            }
        }
    }).await.expect("no transfer progress");
    manager.control("default", &job.id, DownloadAction::Pause).await.unwrap();
    wait_job(&app, &job.id, DownloadState::Paused).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match manager.control("default", &job.id, DownloadAction::Resume).await {
                Ok(_) => break,
                Err(error) if error.to_string().contains("still stopping") => tokio::time::sleep(Duration::from_millis(30)).await,
                Err(error) => panic!("resume failed: {error}"),
            }
        }
    }).await.unwrap();
    let completed = wait_job(&app, &job.id, DownloadState::Completed).await;
    assert_eq!(std::fs::read(completed.output.unwrap()).unwrap(), *body);
    let requests = logs.lock().await;
    assert!(requests.len() >= 2);
    assert!(requests.iter().filter(|r| !r.contains("/subs/provider.srt")).all(|r| r.to_ascii_lowercase().contains("x-download-secret: fixture")));
    assert!(requests.iter().skip(1).any(|r| r.to_ascii_lowercase().contains("range: bytes=") && r.to_ascii_lowercase().contains("if-range: \"fixture-v1\"")));
    drop(requests);
    manager.shutdown().await;
    tokio::time::timeout(Duration::from_secs(1), manager.shutdown()).await.expect("shutdown should be idempotent");
    drop(manager);
    app.worker.stop().await; server.abort();
}


#[tokio::test]
async fn cancel_releases_the_slot_and_does_not_delete_or_publish_a_partial() {
    let (dir, app, base, body, _logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let mut events = manager.subscribe();
    let first = manager.enqueue("default", request(dir.path(), provider.id.clone(), base.clone(), 1)).unwrap();
    let second = manager.enqueue("default", request(dir.path(), provider.id, base, 2)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(cloudstream_core::download_queue::DownloadEvent::JobChanged { job }) = events.recv().await {
                if job.id == first.id && job.state == DownloadState::Downloading && job.bytes > 0 { break; }
                if job.id == first.id && job.state == DownloadState::Failed { panic!("download failed: {:?}", job.error); }
            }
        }
    }).await.unwrap();
    assert_eq!(app.store.get_download_job(&second.id).unwrap().unwrap().state, DownloadState::Queued);
    manager.control("default", &first.id, DownloadAction::Cancel).await.unwrap();
    let completed = wait_job(&app, &second.id, DownloadState::Completed).await;
    assert_eq!(std::fs::read(completed.output.unwrap()).unwrap(), *body);
    assert_eq!(app.store.get_download_job(&first.id).unwrap().unwrap().state, DownloadState::Canceled);
    assert!(!dir.path().join(format!("cloudstream-{}.mp4", first.id)).exists());
    assert!(dir.path().join(format!(".cloudstream-{}/video.part", first.id)).exists());
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn shutdown_requeues_active_transfer_and_next_manager_resumes_it() {
    let (dir, app, base, body, logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap(); let mut events = manager.subscribe();
    let job = manager.enqueue("default", request(dir.path(), provider.id, base, 1)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(cloudstream_core::download_queue::DownloadEvent::JobChanged { job: updated }) = events.recv().await {
                if updated.id == job.id && updated.state == DownloadState::Downloading && updated.bytes > 0 { break; }
                if updated.id == job.id && updated.state == DownloadState::Failed { panic!("download failed: {:?}", updated.error); }
            }
        }
    }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), manager.shutdown()).await.unwrap();
    assert_eq!(app.store.get_download_job(&job.id).unwrap().unwrap().state, DownloadState::Queued);
    drop(manager);
    let restarted = DownloadManager::start(&app, 1).unwrap();
    let completed = wait_job(&app, &job.id, DownloadState::Completed).await;
    assert_eq!(std::fs::read(completed.output.unwrap()).unwrap(), *body);
    assert!(logs.lock().await.iter().skip(1).any(|r| r.to_ascii_lowercase().contains("range: bytes=")));
    restarted.shutdown().await; drop(restarted); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn removal_requires_an_inactive_job_owned_by_the_profile_and_cleans_up() {
    let (dir, app, base, body, _logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    let mut events = manager.subscribe();
    let first = manager.enqueue("default", request(dir.path(), provider.id.clone(), base.clone(), 1)).unwrap();
    let second = manager.enqueue("default", request(dir.path(), provider.id, base, 2)).unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(cloudstream_core::download_queue::DownloadEvent::JobChanged { job }) = events.recv().await {
                if job.id == first.id && job.state == DownloadState::Downloading && job.bytes > 0 { break; }
            }
        }
    }).await.unwrap();

    // A transfer in progress cannot be removed, and another profile cannot see the job at all.
    let error = manager.remove("default", &first.id, true).await.unwrap_err();
    assert!(error.to_string().contains("Pause or cancel"), "{error}");
    assert!(manager.remove("someone-else", &first.id, true).await.is_err());
    assert!(app.store.get_download_job(&first.id).unwrap().is_some());

    // Cancelling keeps the partial for a retry; removing then discards it and the record.
    manager.control("default", &first.id, DownloadAction::Cancel).await.unwrap();
    wait_job(&app, &first.id, DownloadState::Canceled).await;
    let first_dir = dir.path().join(format!(".cloudstream-{}", first.id));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match manager.remove("default", &first.id, true).await {
                Ok(report) => break report,
                Err(error) if error.to_string().contains("still stopping") => tokio::time::sleep(Duration::from_millis(30)).await,
                Err(error) => panic!("remove failed: {error}"),
            }
        }
    }).await.map(|report| {
        assert!(report.temp_cleaned && !report.file_deleted, "{report:?}");
        assert!(report.notes.iter().any(|n| n.contains("Only finished")), "{report:?}");
    }).unwrap();
    assert!(!first_dir.exists());
    assert!(app.store.get_download_job(&first.id).unwrap().is_none());

    // A finished download can be removed with or without its file.
    let done = wait_job(&app, &second.id, DownloadState::Completed).await;
    let output = done.output.clone().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), *body);
    let report = manager.remove("default", &second.id, true).await.unwrap();
    assert!(report.file_deleted && report.temp_cleaned, "{report:?}");
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains("cloudstream")).count(), 0);
    assert!(app.store.get_download_job(&second.id).unwrap().is_none());
    assert!(manager.remove("default", &second.id, true).await.is_err(), "removing twice reports not found");
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}

#[tokio::test]
async fn batch_enqueue_reports_duplicates_and_keeps_going_after_a_bad_item() {
    let (dir, app, base, _body, _logs, server) = fixture().await;
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Download fixture").unwrap();
    let manager = DownloadManager::start(&app, 1).unwrap();
    // Pause the queue's progress by keeping everything queued: no worker slot is needed to enqueue.
    let mut batch: Vec<DownloadRequest> = (1..=3).map(|n| request(dir.path(), provider.id.clone(), format!("{base}/ep{n}"), n)).collect();
    let mut bad = request(dir.path(), provider.id.clone(), base.clone(), 4);
    bad.extension = "exe".into();
    batch.push(bad);
    let summary = manager.enqueue_many("default", batch).unwrap();
    assert_eq!((summary.added, summary.already_queued), (3, 0));
    assert_eq!(summary.failed.len(), 1);
    assert!(summary.failed[0].starts_with("Episode 4:") && summary.failed[0].contains("Unsupported"), "{:?}", summary.failed);

    // Asking again for the same episodes adds nothing, whatever their order or the folder.
    let again: Vec<DownloadRequest> = (1..=3).rev().map(|n| request(dir.path(), provider.id.clone(), format!("{base}/ep{n}"), n)).collect();
    let summary = manager.enqueue_many("default", again).unwrap();
    assert_eq!((summary.added, summary.already_queued, summary.failed.len()), (0, 3, 0));
    assert_eq!(app.store.download_jobs(Some("default")).unwrap().len(), 3);
    manager.shutdown().await; drop(manager); app.worker.stop().await; server.abort();
}
