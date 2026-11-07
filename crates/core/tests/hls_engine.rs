//! Additional engine checks against the real fixtures (made with ffmpeg by
//! scripts/make-hls-fixtures.sh), served by a local HTTP server that can refuse, truncate,
//! impersonate and disappear. Complements `hls_downloads.rs`.
use cloudstream_core::{
    downloads::DestinationIdentity,
    hls::SelectionPolicy,
    hls_download::{check_folder, download_hls, read_manifest, remove_folder, ExternalSubtitle, HlsFailure, HlsProgress, HlsRequest, HlsResult, SegmentProblem},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::watch,
};

mod support;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hls")
}

#[derive(Clone)]
enum Reply {
    Status(u16),
    Body(Vec<u8>),
}

struct Server {
    address: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<(String, Option<String>)>>>,
    overrides: Arc<Mutex<HashMap<String, Reply>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}/{path}", self.address)
    }
    fn set(&self, path: &str, reply: Reply) {
        self.overrides.lock().unwrap().insert(path.to_string(), reply);
    }
    fn clear(&self, path: &str) {
        self.overrides.lock().unwrap().remove(path);
    }
    fn count(&self, path: &str) -> usize {
        self.requests.lock().unwrap().iter().filter(|(p, _)| p == path).count()
    }
    fn ranges(&self, path: &str) -> Vec<String> {
        self.requests.lock().unwrap().iter().filter(|(p, _)| p == path).filter_map(|(_, r)| r.clone()).collect()
    }
}

async fn serve() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let overrides: Arc<Mutex<HashMap<String, Reply>>> = Arc::new(Mutex::new(HashMap::new()));
    let (log, table) = (requests.clone(), overrides.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (log, table) = (log.clone(), table.clone());
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").trim_start_matches('/').to_string();
                let range = request.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string));
                log.lock().unwrap().push((path.clone(), range.clone()));
                let reply = table.lock().unwrap().get(&path).cloned();
                let body = match reply {
                    Some(Reply::Status(code)) => {
                        let head = format!("HTTP/1.1 {code} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        let _ = socket.write_all(head.as_bytes()).await;
                        return;
                    }
                    Some(Reply::Body(bytes)) => Some(bytes),
                    None if !path.contains("..") => std::fs::read(fixtures().join(&path)).ok(),
                    None => None,
                };
                let Some(body) = body else {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                };
                let kind = if path.ends_with(".m3u8") { "application/vnd.apple.mpegurl" } else if path.ends_with(".vtt") { "text/vtt" } else { "application/octet-stream" };
                let (status, slice, extra) = match range.as_deref().and_then(|r| r.split_once('-')) {
                    Some((start, end)) if start.parse::<usize>().unwrap_or(0) >= body.len() => {
                        let _ = (start, end);
                        let head = format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", body.len());
                        let _ = socket.write_all(head.as_bytes()).await;
                        return;
                    }
                    Some((start, end)) => {
                        let start: usize = start.parse().unwrap_or(0);
                        let end: usize = end.parse().unwrap_or(body.len() - 1).min(body.len() - 1);
                        ("206 Partial Content", body[start..=end].to_vec(), format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len()))
                    }
                    None => ("200 OK", body, String::new()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", slice.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&slice).await;
            });
        }
    });
    Server { address, requests, overrides, task }
}

struct Job {
    root: PathBuf,
    id: String,
    _dir: tempfile::TempDir,
}

impl Job {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("downloads");
        std::fs::create_dir(&root).unwrap();
        Self { root, id: uuid::Uuid::new_v4().to_string(), _dir: dir }
    }
    fn identity(&self) -> DestinationIdentity {
        DestinationIdentity::from_path(&self.root).unwrap()
    }
    fn output(&self) -> PathBuf {
        self.root.join(format!("cloudstream-{}.hls", self.id))
    }
    fn stage(&self) -> PathBuf {
        self.root.join(format!(".cloudstream-{}", self.id)).join("stage")
    }
    fn request<'a>(&'a self, url: &'a str, policy: SelectionPolicy, subtitles: Vec<ExternalSubtitle>) -> HlsRequest<'a> {
        HlsRequest { root: &self.root, identity: self.identity(), job_id: &self.id, source_identity: "show-1/episode-1", url, no_proxy: true, policy, subtitles, skipped_subtitles: 0 }
    }
    async fn run(&self, url: &str) -> anyhow::Result<HlsResult> {
        self.run_with(url, SelectionPolicy::default(), vec![]).await
    }
    async fn run_with(&self, url: &str, policy: SelectionPolicy, subtitles: Vec<ExternalSubtitle>) -> anyhow::Result<HlsResult> {
        let (_tx, rx) = watch::channel(false);
        download_hls(self.request(url, policy, subtitles), rx, |_| {}).await
    }
    /// Run until `after` segments are stored, then pause.
    async fn run_until(&self, url: &str, after: usize) -> HlsResult {
        let (tx, rx) = watch::channel(false);
        download_hls(self.request(url, SelectionPolicy::default(), vec![]), rx, move |p: HlsProgress| {
            if p.segments_done >= after {
                let _ = tx.send(true);
            }
        })
        .await
        .unwrap()
    }
}

