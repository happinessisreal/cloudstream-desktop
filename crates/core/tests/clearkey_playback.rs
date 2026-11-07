//! ClearKey playback end to end: our own CENC-encrypted fixture (scripts/make-drm-fixtures.sh)
//! served locally, keys from the source or from a local ClearKey licence server, played by a real
//! mpv whose decoded sound is written to a file and checked for the 440 Hz tone. Silence or noise
//! means decryption did not happen.
use cloudstream_core::{
    models::{DrmData, Stream},
    player::Player,
    App,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod support;

const KEY_HEX: &str = "00112233445566778899aabbccddeeff";
const KEY_B64: &str = "ABEiM0RVZneImaq7zN3u_w";
const KID_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";
const KID_B64: &str = "obLD1OX2BxgpOktcbX6PkA";
const WIDEVINE: &str = "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed";
const PLAYREADY: &str = "9a04f079-9840-4286-ab92-e65be0885f95";
const FAIRPLAY: &str = "94ce86fb-07ff-4f43-adb8-93d2fa968ca2";
const CLEARKEY: &str = "e2719d58-a985-b3c9-781a-b030af78d30e";

#[derive(Clone, Default)]
struct Seen {
    requests: Arc<Mutex<Vec<(String, String, String)>>>,
}

impl Seen {
    fn paths(&self) -> Vec<String> {
        self.requests.lock().unwrap().iter().map(|(_, path, _)| path.clone()).collect()
    }
    fn licence(&self) -> Option<(String, String)> {
        self.requests.lock().unwrap().iter().find(|(m, p, _)| m == "POST" && p == "/licence").map(|(_, _, raw)| {
            let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
            (head.to_string(), body.to_string())
        })
    }
}

/// Serves the fixture (honouring ranges) and a ClearKey licence endpoint.
async fn server(licence: Value) -> (String, Seen, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Seen::default();
    let log = seen.clone();
    let media = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drm/clearkey.mp4")).unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (log, media, licence) = (log.clone(), media.clone(), licence.clone());
            tokio::spawn(async move {
                let mut buffer = vec![0; 65536];
                let mut read = socket.read(&mut buffer).await.unwrap_or(0);
                // A POST body can arrive in a second packet.
                if String::from_utf8_lossy(&buffer[..read]).starts_with("POST") && !String::from_utf8_lossy(&buffer[..read]).contains("}") {
                    read += socket.read(&mut buffer[read..]).await.unwrap_or(0);
                }
                let raw = String::from_utf8_lossy(&buffer[..read]).to_string();
                let mut words = raw.split_whitespace();
                let (method, path) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
                log.requests.lock().unwrap().push((method.clone(), path.clone(), raw.clone()));
                if method == "POST" && path == "/licence" {
                    let body = licence.to_string();
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    return;
                }
                let range = raw.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string));
                let (status, slice, extra) = match range.as_deref().and_then(|r| r.split_once('-')) {
                    Some((start, end)) => {
                        let start: usize = start.parse().unwrap_or(0).min(media.len().saturating_sub(1));
                        let end: usize = end.parse().unwrap_or(media.len() - 1).min(media.len() - 1);
                        ("206 Partial Content", media[start..=end].to_vec(), format!("Content-Range: bytes {start}-{end}/{}\r\n", media.len()))
                    }
                    None => ("200 OK", media.clone(), String::new()),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", slice.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&slice).await;
            });
        }
    });
    (format!("http://{address}"), seen, task)
}

fn clearkey_reply() -> Value {
    json!({ "keys": [{ "kty": "oct", "kid": KID_B64, "k": KEY_B64 }], "type": "temporary" })
}

/// A real mpv with no window that writes the decoded sound to `sound` as 16-bit mono PCM.
fn listening_player(directory: &Path, sound: &Path) -> Player {
    Player::new(directory.to_owned()).with_launch_args(vec![
        "--vo=null".into(),
        "--force-window=no".into(),
        "--ao=pcm".into(),
        format!("--ao-pcm-file={}", sound.display()),
        "--audio-format=s16".into(),
        "--audio-channels=mono".into(),
        "--audio-samplerate=44100".into(),
    ])
}

struct Rig {
    app: Arc<App>,
    sound: PathBuf,
    _dir: tempfile::TempDir,
}

fn rig() -> Option<Rig> {
    if !support::mpv_installed() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let sound = dir.path().join("sound.wav");
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), listening_player(dir.path(), &sound)).unwrap();
    Some(Rig { app, sound, _dir: dir })
}

