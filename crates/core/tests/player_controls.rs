//! Typed player commands against a fake mpv IPC server: exact IPC messages, validation, the
//! state that mpv reports back, and capability-aware launch options. No mpv or display needed.
mod support;

use cloudstream_core::{models::*, player::*};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

fn mpv_json(command: &PlayerCommand) -> Vec<Value> {
    command.to_mpv().into_iter().map(Value::Array).collect()
}

#[test]
fn typed_commands_map_to_exact_mpv_ipc_messages() {
    let cases: Vec<(PlayerCommand, Vec<Value>)> = vec![
        (PlayerCommand::Play, vec![json!(["set_property", "pause", false])]),
        (PlayerCommand::Pause, vec![json!(["set_property", "pause", true])]),
        (PlayerCommand::TogglePause, vec![json!(["cycle", "pause"])]),
        (PlayerCommand::SeekRelative { seconds: -10.0 }, vec![json!(["seek", -10.0, "relative"])]),
        (PlayerCommand::SeekAbsolute { seconds: 42.5 }, vec![json!(["seek", 42.5, "absolute"])]),
        (PlayerCommand::SetVolume { volume: 80.0 }, vec![json!(["set_property", "volume", 80.0])]),
        (PlayerCommand::SetMute { muted: true }, vec![json!(["set_property", "mute", true])]),
        (PlayerCommand::SetSpeed { speed: 1.5 }, vec![json!(["set_property", "speed", 1.5])]),
        (PlayerCommand::SetFullscreen { fullscreen: true }, vec![json!(["set_property", "fullscreen", true])]),
        (PlayerCommand::SetAudioDelay { seconds: -0.25 }, vec![json!(["set_property", "audio-delay", -0.25])]),
        (PlayerCommand::SetRotation { degrees: 270 }, vec![json!(["set_property", "video-rotate", 270])]),
        (
            PlayerCommand::SetAspect { mode: AspectMode::Fill },
            vec![
                json!(["set_property", "keepaspect", true]),
                json!(["set_property", "panscan", 1.0]),
                json!(["set_property", "video-aspect-override", -1]),
            ],
        ),
        (
            PlayerCommand::SetAspect { mode: AspectMode::Stretch },
            vec![
                json!(["set_property", "keepaspect", false]),
                json!(["set_property", "panscan", 0.0]),
                json!(["set_property", "video-aspect-override", -1]),
            ],
        ),
        (
            PlayerCommand::SetAspect { mode: AspectMode::Ratio16x9 },
            vec![
                json!(["set_property", "keepaspect", true]),
                json!(["set_property", "panscan", 0.0]),
                json!(["set_property", "video-aspect-override", "16:9"]),
            ],
        ),
    ];
    for (command, expected) in cases {
        command.validate(120.0).unwrap();
        assert_eq!(mpv_json(&command), expected, "{command:?}");
    }
}

