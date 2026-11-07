mod support;
use base64::Engine;
use cloudstream_core::{fonts, models::*, App};
use serde_json::json;
use std::path::{Path, PathBuf};
use support::*;

fn system_font() -> Option<PathBuf> {
    ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/Adwaita/AdwaitaMono-Regular.ttf"]
        .into_iter().map(PathBuf::from).find(|path| path.is_file())
}

#[test]
fn font_import_is_bounded_content_addressed_and_does_not_change_the_original() {
    let Some(system) = system_font() else { eprintln!("SKIPPED: no fixture font available"); return };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("font with spaces 字体.ttf");
    let original = std::fs::read(system).unwrap();
    std::fs::write(&source, &original).unwrap();
    let root = dir.path().join("fonts");
    let font = fonts::import_font(&root, &source).unwrap();
    assert!(!font.family.is_empty());
    assert!(fonts::valid_font_id(&font.id));
    assert_eq!(fonts::read_font(&root, &font.id).unwrap(), original);
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(fonts::import_font(&root, &source).unwrap().id, font.id);
    assert!(fonts::read_font(&root, "../anything").is_err());
    let wrong = dir.path().join("wrong.ttf");
    std::fs::write(&wrong, b"not a font").unwrap();
    assert!(fonts::import_font(&root, &wrong).is_err());
    let huge = dir.path().join("huge.ttf");
    std::fs::File::create(&huge).unwrap().set_len(fonts::MAX_FONT_BYTES + 1).unwrap();
    assert!(fonts::import_font(&root, &huge).unwrap_err().to_string().contains("10 MiB"));
    let cached = fonts::font_directory(&root, &font.id).unwrap().join("font.ttf");
    std::fs::write(cached, b"damaged").unwrap();
    assert!(fonts::read_font(&root, &font.id).is_err());
}