fn failure(error: &anyhow::Error) -> HlsFailure {
    error.downcast_ref::<HlsFailure>().cloned().unwrap_or_else(|| panic!("not an HLS failure: {error:#}"))
}

fn completed(result: anyhow::Result<HlsResult>) -> PathBuf {
    match result.unwrap() {
        HlsResult::Completed { path, .. } => path,
        other => panic!("expected completion: {other:?}"),
    }
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fixture(path: &str) -> Vec<u8> {
    read(&fixtures().join(path))
}

fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Play the finished folder in a headless mpv with no network and report what it found.
fn play_offline(playlist: &Path) -> Option<String> {
    if !support::mpv_installed() {
        return None;
    }
    let output = std::process::Command::new("timeout")
        .args(["60", "mpv", "--no-config", "--vo=null", "--ao=null", "--force-window=no", "--msg-level=all=warn,cplayer=info", "--network-timeout=1"])
        .arg(playlist)
        .env("http_proxy", "http://127.0.0.1:9")
        .env("https_proxy", "http://127.0.0.1:9")
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(text.contains("Exiting... (End of file)"), "mpv did not play the local copy to the end:\n{text}");
    Some(text)
}

#[tokio::test]
async fn a_muxed_transport_stream_becomes_a_complete_local_folder_that_plays_offline() {
    let server = serve().await;
    let job = Job::new();
    let url = server.url("ts/media.m3u8");
    let output = completed(job.run(&url).await);
    assert_eq!(output, job.output());

    let manifest = read_manifest(&job.root, job.identity(), &job.id).unwrap().expect("a manifest for the finished folder");
    assert!((manifest.duration_seconds - 6.0).abs() < 0.01);
    for n in 0..3 {
        assert_eq!(read(&output.join(format!("video/seg-00000{n}.ts"))), fixture(&format!("ts/seg{n}.ts")), "segment {n} is stored byte for byte");
    }
    let master = String::from_utf8(read(&output.join("index.m3u8"))).unwrap();
    assert!(master.contains("video/index.m3u8") && !master.contains("http"), "{master}");
    let playlist = String::from_utf8(read(&output.join("video/index.m3u8"))).unwrap();
    assert!(!playlist.contains("http") && !playlist.contains("EXT-X-KEY") && playlist.contains("seg-000002.ts"));

    // Nothing from the network address survived on disk, and no temporary data is left.
    for file in tree(&job.root) {
        let bytes = read(&job.root.join(&file));
        assert!(!String::from_utf8_lossy(&bytes).contains(&server.address.to_string()), "{file} mentions the source address");
    }
    assert!(!job.stage().exists());
    assert!(check_folder(&job.root, job.identity(), &job.id, true).unwrap().healthy());

    // Run again: the same call is a no-op that reports the same result.
    let before = server.requests.lock().unwrap().len();
    assert_eq!(completed(job.run(&url).await), output);
    assert_eq!(server.requests.lock().unwrap().len(), before, "a finished download makes no requests");

    drop(server);
    if let Some(text) = play_offline(&output.join("index.m3u8")) {
        assert!(text.contains("Video") && text.contains("Audio"), "{text}");
    }
}

#[tokio::test]
async fn separate_audio_subtitles_and_init_segments_are_kept_and_selected_by_language() {
    let server = serve().await;
    let master = "#EXTM3U\n#EXT-X-VERSION:7\n\
        #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,LANGUAGE=\"eng\",URI=\"fmp4/pEnglish.m3u8\"\n\
        #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"sub\",NAME=\"English\",LANGUAGE=\"en\",URI=\"subs/en.m3u8\"\n\
        #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"sub\",NAME=\"Deutsch\",LANGUAGE=\"de\",URI=\"subs/de.m3u8\"\n\
        #EXT-X-STREAM-INF:BANDWIDTH=51000,AVERAGE-BANDWIDTH=50000,RESOLUTION=64x48,CODECS=\"avc1.42c00a,mp4a.40.2\",AUDIO=\"aud\",SUBTITLES=\"sub\"\nfmp4/p0.m3u8\n";
    server.set("master.m3u8", Reply::Body(master.as_bytes().to_vec()));

    let job = Job::new();
    let policy = SelectionPolicy { subtitle_languages: vec!["en".into()], ..Default::default() };
    let output = completed(job.run_with(&server.url("master.m3u8"), policy, vec![ExternalSubtitle { language: "en".into(), name: "From the provider".into(), url: server.url("subs/provider.srt"), no_proxy: true }]).await);

    let manifest = read_manifest(&job.root, job.identity(), &job.id).unwrap().unwrap();
    assert_eq!((manifest.width, manifest.height), (Some(64), Some(48)));
    assert_eq!(manifest.audio.as_ref().unwrap().language.as_deref(), Some("eng"));
    assert_eq!(manifest.subtitles.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["English"], "only the wanted language is stored");
    assert_eq!(manifest.external_subtitles.len(), 1);
    assert_eq!(read(&output.join("external/sub-0.srt")), fixture("subs/provider.srt"));
    assert_eq!(read(&output.join("video/init-0.mp4")), fixture("fmp4/init_0.mp4"));
    assert_eq!(read(&output.join("audio/init-0.mp4")), fixture("fmp4/init_1.mp4"));
    assert_eq!(read(&output.join("audio/seg-000002.m4s")), fixture("fmp4/sEnglish_2.m4s"));
    assert_eq!(read(&output.join("subs-0/seg-000000.vtt")), fixture("subs/en0.vtt"));
    assert!(!server.requests.lock().unwrap().iter().any(|(p, _)| p == "subs/de.m3u8" || p == "subs/de0.vtt"), "unwanted languages are never fetched");
    assert!(check_folder(&job.root, job.identity(), &job.id, true).unwrap().healthy());

    drop(server);
    if let Some(text) = play_offline(&output.join("index.m3u8")) {
        assert!(text.contains("Video") && text.contains("Audio") && text.contains("Subs"), "video, audio and subtitles all play offline:\n{text}");
    }
}

#[tokio::test]
async fn aes128_segments_are_decrypted_as_stored_and_no_key_is_kept() {
    let server = serve().await;
    // Explicit initialisation vector (made by ffmpeg).
    let job = Job::new();
    let output = completed(job.run(&server.url("aes/media.m3u8")).await);
    assert!(read(&output.join("video/seg-000000.ts")).starts_with(&[0x47]), "decrypted segments are MPEG-TS again");
    assert_eq!(read(&output.join("video/seg-000001.ts")).len() % 188, 0);
    assert!(!tree(&job.root).iter().any(|f| f.contains("key")), "{:?}", tree(&job.root));
    let playlist = String::from_utf8(read(&output.join("video/index.m3u8"))).unwrap();
    assert!(!playlist.contains("KEY") && !playlist.contains("key.bin"));
    assert_eq!(server.count("aes/key.bin"), 1, "the key is fetched once per run, not once per segment");

    // No IV in the playlist: each segment's IV is its media sequence number (starting at 5 here).
    let job = Job::new();
    let output = completed(job.run(&server.url("aesseq/media.m3u8")).await);
    for n in 0..3 {
        assert_eq!(read(&output.join(format!("video/seg-00000{n}.ts"))), fixture(&format!("ts/seg{n}.ts")), "segment {n} decrypts to the original with its sequence-number IV");
    }
    drop(server);
    play_offline(&output.join("index.m3u8"));
}

#[tokio::test]
async fn a_wrong_key_or_a_missing_key_is_diagnosed_not_stored() {
    let server = serve().await;
    server.set("aesseq/key.bin", Reply::Body(vec![9; 16]));
    let job = Job::new();
    let error = job.run(&server.url("aesseq/media.m3u8")).await.unwrap_err();
    assert!(matches!(failure(&error), HlsFailure::Segment { problem: SegmentProblem::CannotDecrypt | SegmentProblem::NotMedia, .. }), "{error:#}");
    assert!(!job.output().exists());

    server.set("aesseq/key.bin", Reply::Body(vec![1; 5]));
    let job = Job::new();
    assert_eq!(failure(&job.run(&server.url("aesseq/media.m3u8")).await.unwrap_err()), HlsFailure::KeyUnavailable);
    server.set("aesseq/key.bin", Reply::Status(403));
    let job = Job::new();
    assert_eq!(failure(&job.run(&server.url("aesseq/media.m3u8")).await.unwrap_err()), HlsFailure::KeyUnavailable);
}

#[tokio::test]
async fn byte_ranges_are_requested_exactly_and_each_range_is_its_own_valid_file() {
    let server = serve().await;
    let job = Job::new();
    let output = completed(job.run(&server.url("range/media.m3u8")).await);
    let whole = fixture("range/media.ts");
    let joined: Vec<u8> = (0..3).flat_map(|n| read(&output.join(format!("video/seg-00000{n}.ts")))).collect();
    assert_eq!(joined, whole, "ranges cover the file exactly, in order");
    let mut asked = server.ranges("range/media.ts");
    asked.sort();
    assert_eq!(asked, ["0-25003", "25004-50195", "50196-75011"], "each segment asks for exactly its own bytes");

    let job = Job::new();
    let output = completed(job.run(&server.url("rangemp4/media.m3u8")).await);
    let whole = fixture("rangemp4/media.mp4");
    assert_eq!(read(&output.join("video/init-0.mp4")), whole[..1331], "the init range is its own file");
    assert_eq!(read(&output.join("video/seg-000000.m4s")), whole[1331..22697]);
    drop(server);
    play_offline(&output.join("index.m3u8"));
}

#[tokio::test]
async fn a_source_that_ignores_ranges_is_refused_instead_of_downloading_the_whole_file_per_segment() {
    let server = serve().await;
    // A plain 200 answer to a range request: the whole file would follow for every segment.
    server.set("range/media.ts", Reply::Status(200));
    let job = Job::new();
    let error = job.run(&server.url("range/media.m3u8")).await.unwrap_err();
    assert_eq!(failure(&error), HlsFailure::RangesUnsupported);
    assert!(!job.output().exists());
}

#[tokio::test]
async fn pausing_keeps_finished_segments_and_resuming_fetches_only_the_rest() {
    let server = serve().await;
    let job = Job::new();
    let url = server.url("ts/media.m3u8");
    let HlsResult::Paused { bytes } = job.run_until(&url, 1).await else { panic!("expected a pause") };
    assert!(bytes > 0 && !job.output().exists());
    let fetched_before: usize = (0..3).map(|n| server.count(&format!("ts/seg{n}.ts"))).sum();
    assert!((1..3).contains(&fetched_before) || fetched_before == 3);

    let output = completed(job.run(&url).await);
    for n in 0..3 {
        assert_eq!(read(&output.join(format!("video/seg-00000{n}.ts"))), fixture(&format!("ts/seg{n}.ts")));
        assert!(server.count(&format!("ts/seg{n}.ts")) <= 2);
    }
    // A segment stored before the pause was not fetched again.
    let total: usize = (0..3).map(|n| server.count(&format!("ts/seg{n}.ts"))).sum();
    assert!(total <= 3 + fetched_before.saturating_sub(1) + 3, "{total}");
    assert!(check_folder(&job.root, job.identity(), &job.id, true).unwrap().healthy());
}

#[tokio::test]
async fn resume_repairs_missing_and_altered_parts_and_nothing_else() {
    let server = serve().await;
    let job = Job::new();
    let url = server.url("fmp4/p0.m3u8");
    assert!(matches!(job.run_until(&url, 4).await, HlsResult::Paused { .. }));
    let video = job.stage().join("video");
    let mut stored: Vec<String> = std::fs::read_dir(&video).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("seg-") || n.starts_with("init-")).collect();
    stored.sort();
    assert!(stored.len() >= 2, "{stored:?}");
    // One stored part vanishes and another is altered without changing its size.
    std::fs::remove_file(video.join(&stored[0])).unwrap();
    let mut altered = read(&video.join(&stored[1]));
    let last = altered.len() - 1;
    altered[last] ^= 0xff;
    std::fs::write(video.join(&stored[1]), altered).unwrap();
    // A half-written file from a crash is never trusted either.
    std::fs::write(video.join(".tmp-crash"), b"partial").unwrap();

    let before = server.requests.lock().unwrap().len();
    let output = completed(job.run(&url).await);
    assert!(!tree(&job.root).iter().any(|f| f.contains(".tmp-")));
    assert_eq!(read(&output.join("video/seg-000000.m4s")), fixture("fmp4/s0_0.m4s"));
    assert_eq!(read(&output.join("video/seg-000002.m4s")), fixture("fmp4/s0_2.m4s"));
    assert_eq!(read(&output.join("video/init-0.mp4")), fixture("fmp4/init_0.mp4"));
    assert!(check_folder(&job.root, job.identity(), &job.id, true).unwrap().healthy());
    let new_requests = server.requests.lock().unwrap().len() - before;
    assert!(new_requests <= 1 + 4, "the playlist plus only what was missing, altered or unfinished: {new_requests}");
}