#[test]
fn commands_deserialize_from_the_ui_wire_format() {
    let command: PlayerCommand =
        serde_json::from_value(json!({ "type": "set_audio_delay", "seconds": 0.5 })).unwrap();
    assert_eq!(command, PlayerCommand::SetAudioDelay { seconds: 0.5 });
    let aspect: PlayerCommand =
        serde_json::from_value(json!({ "type": "set_aspect", "mode": "21:9" })).unwrap();
    assert_eq!(aspect, PlayerCommand::SetAspect { mode: AspectMode::Ratio21x9 });
    assert!(serde_json::from_value::<PlayerCommand>(json!({ "type": "launch_missiles" })).is_err());
    // The wire format round-trips so the Tauri command and the TypeScript union agree.
    let text = serde_json::to_string(&PlayerCommand::SeekAbsolute { seconds: 3.0 }).unwrap();
    assert_eq!(text, r#"{"type":"seek_absolute","seconds":3.0}"#);
}

#[test]
fn out_of_range_commands_are_rejected_with_useful_messages() {
    let reject = |command: PlayerCommand, duration: f64, needle: &str| {
        let error = command.validate(duration).unwrap_err().to_string();
        assert!(error.contains(needle), "{command:?}: '{error}' should mention '{needle}'");
    };
    reject(PlayerCommand::SetVolume { volume: 131.0 }, 0.0, "between 0 and 130");
    reject(PlayerCommand::SetVolume { volume: -1.0 }, 0.0, "between 0 and 130");
    reject(PlayerCommand::SetVolume { volume: f64::NAN }, 0.0, "finite");
    reject(PlayerCommand::SetSpeed { speed: 0.0 }, 0.0, "between 0.25x and 4x");
    reject(PlayerCommand::SetSpeed { speed: 8.0 }, 0.0, "between 0.25x and 4x");
    reject(PlayerCommand::SetAudioDelay { seconds: 31.0 }, 0.0, "±30");
    reject(PlayerCommand::SetSubtitleDelay { seconds: -31.0 }, 0.0, "±30");
    reject(PlayerCommand::SetRotation { degrees: 45 }, 0.0, "0, 90, 180 or 270");
    reject(PlayerCommand::SeekAbsolute { seconds: -1.0 }, 0.0, "negative");
    reject(PlayerCommand::SeekAbsolute { seconds: 99.0 }, 60.0, "only 60.0s long");
    reject(PlayerCommand::SeekAbsolute { seconds: f64::INFINITY }, 0.0, "finite");
    reject(PlayerCommand::SeekRelative { seconds: 1.0e9 }, 0.0, "within");
    reject(PlayerCommand::SetAspect { mode: AspectMode::Custom }, 0.0, "Choose Fit");
    // Live or unknown-duration streams may be seeked anywhere that is not negative.
    PlayerCommand::SeekAbsolute { seconds: 5_000.0 }.validate(0.0).unwrap();
    // Boundaries are accepted.
    PlayerCommand::SetVolume { volume: 130.0 }.validate(0.0).unwrap();
    PlayerCommand::SetSpeed { speed: 0.25 }.validate(0.0).unwrap();
    PlayerCommand::SetSpeed { speed: 4.0 }.validate(0.0).unwrap();
    PlayerCommand::SeekAbsolute { seconds: 60.0 }.validate(60.0).unwrap();
}

#[test]
fn aspect_mode_is_derived_from_mpv_properties() {
    assert_eq!(AspectMode::from_mpv(true, 0.0, -1.0), AspectMode::Fit);
    assert_eq!(AspectMode::from_mpv(true, 0.0, -2.0), AspectMode::Fit, "mpv reports -2 for 'follow the file'");
    assert_eq!(AspectMode::from_mpv(true, 1.0, -1.0), AspectMode::Fill);
    assert_eq!(AspectMode::from_mpv(false, 0.0, -1.0), AspectMode::Stretch);
    assert_eq!(AspectMode::from_mpv(true, 0.0, 16.0 / 9.0), AspectMode::Ratio16x9);
    assert_eq!(AspectMode::from_mpv(true, 0.0, 4.0 / 3.0), AspectMode::Ratio4x3);
    assert_eq!(AspectMode::from_mpv(true, 0.0, 21.0 / 9.0), AspectMode::Ratio21x9);
    assert_eq!(AspectMode::from_mpv(true, 0.4, -1.0), AspectMode::Custom);
    assert_eq!(AspectMode::from_mpv(true, 0.0, 1.9), AspectMode::Custom);
    // Every offered mode round-trips through its mpv values.
    for mode in [
        AspectMode::Fit,
        AspectMode::Fill,
        AspectMode::Stretch,
        AspectMode::Ratio16x9,
        AspectMode::Ratio4x3,
        AspectMode::Ratio21x9,
    ] {
        let (keep, pan, aspect) = mode.mpv_values().unwrap();
        let ratio = match aspect {
            Value::String(text) => {
                let (a, b) = text.split_once(':').unwrap();
                a.parse::<f64>().unwrap() / b.parse::<f64>().unwrap()
            }
            other => other.as_f64().unwrap(),
        };
        assert_eq!(AspectMode::from_mpv(keep, pan, ratio), mode);
    }
}

struct FakeMpv {
    /// Commands the app sent after the observe_property handshake, in order.
    received: mpsc::UnboundedReceiver<Value>,
    /// Lines to push to the app as if mpv had emitted them.
    inject: mpsc::UnboundedSender<Value>,
    /// Properties that should answer with this mpv error instead of success.
    fail_with: std::sync::Arc<std::sync::Mutex<Option<&'static str>>>,
    socket: std::path::PathBuf,
}

async fn fake_session(player: &Player, directory: &std::path::Path) -> FakeMpv {
    let socket = directory.join("fake.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (received_tx, received) = mpsc::unbounded_channel();
    let (inject, mut inject_rx) = mpsc::unbounded_channel::<Value>();
    let fail_with = std::sync::Arc::new(std::sync::Mutex::new(None));
    let fail = fail_with.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        let mut observed = 0;
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { return };
                    let value: Value = serde_json::from_str(&line).unwrap();
                    if observed < OBSERVED_PROPERTIES.len() {
                        observed += 1;
                        continue;
                    }
                    let _ = received_tx.send(value["command"].clone());
                    let error = fail.lock().unwrap().unwrap_or("success");
                    let reply = json!({ "request_id": value["request_id"], "error": error, "data": null });
                    write_half.write_all((reply.to_string() + "\n").as_bytes()).await.unwrap();
                }
                Some(event) = inject_rx.recv() => {
                    write_half.write_all((event.to_string() + "\n").as_bytes()).await.unwrap();
                }
            }
        }
    });
    let client = tokio::net::UnixStream::connect(&socket).await.unwrap();
    player
        .start_session("fake-session".into(), "default".into(), None, socket.clone(), client, None, 0.0)
        .await;
    FakeMpv { received, inject, fail_with, socket }
}