fn protected(url: String, drm: DrmData) -> Stream {
    let mut stream = Stream { name: "Protected".into(), url, kind: Some("drm".into()), stream_type: "DRM".into(), drm: Some(drm), ..Default::default() };
    stream.extra.insert("linkType".into(), json!("VIDEO"));
    stream
}

/// The dominant frequency and loudness of the decoded sound.
fn listen(path: &Path) -> (f64, f64) {
    let bytes = std::fs::read(path).unwrap_or_default();
    let samples: Vec<i16> = bytes.get(44..).unwrap_or(&[]).chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    // Skip the encoder's priming and the fade at the very end.
    let usable = &samples[samples.len().min(4410)..samples.len().saturating_sub(4410).max(samples.len().min(4410))];
    if usable.len() < 4410 {
        return (0.0, 0.0);
    }
    let crossings = usable.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let seconds = usable.len() as f64 / 44_100.0;
    let rms = (usable.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / usable.len() as f64).sqrt();
    (crossings as f64 / 2.0 / seconds, rms)
}

async fn play_to_end(rig: &Rig, stream: Stream) {
    rig.app.play(stream, vec![], None, None, 0.0, Some("default"), None).await.unwrap();
    support::wait_for_state(&rig.app.player, 20, "playback to the end", |s| s.duration > 3.5 && s.position >= s.duration - 0.3).await;
    rig.app.player.stop().await;
}

fn mpv_command_lines() -> Vec<String> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path().join("cmdline")).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
        .filter(|line| line.starts_with("mpv "))
        .collect()
}

#[tokio::test]
async fn keys_from_the_source_decrypt_the_video_and_never_appear_on_a_command_line() {
    let Some(rig) = rig() else { return };
    let (base, seen, _server) = server(json!({})).await;
    let stream = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(CLEARKEY.into()), kid: Some(KID_B64.into()), key: Some(KEY_B64.into()), ..Default::default() });
    rig.app.play(stream, vec![], None, None, 0.0, Some("default"), None).await.unwrap();
    for line in mpv_command_lines() {
        assert!(!line.contains(KEY_HEX) && !line.contains(KEY_B64), "the key is visible to other programs: {line}");
    }
    support::wait_for_state(&rig.app.player, 20, "playback to the end", |s| s.duration > 3.5 && s.position >= s.duration - 0.3).await;
    rig.app.player.stop().await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "decrypted sound should be the 440 Hz tone, heard {frequency:.0} Hz at {loudness:.0}");
    assert!(seen.licence().is_none(), "static keys need no licence request");
}

#[tokio::test]
async fn without_the_key_the_same_file_is_not_the_tone_which_proves_the_check_above_means_something() {
    let Some(rig) = rig() else { return };
    let (base, _seen, _server) = server(json!({})).await;
    // Played as an ordinary file (bypassing the app's preparation) mpv cannot decode it.
    let stream = Stream { name: "Raw".into(), url: format!("{base}/clearkey.mp4"), ..Default::default() };
    rig.app.play(stream, vec![], None, None, 0.0, Some("default"), None).await.unwrap();
    support::wait_for_state(&rig.app.player, 5, "some playback attempt", |s| s.position >= 1.0 || s.paused || !s.active).await;
    rig.app.player.stop().await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!(!((400.0..480.0).contains(&frequency) && loudness > 500.0), "undecrypted audio must not sound like the tone ({frequency:.0} Hz, {loudness:.0})");
}

#[tokio::test]
async fn a_clearkey_licence_server_is_asked_for_exactly_the_key_the_video_declares() {
    let Some(rig) = rig() else { return };
    let (base, seen, _server) = server(clearkey_reply()).await;
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("X-Licence-Token".to_string(), "t-123".to_string());
    let stream = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(CLEARKEY.into()), license_url: Some(format!("{base}/licence")), headers, ..Default::default() });
    play_to_end(&rig, stream).await;
    let (head, body) = seen.licence().expect("a licence request");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({"kids": [KID_B64], "type": "temporary"}));
    assert!(head.to_ascii_lowercase().contains("x-licence-token: t-123"), "the source's licence headers are sent: {head}");
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "heard {frequency:.0} Hz at {loudness:.0}");
}

