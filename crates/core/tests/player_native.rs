//! Real mpv, run headless (`--vo=null --ao=null`), against a local HTTP fixture. Proves the
//! playlist timeline, control state, decoding/cache/compressor options, failover and manual
//! switching on the installed mpv. Skips (loudly) when mpv is not installed.
mod support;

use cloudstream_core::{models::*, player::*, streams, App};
use serde_json::json;
use support::*;

fn skip_without_mpv() -> bool {
    if mpv_installed() {
        false
    } else {
        eprintln!("SKIPPED: mpv is not installed");
        true
    }
}

fn stream(name: &str, url: String, quality: i32) -> Stream {
    Stream { name: name.into(), url, quality, stream_type: "VIDEO".into(), ..Default::default() }
}

fn paused_policy() -> PlaybackPolicy {
    PlaybackPolicy { start_paused: true, ..Default::default() }
}

#[tokio::test]
async fn capabilities_are_probed_from_the_installed_mpv() {
    if skip_without_mpv() {
        return;
    }
    let caps = probe_capabilities().await;
    assert!(caps.available);
    assert!(caps.version.as_deref().is_some_and(|v| v.starts_with("0.")), "{:?}", caps.version);
    for id in ["volume", "mute", "speed", "aspect", "audio_delay", "rotation", "hardware_decoding", "buffer_size", "buffer_time", "audio_compressor", "playlist_timeline"] {
        assert!(caps.supports(id), "{id} should be supported: {:?}", caps.reason(id));
    }
    // mpv has no independent video delay and no size-limited disk cache; say so.
    assert!(!caps.supports("video_delay"));
    assert!(caps.reason("video_delay").unwrap().contains("audio delay"));
    assert!(!caps.supports("buffer_disk"));
}

#[tokio::test]
async fn multi_segment_playlist_plays_as_one_timeline() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/a.wav", wav(1.0)), ("/b.wav", wav(2.0))]).await;
    let directory = tempfile::tempdir().unwrap();
    let player = headless_player(directory.path());
    player.set_policy(paused_policy()).await;
    let playlist = Stream {
        name: "Two segments".into(),
        kind: Some("playlist".into()),
        playlist: Some(vec![
            PlaylistItem { url: server.url("/a.wav"), duration_us: Some(1_000_000), extra: Default::default() },
            PlaylistItem { url: server.url("/b.wav"), duration_us: Some(2_000_000), extra: Default::default() },
        ]),
        ..Default::default()
    };
    player.play(&playlist, &[], None, 0.0, Some("default"), None).await.unwrap();
    let state = wait_for_state(&player, 10, "the whole playlist duration", |s| s.duration > 2.5).await;
    assert!((state.duration - 3.0).abs() < 0.1, "duration {} should span both segments", state.duration);
    assert_eq!(state.stream.as_ref().map(|s| s.name.as_str()), Some("Two segments"));

    // The scrubber can jump into the second segment, which is then actually fetched.
    player.execute(PlayerCommand::SeekAbsolute { seconds: 2.0 }).await.unwrap();
    wait_for_state(&player, 10, "position inside the second segment", |s| s.position >= 1.9).await;
    assert!(server.requested("/b.wav").await, "the second segment was never requested");
    player.stop().await;
}

#[tokio::test]
async fn playlist_with_an_unplayable_segment_is_rejected_before_mpv_starts() {
    if skip_without_mpv() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let player = headless_player(directory.path());
    let playlist = Stream {
        name: "Bad".into(),
        kind: Some("playlist".into()),
        playlist: Some(vec![PlaylistItem { url: "file:///etc/passwd".into(), duration_us: None, extra: Default::default() }]),
        ..Default::default()
    };
    let error = player.play(&playlist, &[], None, 0.0, None, None).await.unwrap_err();
    assert!(error.to_string().contains("segment 1"), "{error}");
    assert!(!player.state().await.active);
}