#[tokio::test]
async fn a_segment_that_disappears_is_named_and_finished_parts_survive_until_it_returns() {
    let server = serve().await;
    server.set("ts/seg1.ts", Reply::Status(404));
    let job = Job::new();
    let url = server.url("ts/media.m3u8");
    let error = job.run(&url).await.unwrap_err();
    assert_eq!(failure(&error), HlsFailure::Segment { track: "Video", number: 2, problem: SegmentProblem::Gone(404) });
    let message = failure(&error).to_string();
    assert!(message.contains("Video segment 2") && message.contains("404") && !message.contains("127.0.0.1"), "{message}");
    assert!(!job.output().exists(), "nothing is published for an incomplete download");

    server.clear("ts/seg1.ts");
    let stored_before = server.count("ts/seg0.ts");
    let output = completed(job.run(&url).await);
    assert_eq!(read(&output.join("video/seg-000001.ts")), fixture("ts/seg1.ts"));
    assert!(server.count("ts/seg0.ts") <= stored_before + 1);
}

#[tokio::test]
async fn bad_content_and_wrong_sizes_are_diagnosed() {
    let server = serve().await;
    server.set("ts/seg2.ts", Reply::Body(b"<html><body>Please sign in</body></html>".to_vec()));
    let job = Job::new();
    assert_eq!(failure(&job.run(&server.url("ts/media.m3u8")).await.unwrap_err()), HlsFailure::Segment { track: "Video", number: 3, problem: SegmentProblem::NotMedia });
    assert!(!job.output().exists());

    // A range reply shorter than promised.
    server.clear("ts/seg2.ts");
    let mut short = fixture("range/media.ts");
    short.truncate(60_000);
    server.set("range/media.ts", Reply::Body(short));
    let job = Job::new();
    let error = job.run(&server.url("range/media.m3u8")).await.unwrap_err();
    assert!(matches!(failure(&error), HlsFailure::RangesUnsupported | HlsFailure::Segment { problem: SegmentProblem::WrongSize | SegmentProblem::Unreachable, .. }), "a reply that does not match the requested range is refused: {error:#}");
}