#[tokio::test]
async fn imported_font_applies_to_mpv_and_survives_reopen() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let Some(font_path) = system_font() else { eprintln!("SKIPPED: no fixture font available"); return };
    let dir = tempfile::tempdir().unwrap();
    let server = serve(vec![("/video.wav", wav(8.0)), ("/sub.vtt", b"WEBVTT\n\n00:00:00.000 --> 00:00:05.000\nHello\n".to_vec())]).await;
    let data = dir.path().join("data");
    let resources = dir.path().join("resources");
    let app = App::open_with_player(data.clone(), resources.clone(), headless_player(dir.path())).unwrap();
    let imported = app.import_subtitle_font(Path::new(&font_path)).unwrap();
    let preview = app.subtitle_font_preview(&imported.id).unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(preview).unwrap(), std::fs::read(font_path).unwrap());
    assert!(app.subtitle_font_preview("../../etc/passwd").is_err());
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let stream = Stream { name: "Video".into(), url: server.url("/video.wav"), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), lang_tag: Some("en".into()), url: server.url("/sub.vtt"), ..Default::default() };
    let session = app.play(stream.clone(), vec![sub.clone()], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&app.player, 10, "subtitle track", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    let settings = app.store.update_settings(json!({"player":{"subtitleStyle":{"font": imported.family, "fontId":imported.id}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    let expected_dir = data.join("subtitle-fonts").join(&imported.id);
    assert_eq!(app.player.command(json!(["get_property", "sub-fonts-dir"])).await.unwrap(), json!(expected_dir.to_str().unwrap()));
    assert_eq!(app.player.command(json!(["get_property", "sub-font"])).await.unwrap(), json!(imported.family));
    assert_eq!(app.player.state().await.session_id.as_deref(), Some(session.as_str()));
    app.player.stop().await;
    let reopened = App::open_with_player(data, resources, headless_player(dir.path())).unwrap();
    assert_eq!(reopened.player.preferences().await.subtitle_style.font_id.as_deref(), Some(imported.id.as_str()));
    reopened.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    reopened.play(stream, vec![sub], None, None, 0.0, None, None).await.unwrap();
    wait_for_state(&reopened.player, 10, "subtitle after restart", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    assert_eq!(reopened.player.command(json!(["get_property", "sub-fonts-dir"])).await.unwrap(), json!(expected_dir.to_str().unwrap()));
    reopened.player.stop().await;
}

#[tokio::test]
async fn imported_unique_font_is_loaded_by_libass_without_restarting_video() {
    if !mpv_installed() { eprintln!("SKIPPED: mpv is not installed"); return; }
    let Some(font_path) = system_font() else { eprintln!("SKIPPED: no fixture font available"); return };
    let original = std::fs::read(font_path).unwrap();
    let old: Vec<u8> = "DejaVu Sans".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let new: Vec<u8> = "CodexV Sans".encode_utf16().flat_map(u16::to_be_bytes).collect();
    if !original.windows(old.len()).any(|w| w == old) { eprintln!("SKIPPED: DejaVu fixture not available"); return; }
    let mut unique = original.clone();
    for (from, to) in [(old, new), (b"DejaVu Sans".to_vec(), b"CodexV Sans".to_vec())] {
        let mut pos = 0;
        while pos + from.len() <= unique.len() {
            if unique[pos..].starts_with(&from) { unique[pos..pos + from.len()].copy_from_slice(&to); pos += from.len(); }
            else { pos += 1; }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("unique font.ttf");
    std::fs::write(&source, unique).unwrap();
    let mut video = b"YUV4MPEG2 W160 H90 F5:1 Ip A1:1 C420jpeg\n".to_vec();
    for _ in 0..20 { video.extend_from_slice(b"FRAME\n"); video.extend(vec![100; 160 * 90]); video.extend(vec![128; 160 * 90 / 2]); }
    let server = serve(vec![("/video.y4m", video), ("/sub.vtt", b"WEBVTT\n\n00:00:00.000 --> 00:00:04.000\nHello\n".to_vec())]).await;
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    let log = dir.path().join("mpv.log");
    let player = cloudstream_core::player::Player::new(dir.path().to_owned()).with_launch_args(vec![
        "--vo=image".into(), "--vo-image-format=png".into(), format!("--vo-image-outdir={}", frames.display()),
        "--ao=null".into(), "--force-window=no".into(), format!("--log-file={}", log.display()), "--msg-level=all=warn,sub/ass=debug".into(),
    ]);
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), player).unwrap();
    let font = app.import_subtitle_font(&source).unwrap();
    assert_eq!(font.family, "CodexV Sans");
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;
    let video = Stream { name: "Font fixture".into(), url: server.url("/video.y4m"), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), url: server.url("/sub.vtt"), ..Default::default() };
    let session = app.play(video, vec![sub], None, None, 0.0, None, None).await.unwrap();
    let state = wait_for_state(&app.player, 10, "subtitle", |s| s.tracks.iter().any(|t| t.kind == "sub")).await;
    let subtitle_id = state.tracks.iter().find(|t| t.kind == "sub").unwrap().id;
    app.player.execute(PlayerCommand::SetSubtitleTrack { id: Some(subtitle_id) }).await.unwrap();
    let settings = app.store.update_settings(json!({"player":{"subtitleStyle":{"font":font.family, "fontId":font.id}}}), Some("global"), None).unwrap();
    app.apply_player_settings(&settings).await;
    app.player.execute(PlayerCommand::SeekAbsolute { seconds: 1.0 }).await.unwrap();
    let imported_file = dir.path().join("data/subtitle-fonts").join(&font.id).join("font.ttf");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("Loading font file") && text.contains(imported_file.to_str().unwrap()) && text.contains("fontselect: (CodexV Sans") { break; }
        if tokio::time::Instant::now() > deadline { panic!("custom font was not selected by libass: {text}"); }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(app.player.state().await.session_id.as_deref(), Some(session.as_str()));
    app.player.stop().await;
}