#[tokio::test]
async fn controls_follow_real_mpv_state_and_bad_values_are_refused() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/long.wav", wav(8.0))]).await;
    let directory = tempfile::tempdir().unwrap();
    let player = headless_player(directory.path());
    player.set_policy(paused_policy()).await;
    player.play(&stream("Long", server.url("/long.wav"), 720), &[], None, 0.0, None, None).await.unwrap();
    wait_for_state(&player, 10, "the file to load", |s| s.duration > 7.0).await;

    let volume = player.execute(PlayerCommand::SetVolume { volume: 55.0 }).await.unwrap();
    assert_eq!(volume.unwrap().volume, 55.0);
    wait_for_state(&player, 5, "volume", |s| (s.volume - 55.0).abs() < 0.1).await;
    player.execute(PlayerCommand::SetMute { muted: true }).await.unwrap();
    wait_for_state(&player, 5, "mute", |s| s.muted).await;
    player.execute(PlayerCommand::SetSpeed { speed: 1.5 }).await.unwrap();
    wait_for_state(&player, 5, "speed", |s| (s.speed - 1.5).abs() < 0.01).await;
    player.execute(PlayerCommand::SetAudioDelay { seconds: 0.25 }).await.unwrap();
    wait_for_state(&player, 5, "audio delay", |s| (s.audio_delay - 0.25).abs() < 0.001).await;
    player.execute(PlayerCommand::SetAspect { mode: AspectMode::Ratio16x9 }).await.unwrap();
    wait_for_state(&player, 5, "16:9", |s| s.aspect == AspectMode::Ratio16x9).await;
    player.execute(PlayerCommand::SetAspect { mode: AspectMode::Stretch }).await.unwrap();
    wait_for_state(&player, 5, "stretch", |s| s.aspect == AspectMode::Stretch).await;
    player.execute(PlayerCommand::SetAspect { mode: AspectMode::Fit }).await.unwrap();
    wait_for_state(&player, 5, "fit", |s| s.aspect == AspectMode::Fit).await;
    player.execute(PlayerCommand::SetRotation { degrees: 90 }).await.unwrap();
    wait_for_state(&player, 5, "rotation", |s| s.rotation == 90).await;
    player.execute(PlayerCommand::SeekAbsolute { seconds: 3.0 }).await.unwrap();
    wait_for_state(&player, 5, "seek", |s| (s.position - 3.0).abs() < 0.3).await;
    player.execute(PlayerCommand::Play).await.unwrap();
    wait_for_state(&player, 5, "resume", |s| !s.paused).await;
    player.execute(PlayerCommand::TogglePause).await.unwrap();
    wait_for_state(&player, 5, "toggle pause", |s| s.paused).await;

    let before = player.state().await;
    assert!(player.execute(PlayerCommand::SetVolume { volume: 400.0 }).await.is_err());
    assert!(player.execute(PlayerCommand::SeekAbsolute { seconds: 99.0 }).await.is_err());
    assert!(player.execute(PlayerCommand::SetRotation { degrees: 45 }).await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let after = player.state().await;
    assert_eq!((before.volume, before.rotation), (after.volume, after.rotation));
    player.stop().await;
}

#[tokio::test]
async fn decoding_cache_and_compressor_preferences_reach_mpv() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/long.wav", wav(8.0))]).await;
    let directory = tempfile::tempdir().unwrap();
    let player = headless_player(directory.path());
    player.set_policy(paused_policy()).await;
    player
        .set_preferences(PlayerPreferences {
            decoding: DecodingMode::Software,
            buffer_ram_mb: 64,
            buffer_seconds: 20,
            audio_compressor: true,
            volume: 40.0,
            muted: true,
            ..Default::default()
        })
        .await;
    player.play(&stream("Long", server.url("/long.wav"), 0), &[], None, 0.0, None, None).await.unwrap();
    // Saved volume and mute are applied at start, not just stored.
    wait_for_state(&player, 10, "saved volume", |s| s.duration > 7.0 && (s.volume - 40.0).abs() < 0.1 && s.muted).await;

    let hwdec = player.command(json!(["get_property", "hwdec"])).await.unwrap();
    assert!(hwdec == json!("no") || hwdec == json!(["no"]), "{hwdec}");
    assert_eq!(player.command(json!(["get_property", "demuxer-max-bytes"])).await.unwrap(), json!(64 * 1024 * 1024));
    assert_eq!(player.command(json!(["get_property", "demuxer-readahead-secs"])).await.unwrap(), json!(20.0));
    let filters = player.command(json!(["get_property", "af"])).await.unwrap();
    assert_eq!(filters[0]["name"], "acompressor", "{filters}");

    // Changing preferences mid-playback applies what mpv can change live.
    let mut prefs = player.preferences().await;
    prefs.audio_compressor = false;
    prefs.decoding = DecodingMode::Hardware;
    player.set_preferences(prefs).await;
    assert_eq!(player.command(json!(["get_property", "af"])).await.unwrap(), json!([]));
    let hwdec = player.command(json!(["get_property", "hwdec"])).await.unwrap();
    assert!(hwdec == json!("auto") || hwdec == json!(["auto"]), "{hwdec}");
    player.stop().await;
}

