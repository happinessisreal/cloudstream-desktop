mod support;
use cloudstream_core::{models::*, player::Player, subtitles, App, SubtitleFileSelection};
use std::io::{Cursor, Write};
use support::*;

fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn zip_inspection_rejects_unsafe_entries_and_extracts_only_selected_subtitle() {
    let bytes = archive(&[("notes.txt", b"ignore"), ("English/字幕.srt", b"caption one"), ("Arabic.vtt", b"caption two")]);
    let list = subtitles::list_archive(&bytes).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, "English/字幕.srt");
    let (entry, content) = subtitles::extract_archive_entry(&bytes, list[1].index).unwrap();
    assert_eq!(entry.name, "Arabic.vtt");
    assert_eq!(content, b"caption two");
    assert!(subtitles::extract_archive_entry(&bytes, 0).is_err());
    assert!(subtitles::list_archive(&archive(&[("../escape.srt", b"bad")])).is_err());
    assert!(subtitles::list_archive(&archive(&[("folder\\escape.srt", b"bad")])).is_err());
    assert!(subtitles::list_archive(&archive(&[("readme.txt", b"nothing")])).is_err());
    assert!(subtitles::list_archive(&vec![0; subtitles::MAX_SUBTITLE_BYTES as usize + 1]).is_err());
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_symlink("link.srt", "target", zip::write::SimpleFileOptions::default()).unwrap();
    assert!(subtitles::list_archive(&zip.finish().unwrap().into_inner()).is_err());
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..257 { zip.start_file(format!("{i}.srt"), zip::write::SimpleFileOptions::default()).unwrap(); }
    assert!(subtitles::list_archive(&zip.finish().unwrap().into_inner()).unwrap_err().to_string().contains("256"));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("huge.srt", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
    zip.write_all(&vec![0; subtitles::MAX_SUBTITLE_BYTES as usize + 1]).unwrap();
    let compressed = zip.finish().unwrap().into_inner();
    assert!(compressed.len() < subtitles::MAX_SUBTITLE_BYTES as usize);
    assert!(subtitles::list_archive(&compressed).unwrap_err().to_string().contains("20 MiB"));
}

#[tokio::test]
async fn archive_choice_loads_selected_entry_and_cleans_up_with_session() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0))]).await;
    let player: Player = headless_player(dir.path());
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), player).unwrap();
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "fixture".into(), url: server.url("/video.wav"), ..Default::default() };
    let session = app.play(video, vec![], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "video", |s| s.duration > 7.0).await;
    let bytes = archive(&[("one.srt", b"1\n00:00:00,000 --> 00:00:04,000\none\n"), ("nested/日本語.srt", "1\n00:00:00,000 --> 00:00:04,000\nمرحبا\n".as_bytes())]);
    let path = dir.path().join("subtitle pack.zip");
    std::fs::write(&path, &bytes).unwrap();
    let choice = app.prepare_local_subtitle(&path, &session).await.unwrap();
    let SubtitleFileSelection::Archive { id, entries } = choice else { panic!("expected archive choices") };
    assert_eq!(entries.len(), 2);
    app.select_archived_subtitle(&id, entries[1].index).await.unwrap();
    let state = wait_for_state(&app.player, 10, "archive subtitle", |s| s.tracks.iter().any(|t| t.selected && t.title.as_deref() == Some("日本語.srt"))).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let assets = || std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")).count();
    assert_eq!(assets(), 1);
    let pending = app.prepare_local_subtitle(&path, &session).await.unwrap();
    let SubtitleFileSelection::Archive { id: stale_id, entries: stale_entries } = pending else { panic!("expected archive choices") };
    app.player.stop().await;
    assert_eq!(assets(), 0);
    assert!(app.select_archived_subtitle(&id, 0).await.is_err());
    assert!(app.select_archived_subtitle(&stale_id, stale_entries[0].index).await.unwrap_err().to_string().contains("Playback changed"));
    assert_eq!(assets(), 0);
}

#[tokio::test]
async fn provider_archive_download_is_bounded_and_requires_session_for_headers() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let huge = vec![0; subtitles::MAX_SUBTITLE_BYTES as usize + 1];
    let server = serve(vec![("/video.wav", wav(6.0)), ("/large.zip", huge)]).await;
    let dir = tempfile::tempdir().unwrap();
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "fixture".into(), url: server.url("/video.wav"), ..Default::default() };
    let session = app.play(video, vec![], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "video", |s| s.duration > 5.0).await;
    let archive = Subtitle { lang: "English".into(), url: server.url("/large.zip"), ..Default::default() };
    app.playback_context.lock().await.as_mut().unwrap().subtitles = vec![archive.clone()];
    assert!(app.prepare_provider_subtitle_archive(&session, 0).await.unwrap_err().to_string().contains("20 MiB"));
    let mut protected = archive;
    protected.headers.insert("Authorization".into(), "secret".into());
    app.playback_context.lock().await.as_mut().unwrap().subtitles = vec![protected];
    assert!(app.prepare_provider_subtitle_archive(&session, 0).await.unwrap_err().to_string().contains("requires a provider session"));
    app.player.stop().await;
}