#[tokio::test]
async fn a_changed_source_discards_old_parts_and_downloads_the_new_version_whole() {
    let server = serve().await;
    let job = Job::new();
    let url = server.url("ts/media.m3u8");
    assert!(matches!(job.run_until(&url, 1).await, HlsResult::Paused { .. }));

    // The source is republished with only two segments.
    let shorter = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.000000,\nseg2.ts\n#EXTINF:2.000000,\nseg0.ts\n#EXT-X-ENDLIST\n";
    server.set("ts/media.m3u8", Reply::Body(shorter.as_bytes().to_vec()));
    let output = completed(job.run(&url).await);
    assert_eq!(tree(&output).iter().filter(|f| f.starts_with("video/seg-")).count(), 2);
    assert_eq!(read(&output.join("video/seg-000000.ts")), fixture("ts/seg2.ts"), "the new order, not a stitch of old and new");
    assert_eq!(read(&output.join("video/seg-000001.ts")), fixture("ts/seg0.ts"));
}

#[tokio::test]
async fn protected_live_and_unreadable_streams_are_refused_with_a_reason_and_leave_nothing() {
    let server = serve().await;
    let cases: [(&str, &str, &str); 5] = [
        ("sample.m3u8", "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:2,\nts/seg0.ts\n#EXT-X-ENDLIST\n", "SAMPLE-AES"),
        ("drm.m3u8", "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"k\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"\n#EXTINF:2,\nts/seg0.ts\n#EXT-X-ENDLIST\n", "DRM"),
        ("live.m3u8", "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:100\n#EXTINF:2,\nts/seg0.ts\n#EXTINF:2,\nts/seg1.ts\n", "live"),
        ("html.m3u8", "<html>Access denied</html>", "playlist"),
        ("empty.m3u8", "#EXTM3U\n#EXT-X-ENDLIST\n", "no video"),
    ];
    for (path, body, expected) in cases {
        server.set(path, Reply::Body(body.as_bytes().to_vec()));
        let job = Job::new();
        let error = job.run(&server.url(path)).await.unwrap_err();
        let message = failure(&error).to_string();
        assert!(message.to_lowercase().contains(&expected.to_lowercase()), "{path}: {message}");
        assert!(!job.output().exists() && !job.stage().exists(), "{path}");
    }
    server.set("gone.m3u8", Reply::Status(404));
    let job = Job::new();
    assert_eq!(failure(&job.run(&server.url("gone.m3u8")).await.unwrap_err()), HlsFailure::PlaylistUnavailable);
}