fn property(name: &str, data: Value) -> Value {
    json!({ "event": "property-change", "name": name, "data": data })
}

async fn settle<F: Fn(&PlayerState) -> bool>(player: &Player, condition: F) -> PlayerState {
    for _ in 0..100 {
        let state = player.state().await;
        if condition(&state) {
            return state;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("state never matched: {:?}", player.state().await);
}

#[tokio::test]
async fn acknowledged_commands_reach_mpv_and_state_follows_what_mpv_reports() {
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let mut mpv = fake_session(&player, directory.path()).await;
    mpv.inject.send(property("duration", json!(100.0))).unwrap();
    settle(&player, |s| s.duration > 99.0).await;

    // Volume is persisted as a preference; the displayed value only changes when mpv says so.
    let prefs = player.execute(PlayerCommand::SetVolume { volume: 80.0 }).await.unwrap();
    assert_eq!(prefs.unwrap().volume, 80.0);
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "volume", 80.0]));
    assert_eq!(player.state().await.volume, 100.0, "no optimistic update before mpv confirms");
    mpv.inject.send(property("volume", json!(80.0))).unwrap();
    settle(&player, |s| s.volume == 80.0).await;

    // Seeking and rotation are per video and are not persisted.
    assert!(player.execute(PlayerCommand::SeekAbsolute { seconds: 30.0 }).await.unwrap().is_none());
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["seek", 30.0, "absolute"]));
    assert!(player.execute(PlayerCommand::SetRotation { degrees: 90 }).await.unwrap().is_none());
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "video-rotate", 90]));
    mpv.inject.send(property("video-rotate", json!(90))).unwrap();
    assert_eq!(settle(&player, |s| s.rotation == 90).await.rotation, 90);

    // Aspect is three properties; the resulting mode is derived from mpv's report.
    let prefs = player.execute(PlayerCommand::SetAspect { mode: AspectMode::Fill }).await.unwrap();
    assert_eq!(prefs.unwrap().aspect, AspectMode::Fill);
    for _ in 0..3 {
        mpv.received.recv().await.unwrap();
    }
    mpv.inject.send(property("panscan", json!(1.0))).unwrap();
    assert_eq!(settle(&player, |s| s.aspect == AspectMode::Fill).await.aspect, AspectMode::Fill);
    // A change made with mpv's own keys that this app does not offer is reported honestly.
    mpv.inject.send(property("panscan", json!(0.4))).unwrap();
    assert_eq!(settle(&player, |s| s.aspect == AspectMode::Custom).await.aspect, AspectMode::Custom);

    // Native changes show up even though no command was sent.
    mpv.inject.send(property("mute", json!(true))).unwrap();
    mpv.inject.send(property("fullscreen", json!(true))).unwrap();
    mpv.inject.send(property("audio-delay", json!(0.35))).unwrap();
    mpv.inject.send(property("hwdec-current", json!("vaapi"))).unwrap();
    let state = settle(&player, |s| s.muted && s.fullscreen && s.hwdec_current.is_some()).await;
    assert!((state.audio_delay - 0.35).abs() < 1e-9);
    assert_eq!(state.hwdec_current.as_deref(), Some("vaapi"));
    mpv.inject.send(property("hwdec-current", json!("no"))).unwrap();
    assert_eq!(settle(&player, |s| s.hwdec_current.is_none()).await.hwdec_current, None);
    let _ = &mpv.socket;
}