fn headless_app(dir: &std::path::Path) -> std::sync::Arc<App> {
    App::open_with_player(dir.join("data"), dir.join("resources"), headless_player(dir)).unwrap()
}

async fn next_source_change(rx: &mut tokio::sync::broadcast::Receiver<PlayerEvent>) -> (String, String, u32) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(PlayerEvent::SourceChanged { name, reason, attempt, .. })) => return (name, reason, attempt),
            Ok(Ok(other)) => {
                if std::env::var("TRACE_EVENTS").is_ok() {
                    eprintln!("event: {}", serde_json::to_string(&other).unwrap().chars().take(200).collect::<String>());
                }
                continue;
            }
            other => panic!("no source change arrived: {other:?}"),
        }
    }
}

#[tokio::test]
async fn failing_mirror_fails_over_in_ranked_order_and_stays_bounded() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/good.wav", wav(6.0))]).await;
    let dir = tempfile::tempdir().unwrap();
    let app = headless_app(dir.path());
    app.player.set_policy(paused_policy()).await;
    let mirrors = vec![
        stream("Dead 1080", server.url("/dead1.wav"), 1080),
        stream("Dead 720", server.url("/dead2.wav"), 720),
        stream("Alive 480", server.url("/good.wav"), 480),
    ];
    let mut rx = app.player.subscribe();
    app.play_with_mirrors(mirrors[0].clone(), mirrors.clone(), vec![], None, None, 0.0, None, None)
        .await
        .unwrap();
    assert_eq!(next_source_change(&mut rx).await, ("Dead 720".into(), "failover".into(), 1));
    assert_eq!(next_source_change(&mut rx).await, ("Alive 480".into(), "failover".into(), 2));
    let state = wait_for_state(&app.player, 10, "the working mirror", |s| s.duration > 5.0).await;
    assert_eq!(state.stream.unwrap().name, "Alive 480");
    // The failed mirrors are remembered, so ranking flags them for the UI.
    let current = app.player.state().await;
    let ranked = app.rank_streams_for_session(&mirrors, None, current.session_id.as_deref()).await;
    assert!(ranked.iter().find(|r| r.index == 0).unwrap().errored);
    assert!(!ranked.iter().find(|r| r.index == 2).unwrap().errored);
    app.player.stop().await;

    // With only one automatic retry allowed and every other mirror dead, playback ends clearly.
    let mut policy = paused_policy();
    policy.failover_attempts = 1;
    app.player.set_policy(policy).await;
    let dead = vec![
        stream("Dead A", server.url("/x1.wav"), 1080),
        stream("Dead B", server.url("/x2.wav"), 720),
        stream("Dead C", server.url("/x3.wav"), 480),
    ];
    let mut rx = app.player.subscribe();
    app.play_with_mirrors(dead[0].clone(), dead.clone(), vec![], None, None, 0.0, None, None)
        .await
        .unwrap();
    assert_eq!(next_source_change(&mut rx).await.2, 1);
    let message = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await {
            Ok(Ok(PlayerEvent::Error { message, .. })) => break message,
            Ok(Ok(_)) => continue,
            other => panic!("no terminal error: {other:?}"),
        }
    };
    assert!(message.contains("2 sources failed"), "{message}");
    wait_for_state(&app.player, 5, "the idle mpv to be closed", |s| !s.active).await;
}

#[tokio::test]
async fn failover_can_be_turned_off() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/good.wav", wav(4.0))]).await;
    let dir = tempfile::tempdir().unwrap();
    let app = headless_app(dir.path());
    let mut policy = paused_policy();
    policy.failover_attempts = 0;
    app.player.set_policy(policy).await;
    let mirrors = vec![stream("Dead", server.url("/dead.wav"), 1080), stream("Alive", server.url("/good.wav"), 480)];
    let mut rx = app.player.subscribe();
    app.play_with_mirrors(mirrors[0].clone(), mirrors, vec![], None, None, 0.0, None, None).await.unwrap();
    let message = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv()).await {
            Ok(Ok(PlayerEvent::Error { message, .. })) => break message,
            Ok(Ok(PlayerEvent::SourceChanged { .. })) => panic!("failover ran although it is off"),
            Ok(Ok(_)) => continue,
            other => panic!("no error: {other:?}"),
        }
    };
    assert!(message.contains("Dead failed to play") && message.contains("Choose another source"), "{message}");
}