#[tokio::test]
async fn publishing_never_replaces_something_that_is_already_there() {
    let server = serve().await;
    let job = Job::new();
    std::fs::create_dir(job.output()).unwrap();
    std::fs::write(job.output().join("mine.txt"), b"keep me").unwrap();
    assert!(job.run(&server.url("ts/media.m3u8")).await.is_err());
    assert_eq!(read(&job.output().join("mine.txt")), b"keep me");
}

#[tokio::test]
async fn removal_deletes_only_what_the_download_made_and_the_check_reports_damage() {
    let server = serve().await;
    let job = Job::new();
    let output = completed(job.run(&server.url("fmp4/p0.m3u8")).await);
    drop(server);

    // Damage is reported precisely.
    std::fs::remove_file(output.join("video/seg-000001.m4s")).unwrap();
    std::fs::write(output.join("video/seg-000002.m4s"), b"short").unwrap();
    let check = check_folder(&job.root, job.identity(), &job.id, false).unwrap();
    assert!(check.intact_identity && !check.healthy());
    assert_eq!(check.missing, ["video/seg-000001.m4s"]);
    assert_eq!(check.changed, ["video/seg-000002.m4s"]);

    // A file the user added is never deleted, and neither is a downloaded file that was edited
    // since: both stay, and so does the folder around them.
    std::fs::write(output.join("notes.txt"), b"mine").unwrap();
    let report = remove_folder(&job.root, job.identity(), &job.id, true).unwrap();
    assert!(!report.file_deleted && !report.notes.is_empty(), "{report:?}");
    assert_eq!(tree(&output), ["notes.txt", "video/seg-000002.m4s"]);
    assert!(!job.root.join(format!(".cloudstream-{}", job.id)).exists(), "no private bookkeeping is left behind");

    // Without any foreign file the whole folder goes.
    let server = serve().await;
    let job = Job::new();
    let output = completed(job.run(&server.url("ts/media.m3u8")).await);
    let report = remove_folder(&job.root, job.identity(), &job.id, true).unwrap();
    assert!(report.file_deleted && report.temp_cleaned, "{report:?}");
    assert!(!output.exists());
    assert!(std::fs::read_dir(&job.root).unwrap().next().is_none(), "nothing orphaned in the download folder");
}

#[tokio::test]
async fn a_replaced_or_tampered_folder_is_not_trusted_for_deletion() {
    let server = serve().await;
    let job = Job::new();
    let output = completed(job.run(&server.url("ts/media.m3u8")).await);
    // The user swaps the folder for their own directory of the same name.
    let moved = job.root.join("moved");
    std::fs::rename(&output, &moved).unwrap();
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("video.txt"), b"not ours").unwrap();
    assert!(read_manifest(&job.root, job.identity(), &job.id).unwrap().is_none());
    let report = remove_folder(&job.root, job.identity(), &job.id, true).unwrap();
    assert!(!report.file_deleted);
    assert_eq!(read(&output.join("video.txt")), b"not ours");

    // A rewritten manifest (pointing outside the folder) is not believed either.
    let job = Job::new();
    let output = completed(job.run(&server.url("ts/media.m3u8")).await);
    let mut manifest: serde_json::Value = serde_json::from_slice(&read(&output.join("manifest.json"))).unwrap();
    manifest["files"].as_array_mut().unwrap().push(serde_json::json!({"path": "../../escape.txt", "bytes": 1, "sha256": "00"}));
    std::fs::write(output.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(read_manifest(&job.root, job.identity(), &job.id).unwrap().is_none());
}