#[tokio::test]
async fn speed_hold_restores_session_speed_without_changing_saved_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let mut mpv = fake_session(&player, directory.path()).await;
    mpv.inject.send(property("speed", json!(1.25))).unwrap();
    settle(&player, |state| state.speed == 1.25).await;
    assert!(player.begin_speed_hold("old-session", 2.0).await.is_err());
    assert!(player.begin_speed_hold("fake-session", 5.0).await.is_err());
    player.begin_speed_hold("fake-session", 2.0).await.unwrap();
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "speed", 2.0]));
    player.begin_speed_hold("fake-session", 2.0).await.unwrap();
    player.end_speed_hold("old-session").await.unwrap();
    player.end_speed_hold("fake-session").await.unwrap();
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "speed", 1.25]));
    assert_eq!(player.preferences().await.speed, 1.0);
    player.stop().await;
    player.end_speed_hold("fake-session").await.unwrap();
}

#[tokio::test]
async fn rejected_commands_never_reach_mpv_and_mpv_errors_become_readable() {
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let mut mpv = fake_session(&player, directory.path()).await;
    mpv.inject.send(property("duration", json!(60.0))).unwrap();
    settle(&player, |s| s.duration > 59.0).await;

    let error = player.execute(PlayerCommand::SetVolume { volume: 500.0 }).await.unwrap_err();
    assert!(error.to_string().contains("between 0 and 130"), "{error}");
    let error = player.execute(PlayerCommand::SeekAbsolute { seconds: 61.0 }).await.unwrap_err();
    assert!(error.to_string().contains("only 60.0s long"), "{error}");
    assert!(mpv.received.try_recv().is_err(), "invalid commands must not be sent to mpv");
    assert_eq!(player.preferences().await.volume, 100.0, "a rejected command changes no preference");

    *mpv.fail_with.lock().unwrap() = Some("property unavailable");
    let error = player.execute(PlayerCommand::SetSpeed { speed: 2.0 }).await.unwrap_err();
    assert_eq!(error.to_string(), "Could not change the speed: the video has not finished loading yet");
    assert_eq!(player.preferences().await.speed, 1.0, "a refused command is not remembered");
    *mpv.fail_with.lock().unwrap() = Some("property not found");
    let error = player.execute(PlayerCommand::SetRotation { degrees: 90 }).await.unwrap_err();
    assert_eq!(error.to_string(), "Could not rotate the video: this mpv does not support it");
}

#[tokio::test]
async fn commands_without_a_session_explain_why() {
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let error = player.execute(PlayerCommand::TogglePause).await.unwrap_err();
    assert_eq!(error.to_string(), "Nothing is playing");
}

fn all_capabilities(unsupported: &[(&str, &str)]) -> PlayerCapabilities {
    let ids = [
        "volume", "mute", "seek", "speed", "fullscreen", "aspect", "audio_delay", "subtitle_style", "subtitle_delay", "rotation",
        "hardware_decoding", "buffer_size", "buffer_time", "audio_compressor", "playlist_timeline",
    ];
    PlayerCapabilities {
        available: true,
        version: Some("0.41.0".into()),
        capabilities: ids
            .iter()
            .map(|id| {
                let blocked = unsupported.iter().find(|(u, _)| u == id);
                CapabilityStatus {
                    id: (*id).into(),
                    label: (*id).into(),
                    supported: blocked.is_none(),
                    reason: blocked.map(|(_, why)| (*why).into()),
                }
            })
            .collect(),
    }
}