#[tokio::test]
async fn manual_switch_keeps_the_position() {
    if skip_without_mpv() {
        return;
    }
    let server = serve(vec![("/one.wav", wav(8.0)), ("/two.wav", wav(8.0))]).await;
    let dir = tempfile::tempdir().unwrap();
    let app = headless_app(dir.path());
    app.player.set_policy(paused_policy()).await;
    let one = stream("Mirror one", server.url("/one.wav"), 720);
    let two = stream("Mirror two", server.url("/two.wav"), 1080);
    let mut rx = app.player.subscribe();
    let first = app.play_with_mirrors(one.clone(), vec![one, two.clone()], vec![], None, None, 3.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "resume position", |s| s.duration > 7.0 && (s.position - 3.0).abs() < 0.3).await;

    app.player.execute(PlayerCommand::SeekAbsolute { seconds: 4.5 }).await.unwrap();
    wait_for_state(&app.player, 5, "seek", |s| (s.position - 4.5).abs() < 0.3).await;
    let second = app.switch_stream(two, None).await.unwrap();
    assert_ne!(first, second, "a switch is a new mpv session, so stale events are ignored");
    assert_eq!(next_source_change(&mut rx).await, ("Mirror two".into(), "manual".into(), 0));
    let state = wait_for_state(&app.player, 10, "the new mirror at the old position", |s| {
        s.session_id.as_deref() == Some(second.as_str()) && s.duration > 7.0 && (s.position - 4.5).abs() < 0.5
    })
    .await;
    assert_eq!(state.stream.unwrap().name, "Mirror two");
    assert!(server.requested("/two.wav").await);
    // Nothing to switch when nothing plays.
    app.player.stop().await;
    let error = app.switch_stream(stream("x", server.url("/one.wav"), 0), None).await.unwrap_err();
    assert!(error.to_string().contains("Nothing is playing"), "{error}");
}

#[test]
fn stream_identity_is_stable_for_the_same_mirror() {
    let a = stream("A", "https://x.example/a".into(), 720);
    assert_eq!(streams::stream_key(&a), streams::stream_key(&a.clone()));
    assert_ne!(streams::stream_key(&a), streams::stream_key(&stream("A", "https://x.example/b".into(), 720)));
}

#[tokio::test]
async fn failed_mirrors_are_scoped_to_the_active_player_session_and_profile() {
    if skip_without_mpv() { return; }
    let server = serve(vec![("/track.wav", wav(4.0))]).await;
    let dir = tempfile::tempdir().unwrap();
    let app = headless_app(dir.path());
    app.player.set_policy(paused_policy()).await;
    let link = stream("Mirror", server.url("/track.wav"), 720);
    let session = app.play(link.clone(), vec![], None, None, 0.0, Some("alice"), None).await.unwrap();
    wait_for_state(&app.player, 10, "active playback", |s| s.duration > 3.0).await;
    app.store.update_settings(json!({"sourcePriority": {"activeProfileId":"default", "profiles":[{
        "id":"default", "name":"Default", "hideErrorSources":true
    }]}}), Some("profile"), Some("alice")).unwrap();
    app.playback_context.lock().await.as_mut().unwrap().errored.insert(streams::stream_key(&link));
    let fresh = app.rank_streams(&[link.clone()], Some("alice")).await;
    assert!(fresh[0].usable && !fresh[0].errored, "new episodes must not inherit old errors");
    let other_session = app.rank_streams_for_session(&[link.clone()], Some("alice"), Some("old-session")).await;
    assert!(other_session[0].usable && !other_session[0].errored);
    let other_profile = app.rank_streams_for_session(&[link.clone()], Some("bob"), Some(&session)).await;
    assert!(other_profile[0].usable && !other_profile[0].errored);
    let current = app.rank_streams_for_session(&[link], Some("alice"), Some(&session)).await;
    assert!(!current[0].usable && current[0].errored);
    app.player.stop().await;
}

#[tokio::test]
async fn next_episode_resolves_through_worker_and_updates_queue() {
    if skip_without_mpv() { return; }
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open_with_player(dir.path().join("data"), resources.clone(), headless_player(dir.path())).unwrap();
    app.install_local(&resources.join("worker/desktop-sample.jar")).await.unwrap();
    let provider = app.providers().await.unwrap().into_iter().next().unwrap();
    let media = MediaItem { provider_id: provider.id, name: "Sample".into(), url: "https://example.com/sample".into(), ..Default::default() };
    let episodes = vec![
        Episode { name: "First".into(), data: "first".into(), ..Default::default() },
        Episode { name: "Next".into(), data: "https://download.blender.org/peach/trailer/trailer_480p.mov".into(), ..Default::default() },
    ];
    app.player.set_queue(Some(PlaybackQueue { profile_id: "default".into(), media, episodes, current_index: 0, dub_status: None, season: None, extra: Default::default() })).await;
    app.player.set_policy(paused_policy()).await;
    assert!(app.advance_queue_and_play(true).await.unwrap());
    assert_eq!(app.player.queue().await.unwrap().current_index, 1);
    assert_eq!(app.player.state().await.episode_name.as_deref(), Some("Next"));
    app.player.stop().await;
}

