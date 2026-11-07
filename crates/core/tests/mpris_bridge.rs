//! MPRIS requests become the app's validated player commands (T27), checked against a fake mpv.
use cloudstream_core::{entry::EntryAction, mpris::*, player::OBSERVED_PROPERTIES, App};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

async fn playing_app(dir: &std::path::Path) -> (std::sync::Arc<App>, mpsc::UnboundedReceiver<Value>) {
    let app = App::open(dir.join("data"), dir.join("resources")).unwrap();
    let socket = dir.join("m.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let mut seen = 0;
        write.write_all(b"{\"event\":\"property-change\",\"name\":\"duration\",\"data\":100.0}\n").await.unwrap();
        while let Ok(Some(line)) = lines.next_line().await {
            let value: Value = serde_json::from_str(&line).unwrap();
            if seen < OBSERVED_PROPERTIES.len() {
                seen += 1;
                continue;
            }
            let _ = tx.send(value["command"].clone());
            let reply = json!({ "request_id": value["request_id"], "error": "success", "data": null }).to_string() + "\n";
            write.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    let client = tokio::net::UnixStream::connect(&socket).await.unwrap();
    app.player.start_session("sess-1".into(), "default".into(), None, socket, client, None, 0.0).await;
    for _ in 0..100 {
        if app.player.state().await.duration > 0.0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    (app, rx)
}

#[tokio::test]
async fn controls_map_onto_validated_player_commands() {
    let dir = tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap();
    let (app, mut mpv) = playing_app(dir.path()).await;
    let next = |rx: &mut mpsc::UnboundedReceiver<Value>| {
        let value = rx.try_recv().unwrap_or_else(|_| panic!("no command reached mpv"));
        value
    };
    app.handle_mpris(MprisRequest::PlayPause).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["cycle", "pause"]));
    app.handle_mpris(MprisRequest::Pause).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["set_property", "pause", true]));
    app.handle_mpris(MprisRequest::Play).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["set_property", "pause", false]));
    app.handle_mpris(MprisRequest::Seek(-7_500_000)).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["seek", -7.5, "relative"]));
    app.handle_mpris(MprisRequest::SetVolume(0.4)).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["set_property", "volume", 40.0]));
    app.handle_mpris(MprisRequest::SetRate(2.0)).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["set_property", "speed", 2.0]));

    // Position: only for the track that is playing, and bounded by its duration.
    let track = track_id_for("sess-1");
    app.handle_mpris(MprisRequest::SetPosition { track_id: track.clone(), position_us: 30_000_000 }).await.unwrap();
    assert_eq!(next(&mut mpv), json!(["seek", 30.0, "absolute"]));
    app.handle_mpris(MprisRequest::SetPosition { track_id: track_id_for("another-session"), position_us: 30_000_000 }).await.unwrap();
    assert!(mpv.try_recv().is_err(), "a stale track id must not move the current one");
    let error = app.handle_mpris(MprisRequest::SetPosition { track_id: track, position_us: 500_000_000 }).await.unwrap_err().to_string();
    assert!(error.contains("only 100.0s long"), "{error}");
    assert!(mpv.try_recv().is_err());
    let error = app.handle_mpris(MprisRequest::SetRate(50.0)).await.unwrap_err().to_string();
    assert!(error.contains("between 0.25x and 4x"), "{error}");
}

#[tokio::test]
async fn open_uri_only_queues_a_confirmation_and_never_plays() {
    let dir = tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();
    app.handle_mpris(MprisRequest::OpenUri("cloudstreamsearch://from%20a%20widget".into())).await.unwrap();
    app.handle_mpris(MprisRequest::OpenUri("cloudstreamplayer://https%3A%2F%2Fevil.test%2Fv.mp4".into())).await.unwrap();
    app.handle_mpris(MprisRequest::OpenUri("javascript:alert(1)".into())).await.unwrap();
    assert!(!app.player.state().await.active, "nothing started playing");
    let (actions, errors) = app.take_entries();
    assert_eq!(actions, vec![
        EntryAction::Search { query: "from a widget".into() },
        EntryAction::PlayUrl { url: "https://evil.test/v.mp4".into(), name: None },
    ]);
    assert_eq!(errors.len(), 1, "the unsupported address is reported, not ignored: {errors:?}");
    assert!(app.take_entries().0.is_empty());
}

#[tokio::test]
async fn controls_with_nothing_playing_fail_without_side_effects() {
    let dir = tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap();
    let app = App::open(dir.path().join("data"), dir.path().join("resources")).unwrap();
    assert!(app.handle_mpris(MprisRequest::PlayPause).await.unwrap_err().to_string().contains("Nothing is playing"));
    app.handle_mpris(MprisRequest::Stop).await.unwrap();
    app.handle_mpris(MprisRequest::Raise).await.unwrap();
    assert!(!app.advance_queue_and_play(true).await.unwrap());
}