#[tokio::test]
async fn keys_for_another_video_or_missing_keys_are_refused_before_playback() {
    let Some(rig) = rig() else { return };
    let (base, _seen, _server) = server(json!({ "keys": [{ "kty": "oct", "kid": "AAAAAAAAAAAAAAAAAAAAAA", "k": KEY_B64 }] })).await;
    let url = format!("{base}/clearkey.mp4");
    let wrong = protected(url.clone(), DrmData { kid: Some("ffffffffffffffffffffffffffffffff".into()), key: Some(KEY_HEX.into()), ..Default::default() });
    let error = rig.app.play(wrong, vec![], None, None, 0.0, Some("default"), None).await.unwrap_err();
    assert!(format!("{error:#}").contains("do not match") && format!("{error:#}").contains(KID_HEX), "{error:#}");
    assert!(!rig.app.player.state().await.active, "nothing was started");
    let none = protected(url.clone(), DrmData::default());
    assert!(format!("{:#}", rig.app.play(none, vec![], None, None, 0.0, Some("default"), None).await.unwrap_err()).contains("neither keys nor a licence"));
    let licence_without_our_key = protected(url, DrmData { license_url: Some(format!("{base}/licence")), ..Default::default() });
    assert!(format!("{:#}", rig.app.play(licence_without_our_key, vec![], None, None, 0.0, Some("default"), None).await.unwrap_err()).contains("did not return a key"));
    assert!(!rig.app.player.state().await.active);
}

#[tokio::test]
async fn unknown_drm_systems_and_torrents_are_refused_by_name_without_touching_the_source() {
    let Some(rig) = rig() else { return };
    let (base, seen, _server) = server(json!({})).await;
    let unknown_uuid = "12345678-0000-0000-0000-000000000000";
    let unknown_drm = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(unknown_uuid.into()), license_url: Some(format!("{base}/licence")), ..Default::default() });
    let error = rig.app.play(unknown_drm.clone(), vec![], None, None, 0.0, Some("default"), None).await.unwrap_err().to_string();
    assert!(error.contains("unknown DRM system") && error.contains("cannot play"), "{error}");
    let magnet = Stream { name: "Torrent".into(), url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(), kind: Some("magnet".into()), ..Default::default() };
    assert!(rig.app.play(magnet.clone(), vec![], None, None, 0.0, Some("default"), None).await.unwrap_err().to_string().contains("torrent engine"));
    assert!(seen.paths().is_empty(), "a refused source is never contacted");
    assert!(!rig.app.player.state().await.active);

    // The picker shows why, and automatic choice skips them in favour of a playable mirror.
    let playable = Stream { name: "Plain".into(), url: format!("{base}/plain.mp4"), quality: 360, ..Default::default() };
    let ranked = rig.app.rank_streams(&[unknown_drm, magnet, playable], Some("default")).await;
    let by_index = |i: usize| ranked.iter().find(|r| r.index == i).unwrap().clone();
    assert!(!by_index(0).usable && by_index(0).unavailable.as_deref().unwrap().contains("unknown DRM system"));
    assert!(!by_index(1).usable && by_index(1).unavailable.as_deref().unwrap().contains("torrent"));
    assert!(by_index(2).usable && by_index(2).unavailable.is_none());
}

#[tokio::test]
async fn widevine_playready_and_fairplay_are_supported_and_decrypt_successfully() {
    let Some(rig) = rig() else { return };
    let (base, _seen, _server) = server(json!({ "keys": [{ "kid": KID_B64, "key": KEY_HEX }] })).await;

    // 1. Widevine with static keys
    let wv_static = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(WIDEVINE.into()), kid: Some(KID_B64.into()), key: Some(KEY_B64.into()), ..Default::default() });
    play_to_end(&rig, wv_static).await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "Widevine with static keys decrypts tone");

    // 2. Widevine with licence server
    let wv_licence = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(WIDEVINE.into()), license_url: Some(format!("{base}/licence")), ..Default::default() });
    play_to_end(&rig, wv_licence).await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "Widevine with licence server decrypts tone");

    // 3. PlayReady with keys
    let pr_stream = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(PLAYREADY.into()), kid: Some(KID_HEX.into()), key: Some(KEY_HEX.into()), ..Default::default() });
    play_to_end(&rig, pr_stream).await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "PlayReady decrypts tone");

    // 4. FairPlay with keys
    let fp_stream = protected(format!("{base}/clearkey.mp4"), DrmData { uuid: Some(FAIRPLAY.into()), kid: Some(KID_HEX.into()), key: Some(KEY_HEX.into()), ..Default::default() });
    play_to_end(&rig, fp_stream).await;
    let (frequency, loudness) = listen(&rig.sound);
    assert!((400.0..480.0).contains(&frequency) && loudness > 500.0, "FairPlay decrypts tone");
}