#[test]
fn preferences_become_mpv_options_and_defaults_stay_quiet() {
    let caps = all_capabilities(&[]);
    let (args, notes) = launch_options(
        &PlayerPreferences::default(),
        &PlaybackPolicy::default(),
        &TrackChoice::default(),
        &caps,
    );
    assert!(notes.is_empty());
    assert_eq!(args, ["--volume=100", "--hwdec=auto-safe"]);

    let prefs = PlayerPreferences {
        volume: 65.0,
        muted: true,
        speed: 1.25,
        fullscreen: true,
        aspect: AspectMode::Ratio4x3,
        audio_delay: -0.5,
        decoding: DecodingMode::Software,
        buffer_ram_mb: 256,
        buffer_seconds: 45,
        audio_compressor: true,
        ..Default::default()
    };
    let policy = PlaybackPolicy { start_paused: true, ..Default::default() };
    let carry = TrackChoice {
        audio_language: Some("ja".into()),
        subtitles_enabled: Some(true),
        subtitle_language: Some("en".into()),
        ..Default::default()
    };
    let (args, notes) = launch_options(&prefs, &policy, &carry, &caps);
    assert!(notes.is_empty());
    assert_eq!(
        args,
        [
            "--volume=65",
            "--mute=yes",
            "--speed=1.25",
            "--fullscreen=yes",
            "--keepaspect=yes",
            "--panscan=0",
            "--video-aspect-override=4:3",
            "--audio-delay=-0.5",
            "--hwdec=no",
            "--demuxer-max-bytes=256MiB",
            "--demuxer-readahead-secs=45",
            &format!("--af={COMPRESSOR_FILTER}"),
            "--pause=yes",
            "--alang=ja",
            "--slang=en",
        ]
    );
}

#[test]
fn subtitles_that_were_off_stay_off_and_unsafe_tags_are_never_passed() {
    let caps = all_capabilities(&[]);
    let off = TrackChoice { subtitles_enabled: Some(false), subtitle_language: Some("en".into()), ..Default::default() };
    let (args, _) = launch_options(&PlayerPreferences::default(), &PlaybackPolicy::default(), &off, &caps);
    assert!(args.contains(&"--sid=no".to_string()));
    assert!(!args.iter().any(|a| a.starts_with("--slang")));
    let hostile = TrackChoice {
        audio_language: Some("ja --script=/tmp/x.lua".into()),
        subtitles_enabled: Some(true),
        subtitle_language: Some("en,../x".into()),
        ..Default::default()
    };
    let (args, _) = launch_options(&PlayerPreferences::default(), &PlaybackPolicy::default(), &hostile, &caps);
    assert!(!args.iter().any(|a| a.contains("script") || a.starts_with("--slang") || a.starts_with("--alang")));
}

#[test]
fn unsupported_capabilities_are_skipped_and_explained() {
    let caps = all_capabilities(&[
        ("audio_compressor", "This mpv build has no acompressor audio filter."),
        ("hardware_decoding", "This mpv does not provide the hwdec option."),
    ]);
    let prefs = PlayerPreferences { audio_compressor: true, buffer_ram_mb: 64, ..Default::default() };
    let (args, notes) = launch_options(&prefs, &PlaybackPolicy::default(), &TrackChoice::default(), &caps);
    assert!(!args.iter().any(|a| a.starts_with("--af") || a.starts_with("--hwdec")));
    assert!(args.contains(&"--demuxer-max-bytes=64MiB".to_string()));
    assert_eq!(
        notes,
        [
            "Decoding preference was skipped: This mpv does not provide the hwdec option.",
            "Audio compressor was skipped: This mpv build has no acompressor audio filter.",
        ]
    );
}

