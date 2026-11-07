//! Opening a file from disk (T27): validated, resolved, and never reachable through a provider.
mod support;
use cloudstream_core::{models::*, App};
use support::*;

#[tokio::test]
async fn a_local_media_file_plays_and_names_what_is_open() {
    if !mpv_installed() {
        eprintln!("SKIPPED: mpv is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("My Videos (日本語)");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("clip with spaces.wav");
    std::fs::write(&file, wav(3.0)).unwrap();
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    app.player.set_policy(PlaybackPolicy { start_paused: true, ..Default::default() }).await;

    let session = app.play_local_file(file.to_str().unwrap(), None).await.unwrap();
    let state = wait_for_state(&app.player, 10, "the local file to load", |s| s.duration > 2.5).await;
    assert_eq!(state.session_id.as_deref(), Some(session.as_str()));
    let stream = state.stream.unwrap();
    assert_eq!(stream.name, "clip with spaces.wav");
    assert_eq!(stream.kind.as_deref(), Some("local_file"));
    app.player.stop().await;
}

#[tokio::test]
async fn files_that_are_not_media_or_do_not_exist_are_refused_before_mpv_starts() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open_with_player(dir.path().join("data"), dir.path().join("resources"), headless_player(dir.path())).unwrap();
    let text = dir.path().join("secret.txt");
    std::fs::write(&text, b"x").unwrap();
    let link = dir.path().join("looks-like-video.mp4");
    std::os::unix::fs::symlink(&text, &link).unwrap();
    for (path, needle) in [
        (text.to_str().unwrap().to_string(), "not a video or audio file"),
        (link.to_str().unwrap().to_string(), "not a video or audio file"),
        (dir.path().join("missing.mkv").display().to_string(), "does not exist"),
        ("relative.mkv".to_string(), "full path"),
        ("/".to_string(), "not a video or audio file"),
    ] {
        let error = app.play_local_file(&path, None).await.unwrap_err().to_string();
        assert!(error.contains(needle), "{path}: '{error}' should mention '{needle}'");
    }
    assert!(!app.player.state().await.active);
}

#[tokio::test]
async fn a_provider_cannot_make_the_player_open_a_local_file() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("private-video.mkv");
    std::fs::write(&secret, b"x").unwrap();
    let player = headless_player(dir.path());
    let hostile = Stream {
        name: "Totally a stream".into(),
        url: secret.display().to_string(),
        kind: Some("local_file".into()),
        ..Default::default()
    };
    let error = player.play(&hostile, &[], None, 0.0, None, None).await.unwrap_err().to_string();
    assert!(error.contains("not supported"), "{error}");
    assert!(!player.state().await.active);
}