#[tokio::test]
async fn native_audio_and_external_subtitle_tracks_can_be_selected_without_restart() {
    if skip_without_mpv() { return; }
    let server = serve(vec![
        ("/track.wav", wav(6.0)),
        ("/captions.vtt", "WEBVTT\n\n00:00:00.000 --> 00:00:03.000\nمرحبا — Hello\n".as_bytes().to_vec()),
    ]).await;
    let dir = tempfile::tempdir().unwrap();
    let player = headless_player(dir.path());
    player.set_policy(paused_policy()).await;
    let sub = Subtitle { lang: "English captions".into(), lang_tag: Some("en".into()), url: server.url("/captions.vtt"), ..Default::default() };
    let session = player.play(&stream("Video", server.url("/track.wav"), 720), &[sub], None, 0.0, None, None).await.unwrap();
    let state = wait_for_state(&player, 10, "audio and external subtitle tracks", |s| s.tracks.iter().any(|t| t.kind == "sub") && s.tracks.iter().any(|t| t.kind == "audio")).await;
    let subtitle = state.tracks.iter().find(|t| t.kind == "sub").unwrap();
    assert!(subtitle.external);
    assert_eq!(subtitle.language.as_deref(), Some("en"));
    assert_eq!(subtitle.title.as_deref(), Some("English captions"));
    let id = subtitle.id;
    player.execute(PlayerCommand::SetSubtitleTrack { id: Some(id) }).await.unwrap();
    wait_for_state(&player, 5, "selected subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.id == id && t.selected)).await;
    player.execute(PlayerCommand::SetSubtitleTrack { id: None }).await.unwrap();
    let state = wait_for_state(&player, 5, "subtitles off", |s| s.tracks.iter().all(|t| t.kind != "sub" || !t.selected)).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    let carry = TrackChoice { subtitles_enabled: Some(true), subtitle_language: Some("en".into()), ..Default::default() };
    player.play_with(&stream("Mirror", server.url("/track.wav"), 720), &[Subtitle { lang: "English captions".into(), lang_tag: Some("en".into()), url: server.url("/captions.vtt"), ..Default::default() }], None, 1.0, None, None, &carry).await.unwrap();
    let state = wait_for_state(&player, 10, "selected subtitle after mirror switch", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected && t.language.as_deref() == Some("en"))).await;
    assert!(state.position >= 0.9);
    let carry_off = TrackChoice { subtitles_enabled: Some(false), ..Default::default() };
    player.play_with(&stream("Mirror off", server.url("/track.wav"), 720), &[Subtitle { lang: "English captions".into(), lang_tag: Some("en".into()), url: server.url("/captions.vtt"), ..Default::default() }], None, 1.0, None, None, &carry_off).await.unwrap();
    let state = wait_for_state(&player, 10, "subtitle available after switch with subtitles off", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert!(state.tracks.iter().all(|t| t.kind != "sub" || !t.selected));
    player.stop().await;
}

#[tokio::test]
async fn provider_resources_use_isolated_headers_and_close_with_playback() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::sync::Arc;
    if skip_without_mpv() { return; }
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let fixture_source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/provider/FixturePlugin.java");
    let classes = dir.path().join("classes");
    std::fs::create_dir(&classes).unwrap();
    let javac = std::process::Command::new("javac").args(["--release", "21"]).arg("-cp").arg(resources.join("worker/worker.jar"))
        .arg("-d").arg(&classes).arg(fixture_source).output().expect("JDK javac is needed for the provider fixture");
    assert!(javac.status.success(), "javac: {}", String::from_utf8_lossy(&javac.stderr));
    let jar_path = dir.path().join("fixture.jar");
    let jar = std::process::Command::new("jar").arg("cf").arg(&jar_path).arg("-C").arg(&classes).arg(".").output().unwrap();
    assert!(jar.status.success(), "jar: {}", String::from_utf8_lossy(&jar.stderr));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 16384];
                let count = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..count]).to_string();
                recorded.lock().await.push(request.clone());
                let path = request.lines().next().and_then(|line| line.split(' ').nth(1)).unwrap_or("").split('?').next().unwrap_or("");
                let expected = match path { "/video.wav" => "video", "/audio.wav" => "audio", "/sub.vtt" => "subtitle", "/subs.zip" => "archive", _ => "unknown" };
                let allowed = request.lines().any(|line| line.trim().eq_ignore_ascii_case(&format!("x-resource: {expected}")));
                if !allowed || expected == "unknown" {
                    let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                }
                let bytes = if expected == "subtitle" { b"WEBVTT\n\n00:00:00.000 --> 00:00:03.000\nHello\n".to_vec() } else if expected == "archive" {
                    use std::io::Write;
                    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                    zip.start_file("English.srt", zip::write::SimpleFileOptions::default()).unwrap();
                    zip.write_all(b"1\n00:00:00,000 --> 00:00:03,000\nFrom archive\n").unwrap();
                    zip.finish().unwrap().into_inner()
                } else { wav(6.0) };
                let start = request.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").and_then(|r| r.split('-').next()?.parse::<usize>().ok()));
                let start = start.unwrap_or(0).min(bytes.len() - 1);
                let extra = if request.to_ascii_lowercase().contains("\nrange:") { format!("Content-Range: bytes {}-{}/{}\r\n", start, bytes.len()-1, bytes.len()) } else { String::new() };
                let code = if extra.is_empty() { "200 OK" } else { "206 Partial Content" };
                let mime = if expected == "subtitle" { "text/vtt" } else { "audio/wav" };
                let head = format!("HTTP/1.1 {code}\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()-start);
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&bytes[start..]).await;
            });
        }
    });
    let app = App::open_with_player(dir.path().join("data"), resources, headless_player(dir.path())).unwrap();
    app.install_local(&jar_path).await.unwrap();
    let provider = app.providers().await.unwrap().into_iter().next().unwrap();
    let value = app.call("loadLinks", json!({"providerId":provider.id, "data":format!("http://{address}")})).await.unwrap();
    let links: ResolvedLinks = serde_json::from_value(value).unwrap();
    assert!(links.streams[0].audio_tracks[0].resource_handle.is_some());
    assert!(links.subtitles[0].resource_handle.is_some());
    app.player.set_policy(paused_policy()).await;
    let settings = app.store.update_settings(json!({"player":{"subtitleTransforms":{"uppercase":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    let session = app.play(links.streams[0].clone(), links.subtitles.clone(), None, None, 0.0, None, None).await.unwrap();
    let state = wait_for_state(&app.player, 15, "separate protected audio and subtitle tracks", |s| s.tracks.iter().filter(|t| t.kind == "audio").count() >= 2 && s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    let assets = || std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")).map(|e| e.path()).collect::<Vec<_>>();
    let copies = assets();
    assert_eq!(copies.len(), 1);
    assert!(std::fs::read_to_string(copies[0].join("subtitle.vtt")).unwrap().contains("HELLO"));
    let tokens = app.active_transport_token.lock().await.as_ref().unwrap().tokens.clone();
    assert_eq!(tokens.len(), 3);
    let captured = requests.lock().await.clone();
    for (path, value) in [("/video.wav", "video"), ("/audio.wav", "audio"), ("/sub.vtt", "subtitle")] {
        let request = captured.iter().find(|r| r.lines().next().is_some_and(|line| line.contains(path))).unwrap_or_else(|| panic!("missing request for {path}"));
        let resource_headers: Vec<_> = request.lines().filter(|l| l.to_ascii_lowercase().starts_with("x-resource:")).collect();
        assert_eq!(resource_headers.len(), 1, "{request}");
        assert!(resource_headers[0].trim().eq_ignore_ascii_case(&format!("x-resource: {value}")), "{request}");
    }
    assert!(app.prepare_provider_subtitle_archive("old-session", 1).await.is_err());
    assert!(app.prepare_provider_subtitle_archive(&session, 0).await.unwrap_err().to_string().contains("not a ZIP"));
    assert!(app.prepare_provider_subtitle_archive(&session, 99).await.is_err());
    app.playback_context.lock().await.as_mut().unwrap().media_id = Some("different-episode".into());
    assert!(app.prepare_provider_subtitle_archive(&session, 1).await.is_err());
    app.playback_context.lock().await.as_mut().unwrap().media_id = None;
    let archive = app.prepare_provider_subtitle_archive(&session, 1).await.unwrap();
    let cloudstream_core::SubtitleFileSelection::Archive { id, entries } = archive else { panic!("expected archive") };
    assert_eq!(entries.len(), 1);
    app.select_archived_subtitle(&id, entries[0].index).await.unwrap();
    wait_for_state(&app.player, 10, "selected provider archive subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected && t.title.as_deref() == Some("English.srt"))).await;
    let captured = requests.lock().await.clone();
    let archive_request = captured.iter().find(|r| r.lines().next().is_some_and(|line| line.contains("/subs.zip?token=opaque"))).expect("archive request must preserve query");
    assert!(archive_request.lines().any(|line| line.trim().eq_ignore_ascii_case("x-resource: archive")));
    assert!(!archive_request.lines().any(|line| line.trim().eq_ignore_ascii_case("x-resource: video")));
    app.player.stop().await;
    assert!(assets().is_empty());
    app.close_transport_for(&session).await;
    assert!(app.active_transport_token.lock().await.is_none());
    let installed = app.store.installed().unwrap();
    for token in tokens { assert!(!app.worker.close_media_session(&installed, &token).await.unwrap()); }
    server.abort();
}

#[tokio::test]
async fn subtitle_preference_and_delay_survive_reopen_and_apply_without_restart() {
    if skip_without_mpv() { return; }
    let server = serve(vec![("/track.wav", wav(6.0)), ("/en.vtt", b"WEBVTT\n\n00:00:00.000 --> 00:00:04.000\nHello\n".to_vec())]).await;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let resources = dir.path().join("resources");
    let app = App::open_with_player(data.clone(), resources.clone(), headless_player(dir.path())).unwrap();
    app.player.set_policy(paused_policy()).await;
    let video = stream("Video", server.url("/track.wav"), 720);
    let subtitle = Subtitle { lang: "English".into(), lang_tag: Some("en".into()), url: server.url("/en.vtt"), ..Default::default() };
    let session = app.play(video.clone(), vec![subtitle.clone()], None, None, 0.0, None, None).await.unwrap();
    let state = wait_for_state(&app.player, 10, "subtitle track", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    let subtitle_id = state.tracks.iter().find(|t| t.kind == "sub").unwrap().id;
    app.player_command(PlayerCommand::SetSubtitleTrack { id: Some(subtitle_id) }).await.unwrap();
    let prefs = app.store.get_settings().unwrap().global.player;
    assert_eq!(prefs.subtitles_enabled, Some(true));
    assert_eq!(prefs.subtitle_language.as_deref(), Some("en"));
    app.player_command(PlayerCommand::SetSubtitleTrack { id: None }).await.unwrap();
    app.player_command(PlayerCommand::SetSubtitleDelay { seconds: 1.25 }).await.unwrap();
    let state = wait_for_state(&app.player, 5, "subtitle delay", |s| (s.subtitle_delay - 1.25).abs() < 0.001).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    app.player.stop().await;

    let reopened = App::open_with_player(data, resources, headless_player(dir.path())).unwrap();
    assert_eq!(reopened.player.preferences().await.subtitles_enabled, Some(false));
    assert_eq!(reopened.player.preferences().await.subtitle_delay, 1.25);
    reopened.player.set_policy(paused_policy()).await;
    reopened.play(video.clone(), vec![subtitle.clone()], None, None, 0.0, None, None).await.unwrap();
    let state = wait_for_state(&reopened.player, 10, "subtitle off after restart", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert!(state.tracks.iter().all(|t| t.kind != "sub" || !t.selected));
    assert!((state.subtitle_delay - 1.25).abs() < 0.001);
    reopened.player.stop().await;

    let settings = reopened.store.update_settings(json!({"player": {"subtitlesEnabled": true, "subtitleLanguage": "en"}}), Some("global"), None).unwrap();
    reopened.apply_player_settings(&settings).await;
    reopened.play(video, vec![subtitle], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&reopened.player, 10, "preferred external subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected && t.language.as_deref() == Some("en"))).await;
    reopened.player.stop().await;
}

#[tokio::test]
async fn local_subtitle_with_unicode_and_spaces_loads_without_modifying_source_or_restarting() {
    if skip_without_mpv() { return; }
    let server = serve(vec![("/video.wav", wav(8.0))]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("字幕 عربي sample.srt");
    let original = "1\n00:00:00,000 --> 00:00:04,000\nمرحبا — こんにちは\n".as_bytes();
    std::fs::write(&path, original).unwrap();
    let player = headless_player(dir.path());
    player.set_policy(paused_policy()).await;
    let session = player.play(&stream("Local", server.url("/video.wav"), 720), &[], None, 0.0, None, None).await.unwrap();
    wait_for_state(&player, 10, "loaded video", |s| s.duration > 7.0).await;
    player.execute(PlayerCommand::SeekAbsolute { seconds: 2.0 }).await.unwrap();
    wait_for_state(&player, 5, "seek position", |s| s.position > 1.9).await;
    assert!(player.add_local_subtitle(&path, "old-session").await.unwrap_err().to_string().contains("Playback changed"));
    player.add_local_subtitle(&path, &session).await.unwrap();
    let state = wait_for_state(&player, 10, "selected local subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected && t.external)).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    assert!(state.position > 1.9);
    assert!(state.tracks.iter().any(|t| t.title.as_deref() == Some("字幕 عربي sample.srt")));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let bad = dir.path().join("not a subtitle.txt");
    std::fs::write(&bad, b"not a subtitle").unwrap();
    assert!(player.add_local_subtitle(&bad, &session).await.unwrap_err().to_string().contains("Choose an SRT"));
    let huge = dir.path().join("huge.srt");
    std::fs::File::create(&huge).unwrap().set_len(20 * 1024 * 1024 + 1).unwrap();
    assert!(player.add_local_subtitle(&huge, &session).await.unwrap_err().to_string().contains("20 MiB"));
    player.stop().await;
}

#[tokio::test]
async fn subtitle_style_applies_live_and_persists_without_restarting_playback() {
    if skip_without_mpv() { return; }
    let server = serve(vec![("/video.wav", wav(8.0)), ("/sub.vtt", b"WEBVTT\n\n00:00:00.000 --> 00:00:05.000\nHello\n".to_vec())]).await;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let resources = dir.path().join("resources");
    let app = App::open_with_player(data.clone(), resources.clone(), headless_player(dir.path())).unwrap();
    assert!(app.player.capabilities().await.supports("subtitle_style"), "installed mpv lacks subtitle style options");
    app.player.set_policy(paused_policy()).await;
    let video = stream("Style", server.url("/video.wav"), 720);
    let sub = Subtitle { lang: "English".into(), lang_tag: Some("en".into()), url: server.url("/sub.vtt"), ..Default::default() };
    let session = app.play(video.clone(), vec![sub.clone()], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    app.player.execute(PlayerCommand::SeekAbsolute { seconds: 2.0 }).await.unwrap();
    wait_for_state(&app.player, 5, "seek", |s| s.position > 1.9).await;
    let settings = app.store.update_settings(json!({"player":{"subtitleStyle":{
        "font":"DejaVu Sans", "size":55.0, "bold":true, "italic":true,
        "color":"#ffeeaa", "outlineSize":3.0, "backgroundBox":true,
        "backgroundOpacity":50, "verticalMargin":60, "overrideAss":false
    }}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    assert_eq!(app.player.command(json!(["get_property", "sub-font"])).await.unwrap(), json!("DejaVu Sans"));
    assert_eq!(app.player.command(json!(["get_property", "sub-font-size"])).await.unwrap(), json!(55.0));
    assert_eq!(app.player.command(json!(["get_property", "sub-bold"])).await.unwrap(), json!(true));
    assert_eq!(app.player.command(json!(["get_property", "sub-italic"])).await.unwrap(), json!(true));
    assert_eq!(app.player.command(json!(["get_property", "sub-border-style"])).await.unwrap(), json!("background-box"));
    let ass_override = app.player.command(json!(["get_property", "sub-ass-override"])).await.unwrap();
    assert!(ass_override == json!("no") || ass_override == json!(false), "{ass_override}");
    let state = app.player.state().await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    assert!(state.position > 1.9);
    app.player.stop().await;
    let reopened = App::open_with_player(data, resources, headless_player(dir.path())).unwrap();
    assert_eq!(reopened.player.preferences().await.subtitle_style.size, 55.0);
    assert!(reopened.player.preferences().await.subtitle_style.background_box);
    reopened.player.set_policy(paused_policy()).await;
    reopened.play(video, vec![sub], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&reopened.player, 10, "subtitle after restart", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert_eq!(reopened.player.command(json!(["get_property", "sub-font-size"])).await.unwrap(), json!(55.0));
    let settings = reopened.store.update_settings(json!({"player":{"subtitleStyle":{"overrideAss":true}}}), Some("global"), None).unwrap();
    reopened.apply_player_settings(&settings).await;
    assert_eq!(reopened.player.command(json!(["get_property", "sub-ass-override"])).await.unwrap(), json!("force"));
    let settings = reopened.store.update_settings(json!({"player":{"subtitleStyle":SubtitleStyle::default()}}), Some("global"), None).unwrap();
    reopened.apply_player_settings(&settings).await;
    assert_eq!(reopened.player.command(json!(["get_property", "sub-font-size"])).await.unwrap(), json!(38.0));
    assert_eq!(reopened.player.command(json!(["get_property", "sub-ass-override"])).await.unwrap(), json!("scale"));
    reopened.player.stop().await;
}