#[test]
fn preference_validation_names_the_bad_field() {
    let bad = |prefs: PlayerPreferences, needle: &str| {
        let error = prefs.validate().unwrap_err().to_string();
        assert!(error.contains(needle), "'{error}' should mention '{needle}'");
    };
    bad(PlayerPreferences { volume: 200.0, ..Default::default() }, "volume");
    bad(PlayerPreferences { speed: 9.0, ..Default::default() }, "speed");
    bad(PlayerPreferences { audio_delay: 99.0, ..Default::default() }, "Audio delay");
    bad(PlayerPreferences { subtitle_delay: -99.0, ..Default::default() }, "Subtitle delay");
    bad(PlayerPreferences { subtitle_language: Some("en,unsafe".into()), ..Default::default() }, "Language");
    bad(PlayerPreferences { buffer_ram_mb: 100_000, ..Default::default() }, "Buffer size");
    bad(PlayerPreferences { buffer_seconds: 99_999, ..Default::default() }, "Buffer length");
    bad(PlayerPreferences { seek_step_seconds: 0, ..Default::default() }, "Seek step");
    bad(PlayerPreferences { hold_speed: 5.0, ..Default::default() }, "Hold speed");
    let old: PlayerPreferences = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old.seek_step_seconds, 10);
    assert_eq!(old.hold_speed, 2.0);
    bad(PlayerPreferences { aspect: AspectMode::Custom, ..Default::default() }, "aspect");
    PlayerPreferences::default().validate().unwrap();
}

#[test]
fn source_profile_updates_replace_priority_maps_and_stay_profile_scoped() {
    let mut settings = Settings::default();
    let config = json!({"activeProfileId":"custom", "profiles":[{
        "id":"custom", "name":"Custom", "qualityPriorities":{"720":12},
        "sourcePriorities":{"Example":3}, "preferredLanguage":"ja"
    }]});
    cloudstream_core::store::apply_settings_patch(&mut settings, &json!({"sourcePriority":config}), Some("profile"), Some("alice")).unwrap();
    let updated = json!({"activeProfileId":"custom", "profiles":[{
        "id":"custom", "name":"Custom", "qualityPriorities":{"720":12}, "sourcePriorities":{}
    }]});
    cloudstream_core::store::apply_settings_patch(&mut settings, &json!({"sourcePriority":updated}), Some("profile"), Some("alice")).unwrap();
    let profile = settings.profiles["alice"].source_priority.as_ref().unwrap().active();
    assert_eq!(profile.quality_priorities["720"], 12);
    assert!(profile.source_priorities.is_empty());
    assert!(!settings.profiles.contains_key("bob"));
}

#[test]
fn playlist_transport_preserves_order_and_timing_and_rejects_missing_segments() {
    let mut stream = Stream { kind: Some("playlist".into()), playlist: Some(vec![
        PlaylistItem { url: "https://a.test/one".into(), duration_us: Some(1_000_000), extra: Default::default() },
        PlaylistItem { url: "https://b.test/two".into(), duration_us: Some(2_000_000), extra: Default::default() },
    ]), ..Default::default() };
    let urls = vec!["http://127.0.0.1:1234/media/token/1".into(), "http://127.0.0.1:1234/media/token/2".into()];
    assert!(cloudstream_core::streams::apply_playlist_transport(&mut stream, &urls[..1]).is_err());
    assert_eq!(stream.playlist.as_ref().unwrap()[0].url, "https://a.test/one");
    cloudstream_core::streams::apply_playlist_transport(&mut stream, &urls).unwrap();
    let items = stream.playlist.as_ref().unwrap();
    assert_eq!(items[0].url, urls[0]);
    assert_eq!(items[1].url, urls[1]);
    assert_eq!(items[1].duration_us, Some(2_000_000));
    let timeline = cloudstream_core::streams::playback_target(&stream).unwrap();
    assert!(timeline.contains(&urls[0]) && timeline.contains(&urls[1]));
}