#[test]
fn provider_archive_detection_uses_url_path_or_format_metadata() {
    let mut subtitle = Subtitle { url: "https://example.test/archive.ZIP?token=.vtt".into(), ..Default::default() };
    assert!(subtitles::is_archive_subtitle(&subtitle));
    subtitle.url = "https://example.test/download?id=1".into();
    assert!(!subtitles::is_archive_subtitle(&subtitle));
    subtitle.extra.insert("format".into(), serde_json::json!("application/zip"));
    assert!(subtitles::is_archive_subtitle(&subtitle));
}

#[test]
fn subtitle_cleanup_preserves_cues_markup_unicode_and_original_bytes() {
    let input = "WEBVTT\r\n\r\nNOTE do not alter\r\nhello\r\n\r\ncue-one\r\n00:00:01.000 --> 00:00:03.000 align:start\r\n<i>مرحبا Straße</i> &amp; [door opens] hello\r\n♪ music ♪\r\n\r\n00:00:04.000 --> 00:00:05.000\r\nSubtitles by Someone\r\n".as_bytes();
    let original = input.to_vec();
    let options = subtitles::TextTransforms { remove_captions: true, remove_music: true, remove_bloat: true, uppercase: true };
    let result = String::from_utf8(subtitles::transform_text(input, "vtt", &options).unwrap()).unwrap();
    assert!(result.contains("NOTE do not alter\nhello"));
    assert!(result.contains("cue-one\n00:00:01.000 --> 00:00:03.000 align:start\n<i>مرحبا STRASSE</i> &amp;  HELLO"));
    assert!(!result.contains("door opens"));
    assert!(!result.contains("♪"));
    assert!(!result.contains("Subtitles by"));
    assert!(!result.contains("00:00:04.000"));
    assert_eq!(input, original);
    assert_eq!(subtitles::transform_text(input, "vtt", &Default::default()).unwrap(), original);
    assert!(subtitles::transform_text(input, "ttml", &options).is_err());
    assert!(subtitles::transform_text(&[0xff], "srt", &options).is_err());
    assert!(subtitles::transform_text(b"not a subtitle", "srt", &options).is_err());
}