#[tokio::test]
async fn track_events_expose_metadata_and_selection_is_acknowledged_by_mpv() {
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(directory.path().to_owned());
    let mut mpv = fake_session(&player, directory.path()).await;
    let mut events = player.subscribe();
    mpv.inject.send(property("track-list", json!([
        {"id":1,"type":"audio","lang":"ja","title":"Original","codec":"aac","selected":true},
        {"id":2,"type":"sub","lang":"en","title":"English SDH","codec":"subrip","external":true,"forced":true,"hearing-impaired":true,"selected":false},
        {"id":1,"type":"video","selected":true}
    ]))).unwrap();
    let state = settle(&player, |s| s.tracks.len() == 2).await;
    let sub = state.tracks.iter().find(|t| t.kind == "sub").unwrap();
    assert_eq!(sub.language.as_deref(), Some("en"));
    assert!(sub.external && sub.forced && sub.hearing_impaired);
    assert!(matches!(tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await.unwrap().unwrap(), PlayerEvent::StateChanged { .. }));
    let saved = player.execute(PlayerCommand::SetSubtitleTrack { id: Some(2) }).await.unwrap().unwrap();
    assert_eq!(saved.subtitles_enabled, Some(true));
    assert_eq!(saved.subtitle_language.as_deref(), Some("en"));
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "sid", 2]));
    assert!(!player.state().await.tracks.iter().find(|t| t.kind == "sub").unwrap().selected);
    mpv.inject.send(property("track-list", json!([
        {"id":1,"type":"audio","lang":"ja","selected":true},
        {"id":2,"type":"sub","lang":"en","selected":true}
    ]))).unwrap();
    settle(&player, |s| s.tracks.iter().any(|t| t.kind == "sub" && t.selected)).await;
    let saved = player.execute(PlayerCommand::SetAudioTrack { id: None }).await.unwrap().unwrap();
    assert_eq!(saved.audio_enabled, Some(false));
    assert_eq!(mpv.received.recv().await.unwrap(), json!(["set_property", "aid", "no"]));
    assert!(player.execute(PlayerCommand::SetSubtitleTrack { id: Some(99) }).await.unwrap_err().to_string().contains("no longer available"));
    assert!(mpv.received.try_recv().is_err());
}

#[tokio::test]
async fn protected_auxiliary_resources_never_silently_lose_or_share_headers() {
    let dir = tempfile::tempdir().unwrap();
    let player = Player::new(dir.path().to_owned());
    let mut video = Stream { name: "Video".into(), url: "https://video.test/video".into(), ..Default::default() };
    let sub = Subtitle { lang: "English".into(), url: "https://sub.test/caption.vtt".into(), headers: [("Authorization".into(), "secret".into())].into_iter().collect(), ..Default::default() };
    let err = player.play(&video, &[sub], None, 0.0, None, None).await.unwrap_err();
    assert!(err.to_string().contains("provider media session"));
    video.headers.insert("Authorization".into(), "video-secret".into());
    let sub = Subtitle { lang: "English".into(), url: "https://sub.test/caption.vtt".into(), ..Default::default() };
    let err = player.play(&video, &[sub], None, 0.0, None, None).await.unwrap_err();
    assert!(err.to_string().contains("isolated media sessions"));
    assert!(!player.state().await.active);
}

#[test]
fn subtitle_style_validates_and_maps_to_mpv_without_overriding_ass_by_default() {
    let mut style = SubtitleStyle { font: "Noto Sans".into(), size: 48.0, color: "#aabbcc".into(),
        bold: true, italic: true, background_box: true, background_color: "#112233".into(),
        background_opacity: 50, vertical_margin: 60, ..Default::default() };
    style.validate().unwrap();
    let properties: std::collections::HashMap<_, _> = subtitle_style_properties(&style).into_iter().collect();
    assert_eq!(properties["sub-font"], json!("Noto Sans"));
    assert_eq!(properties["sub-font-size"], json!(48.0));
    assert_eq!(properties["sub-back-color"], json!("#80112233"));
    assert_eq!(properties["sub-border-style"], json!("background-box"));
    assert_eq!(properties["sub-margin-y"], json!(60));
    assert_eq!(properties["sub-ass-override"], json!("no"));
    style.override_ass = true;
    assert_eq!(subtitle_style_properties(&style).into_iter().find(|(name, _)| *name == "sub-ass-override").unwrap().1, json!("force"));
    style.color = "#oops".into();
    assert!(style.validate().is_err());
    style.color = "#ffffff".into();
    style.font = "font;--other-option".into();
    assert!(style.validate().is_err());
    style.font = "Noto Sans".into();
    style.outline_size = f64::NAN;
    assert!(style.validate().is_err());
}