#[tokio::test]
async fn cleanup_loads_session_copy_and_preserves_local_and_archived_originals() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0))]).await;
    let data = dir.path().join("data");
    let resources = dir.path().join("resources");
    let app = App::open_with_player(data.clone(), resources.clone(), headless_player(dir.path())).unwrap();
    let settings = app.store.update_settings(serde_json::json!({"player":{"subtitleTransforms":{"removeCaptions":true,"uppercase":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "fixture".into(), url: server.url("/video.wav"), ..Default::default() };
    let session = app.play(video, vec![], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "video", |s| s.duration > 7.0).await;
    let source = dir.path().join("local 字幕.srt");
    let original = "1\n00:00:00,000 --> 00:00:04,000\n[door opens] مرحبا hello\n".as_bytes();
    std::fs::write(&source, original).unwrap();
    app.prepare_local_subtitle(&source, &session).await.unwrap();
    wait_for_state(&app.player, 10, "cleaned subtitle", |s| s.tracks.iter().any(|t| t.selected && t.title.as_deref() == Some("local 字幕.srt"))).await;
    let copies = || std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).filter(|entry| entry.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")).map(|entry| entry.path()).collect::<Vec<_>>();
    let paths = copies();
    assert_eq!(paths.len(), 1);
    let cleaned = std::fs::read_to_string(paths[0].join("subtitle.srt")).unwrap();
    assert!(cleaned.contains("مرحبا HELLO"));
    assert!(!cleaned.contains("door opens"));
    assert_eq!(std::fs::read(&source).unwrap(), original);
    let zip = archive(&[("English.srt", original)]);
    let path = dir.path().join("pack.zip");
    std::fs::write(&path, &zip).unwrap();
    let SubtitleFileSelection::Archive { id, entries } = app.prepare_local_subtitle(&path, &session).await.unwrap() else { panic!("expected archive") };
    app.select_archived_subtitle(&id, entries[0].index).await.unwrap();
    let paths = copies();
    assert_eq!(paths.len(), 2);
    assert!(paths.iter().all(|p| std::fs::read_to_string(p.join("subtitle.srt")).unwrap().contains("مرحبا HELLO")));
    assert_eq!(std::fs::read(&path).unwrap(), zip);
    assert_eq!(app.player.state().await.session_id.as_deref(), Some(session.as_str()));
    app.player.stop().await;
    assert!(copies().is_empty());
    let reopened = App::open_with_player(data, resources, headless_player(dir.path())).unwrap();
    assert!(reopened.player.preferences().await.subtitle_transforms.uppercase);
    assert!(reopened.player.preferences().await.subtitle_transforms.remove_captions);
}

#[test]
fn provider_text_format_uses_original_url_or_metadata() {
    let mut sub = Subtitle { url: "https://example.test/subtitle.VTT?token=.srt".into(), ..Default::default() };
    assert_eq!(subtitles::text_format(&sub), Some("vtt"));
    sub.url = "https://example.test/download?token=.vtt".into();
    assert_eq!(subtitles::text_format(&sub), None);
    sub.extra.insert("mimeType".into(), serde_json::json!("text/vtt"));
    assert_eq!(subtitles::text_format(&sub), Some("vtt"));
    sub.extra.insert("mimeType".into(), serde_json::json!("application/x-subrip"));
    assert_eq!(subtitles::text_format(&sub), Some("srt"));
}

#[tokio::test]
async fn provider_cleanup_fallback_is_visible_without_exposing_url() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0)), ("/bad.vtt?secret=private", b"not a subtitle".to_vec())]).await;
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    let settings = app.store.update_settings(serde_json::json!({"player":{"subtitleTransforms":{"uppercase":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let mut events = app.player.subscribe();
    let video = Stream { name: "Video".into(), url: server.url("/video.wav"), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), url: server.url("/bad.vtt?secret=private"), ..Default::default() };
    let session = app.play(video, vec![sub], None, None, 0.0, None, None).await.unwrap();
    let notice = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(PlayerEvent::Notice { session_id, message }) = events.recv().await {
                if session_id == session && message.contains("Could not clean subtitle") { break message; }
            }
        }
    }).await.expect("cleanup failure should be visible");
    assert!(notice.contains("Using the original"));
    assert!(!notice.contains("private"));
    assert!(std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).all(|e| !e.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")));
    app.player.stop().await;
}

#[tokio::test]
async fn slow_provider_cleanup_cannot_override_stop_or_newer_playback() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0))]).await;
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    let settings = app.store.update_settings(serde_json::json!({"player":{"subtitleTransforms":{"uppercase":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    for mode in 0..3 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/slow.vtt", listener.local_addr().unwrap());
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let _ = received_tx.send(());
            let _ = release_rx.await;
            let body = b"WEBVTT\n\n00:00:00.000 --> 00:00:03.000\nhello\n";
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body).await;
        });
        let pending = app.clone();
        let old_video = Stream { name: "Old".into(), url: server.url("/video.wav"), ..Default::default() };
        let old = tokio::spawn(async move { pending.play(old_video, vec![Subtitle { lang: "Slow".into(), url, ..Default::default() }], None, None, 0.0, None, None).await });
        tokio::time::timeout(std::time::Duration::from_secs(5), received_rx).await.unwrap().unwrap();
        let old_session = tokio::time::timeout(std::time::Duration::from_secs(3), old).await.expect("video should start before the subtitle response").unwrap().unwrap();
        assert_eq!(app.player.state().await.session_id.as_deref(), Some(old_session.as_str()));
        let new_session = if mode == 1 {
            let video = Stream { name: "New".into(), url: server.url("/video.wav"), ..Default::default() };
            Some(app.play(video, vec![], None, None, 0.0, None, None).await.unwrap())
        } else if mode == 2 {
            let _ = app.player.command(serde_json::json!(["quit"])).await;
            wait_for_state(&app.player, 5, "player exit", |s| !s.active).await;
            None
        } else { app.stop_playback().await; None };
        release_tx.send(()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let state = app.player.state().await;
        if let Some(id) = new_session { assert_eq!(state.session_id.as_deref(), Some(id.as_str())); }
        else { assert!(!state.active); }
        assert!(std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).all(|e| !e.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")));
        server_task.await.unwrap();
        app.stop_playback().await;
    }
}

#[tokio::test]
async fn delayed_provider_subtitle_does_not_override_user_turning_subtitles_off() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0))]).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/slow.vtt", listener.local_addr().unwrap());
    let (received_tx, received_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let responder = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        let _ = received_tx.send(());
        let _ = release_rx.await;
        let body = b"WEBVTT\n\n00:00:00.000 --> 00:00:03.000\nhello\n";
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(body).await;
    });
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    let settings = app.store.update_settings(serde_json::json!({"player":{"subtitleLanguage":"en","subtitlesEnabled":true,"subtitleTransforms":{"uppercase":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "Video".into(), url: server.url("/video.wav"), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), lang_tag: Some("en".into()), url, ..Default::default() };
    let session = app.play(video, vec![sub], None, None, 0.0, None, None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), received_rx).await.unwrap().unwrap();
    app.player.execute(PlayerCommand::SetSubtitleTrack { id: None }).await.unwrap();
    release_tx.send(()).unwrap();
    let state = wait_for_state(&app.player, 10, "late subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    assert!(state.tracks.iter().all(|t| t.kind != "sub" || !t.selected));
    app.stop_playback().await;
    responder.await.unwrap();
}

#[test]
fn ass_cleanup_preserves_styles_overrides_drawing_and_unicode() {
    let source = "[Script Info]\r\nTitle: mixed Case\r\n[V4+ Styles]\r\nFormat: Name, Fontname\r\nStyle: Default,Comic Sans\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nComment: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,comment remains\r\nDialogue: 0,0:00:00.00,0:00:03.00,Default,,0,0,0,,{\\i1\\k20}hello, Straße [do{\\b1}or]\\N{\\p1}m 0 0 l 1 1{\\p0}{\\i0}مرحبا\r\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\i1}♪ music ♪\\Nhello\r\n";
    let options = subtitles::TextTransforms { remove_captions: true, remove_music: true, remove_bloat: true, uppercase: true };
    let cleaned = String::from_utf8(subtitles::transform_text(source.as_bytes(), "ass", &options).unwrap()).unwrap();
    assert!(cleaned.contains("Title: mixed Case\r\n"));
    assert!(cleaned.contains("Style: Default,Comic Sans\r\n"));
    assert!(cleaned.contains("comment remains\r\n"));
    assert!(cleaned.contains("{\\i1\\k20}HELLO, STRASSE {\\b1}\\N{\\p1}m 0 0 l 1 1{\\p0}{\\i0}مرحبا"));
    assert!(cleaned.contains("{\\i1}\\NHELLO"));
    assert!(!cleaned.contains("[do"));
    assert!(!cleaned.contains("♪ music ♪"));
    assert_eq!(subtitles::transform_text(source.as_bytes(), "ssa", &Default::default()).unwrap(), source.as_bytes());
    let bad = "[Events]\nFormat: Text, Start\nDialogue: hi,0:00:00.00\n";
    assert!(subtitles::transform_text(bad.as_bytes(), "ass", &options).is_err());
    let bad = "[Events]\nFormat: Start, Text\nDialogue: 0:00:00.00,{\\i1 hi\n";
    assert!(subtitles::transform_text(bad.as_bytes(), "ass", &options).is_err());
}

#[tokio::test]
async fn cleaned_ass_loads_from_provider_and_local_copy_without_changing_authored_styles() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let dir = tempfile::tempdir().unwrap();
    let ass = b"[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,{\\i1}hello [door]{\\i0}\n";
    let server = serve(vec![("/video.wav", wav(8.0)), ("/sub.ass", ass.to_vec())]).await;
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    let settings = app.store.update_settings(serde_json::json!({"player":{"subtitleTransforms":{"uppercase":true,"removeCaptions":true}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "ASS".into(), url: server.url("/video.wav"), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), url: server.url("/sub.ass"), ..Default::default() };
    let session = app.play(video, vec![sub], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "provider ASS", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    let copies = || std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("cloudstream-subtitle-")).map(|e| e.path()).collect::<Vec<_>>();
    let paths = copies();
    assert_eq!(paths.len(), 1);
    let cleaned = std::fs::read_to_string(paths[0].join("subtitle.ass")).unwrap();
    assert!(cleaned.contains("Style: Default,Arial,24"));
    assert!(cleaned.contains("{\\i1}HELLO {\\i0}"));
    let local = dir.path().join("字幕.ass");
    std::fs::write(&local, ass).unwrap();
    app.prepare_local_subtitle(&local, &session).await.unwrap();
    let state = wait_for_state(&app.player, 10, "local ASS", |s| s.tracks.iter().any(|t| t.selected && t.title.as_deref() == Some("字幕.ass"))).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    assert_eq!(std::fs::read(&local).unwrap(), ass);
    assert_eq!(copies().len(), 2);
    app.stop_playback().await;
    assert!(copies().is_empty());
}
