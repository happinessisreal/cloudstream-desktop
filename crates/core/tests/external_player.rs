//! External player launching (T27): argv arrays only, honest about headers, no shell anywhere.
use cloudstream_core::{external_player::*, models::*};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::Path};

/// A fake "player": a script that writes each argument it receives on its own line.
fn fake_player(dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\n", dir.join(format!("{name}.argv")).display())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn stream(url: &str, headers: &[(&str, &str)]) -> Stream {
    Stream { name: "S".into(), url: url.into(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(), ..Default::default() }
}

fn custom(program: &Path, args: &[&str]) -> ExternalPlayerSettings {
    ExternalPlayerSettings { kind: ExternalPlayerKind::Custom, program: Some(program.display().to_string()), args: args.iter().map(|s| s.to_string()).collect(), extra: Default::default() }
}

fn wait_for(path: &Path) -> String {
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(path) {
            if !text.is_empty() {
                return text;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    panic!("the player never ran: {}", path.display());
}

#[test]
fn a_hostile_title_and_address_arrive_as_plain_text_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_player(dir.path(), "player");
    let marker = dir.path().join("pwned");
    // Quotes, command substitution and shell separators in every value the app does not control.
    let title = format!("\"; touch {} #; $(touch {}) `touch {}` & echo 'x'", marker.display(), marker.display(), marker.display());
    let url = format!("https://example.test/v.mp4?a=1&b=$(touch {})&c=`id`;'\"", marker.display());
    let s = stream(&url, &[("Referer", &format!("https://r.test/\"$(touch {})", marker.display()))]);
    let settings = custom(&program, &["--title={title}", "--ref", "{referer}", "--start={start}", "{url}"]);
    let plan = plan_launch(&settings, &LaunchRequest { stream: &s, title: &title, start_seconds: 83.9, subtitles: &[] }).unwrap();
    assert_eq!(plan.args[0], format!("--title={title}"), "substituted inside one argument, never split");
    assert_eq!(plan.args[1], "--ref");
    assert_eq!(plan.args[3], "--start=83");
    assert_eq!(plan.args.last().unwrap(), &url);
    assert_eq!(plan.args.len(), 5);
    launch(&plan).unwrap();
    let received = wait_for(&dir.path().join("player.argv"));
    let lines: Vec<&str> = received.lines().collect();
    assert_eq!(lines.len(), 5, "{received}");
    assert_eq!(lines[0], format!("--title={title}"));
    assert_eq!(lines[4], url);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!marker.exists(), "nothing in a title, address or header may ever be executed");
}

#[test]
fn placeholders_without_a_value_drop_the_whole_argument_and_unknown_ones_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_player(dir.path(), "p");
    let s = stream("https://example.test/v.mp4", &[]);
    let request = LaunchRequest { stream: &s, title: "T", start_seconds: 0.0, subtitles: &[] };
    let plan = plan_launch(&custom(&program, &["--referrer={referer}", "{referer}", "{subtitle}", "{url}"]), &request).unwrap();
    // "--referrer={referer}" keeps its text with an empty value; a bare empty placeholder is dropped.
    assert_eq!(plan.args, vec!["--referrer=".to_string(), "https://example.test/v.mp4".to_string()]);
    let unknown = custom(&program, &["{url}", "{password}"]).validate().unwrap_err().to_string();
    assert!(unknown.contains("Unknown placeholder {password}"), "{unknown}");
    assert!(custom(&program, &["--fullscreen"]).validate().unwrap_err().to_string().contains("{url}"));
    let mut relative = custom(&program, &["{url}"]);
    relative.program = Some("bin/player".into());
    assert!(relative.validate().is_err());
    assert!(custom(&program, &["{url}\n"]).validate().is_err(), "line breaks are refused");
}

#[test]
fn headers_a_player_cannot_carry_are_reported_not_silently_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_player(dir.path(), "p");
    let s = stream("https://example.test/v.mp4", &[("Cookie", "a=b"), ("X-Token", "1"), ("Referer", "https://r.test/")]);
    let plan = plan_launch(&custom(&program, &["{url}"]), &LaunchRequest { stream: &s, title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap();
    let warning = plan.warnings.join(" ");
    assert!(warning.contains("Cookie") && warning.contains("X-Token") && warning.contains("Referer"), "{warning}");
    // With the placeholder present the referer is carried and no longer reported.
    let plan = plan_launch(&custom(&program, &["--r={referer}", "{url}"]), &LaunchRequest { stream: &s, title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap();
    assert!(!plan.warnings.join(" ").contains("Referer"));
    assert_eq!(plan.args[0], "--r=https://r.test/");
}

#[test]
fn vlc_gets_referrer_user_agent_start_and_public_subtitles_and_refuses_other_headers() {
    let dir = tempfile::tempdir().unwrap();
    let vlc = fake_player(dir.path(), "vlc");
    let settings = ExternalPlayerSettings { kind: ExternalPlayerKind::Vlc, program: Some(vlc.display().to_string()), args: vec![], extra: Default::default() };
    let s = stream("https://example.test/v.m3u8", &[("Referer", "https://r.test/"), ("User-Agent", "Mozilla/5.0 (X11)")]);
    let subs = vec![
        Subtitle { lang: "en".into(), url: "https://example.test/en.srt".into(), ..Default::default() },
        Subtitle { lang: "fr".into(), url: "https://example.test/fr.srt".into(), headers: [("Cookie".to_string(), "x".to_string())].into(), ..Default::default() },
    ];
    let plan = plan_launch(&settings, &LaunchRequest { stream: &s, title: "T", start_seconds: 125.4, subtitles: &subs }).unwrap();
    assert_eq!(plan.args, ["--start-time=125", "--http-referrer=https://r.test/", "--http-user-agent=Mozilla/5.0 (X11)", "--sub-file=https://example.test/en.srt", "--", "https://example.test/v.m3u8"]);
    assert!(plan.warnings[0].contains("1 subtitle"), "{:?}", plan.warnings);

    let protected = stream("https://example.test/v.mp4", &[("Cookie", "a=b"), ("Authorization", "Bearer t")]);
    let error = plan_launch(&settings, &LaunchRequest { stream: &protected, title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap_err().to_string();
    assert!(error.contains("VLC cannot send these request headers") && error.contains("Cookie") && error.contains("Authorization"), "{error}");
    assert!(!error.contains("Bearer"), "header values never appear in messages");

    // Address cannot be read as an option: it follows "--".
    let flaggy = stream("https://example.test/--fullscreen", &[]);
    let plan = plan_launch(&settings, &LaunchRequest { stream: &flaggy, title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap();
    assert_eq!(&plan.args[plan.args.len() - 2..], ["--", "https://example.test/--fullscreen"]);
}

#[test]
fn streams_that_only_work_inside_the_app_are_not_handed_out() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake_player(dir.path(), "p");
    let settings = custom(&program, &["{url}"]);
    let request = |s: &Stream| plan_launch(&settings, &LaunchRequest { stream: s, title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap_err().to_string();
    let mut protected = stream("https://example.test/v.mp4", &[]);
    protected.resource_handle = Some("handle".into());
    assert!(request(&protected).contains("protects its links"));
    let mut playlist = stream("", &[]);
    playlist.kind = Some("playlist".into());
    playlist.playlist = Some(vec![PlaylistItem { url: "https://example.test/1.ts".into(), duration_us: None, extra: Default::default() }]);
    assert!(request(&Stream { url: "https://example.test/x".into(), ..playlist }).contains("playlists"));
    let mut audio = stream("https://example.test/v.mp4", &[]);
    audio.audio_tracks = vec![AudioTrack { url: "https://example.test/a".into(), headers: Default::default(), resource_handle: None, extra: Default::default() }];
    assert!(request(&audio).contains("separate audio"));
    assert!(request(&stream("file:///etc/passwd", &[])).contains("HTTP"));
    assert!(plan_launch(&ExternalPlayerSettings::default(), &LaunchRequest { stream: &stream("https://example.test/v.mp4", &[]), title: "T", start_seconds: 0.0, subtitles: &[] }).unwrap_err().to_string().contains("No external player"));
}

#[test]
fn a_missing_or_non_executable_program_is_reported_before_anything_runs() {
    let dir = tempfile::tempdir().unwrap();
    let not_exec = dir.path().join("plain.sh");
    std::fs::write(&not_exec, b"#!/bin/sh\n").unwrap();
    let s = stream("https://example.test/v.mp4", &[]);
    let request = LaunchRequest { stream: &s, title: "T", start_seconds: 0.0, subtitles: &[] };
    assert!(plan_launch(&custom(&not_exec, &["{url}"]), &request).unwrap_err().to_string().contains("not found or is not executable"));
    assert!(plan_launch(&custom(&dir.path().join("absent"), &["{url}"]), &request).is_err());
    let vlc_missing = ExternalPlayerSettings { kind: ExternalPlayerKind::Vlc, program: Some(dir.path().join("no-vlc").display().to_string()), ..Default::default() };
    assert!(plan_launch(&vlc_missing, &request).unwrap_err().to_string().contains("VLC was not found"));
}

#[test]
fn settings_are_validated_and_detection_only_reports_real_programs() {
    let dir = tempfile::tempdir().unwrap();
    let app = cloudstream_core::App::open(dir.path().join("data"), dir.path().join("res")).unwrap();
    let bad = |patch: serde_json::Value, needle: &str| {
        let error = app.store.update_settings(serde_json::json!({ "externalPlayer": patch }), Some("global"), None).unwrap_err().to_string();
        assert!(error.contains(needle), "'{error}' should mention '{needle}'");
    };
    bad(serde_json::json!({ "kind": "custom" }), "Choose the program");
    bad(serde_json::json!({ "kind": "custom", "program": "mpv", "args": ["--fullscreen"] }), "{url}");
    bad(serde_json::json!({ "kind": "custom", "program": "mpv", "args": ["{url}", "{nope}"] }), "Unknown placeholder");
    bad(serde_json::json!({ "program": "a/b" }), "full path");
    let ok = app.store.update_settings(serde_json::json!({ "externalPlayer": { "kind": "custom", "program": "mpv", "args": ["--title={title}", "{url}"] } }), Some("global"), None).unwrap();
    assert_eq!(ok.global.external_player.kind, ExternalPlayerKind::Custom);
    for found in detect_players() {
        assert!(std::path::Path::new(&found.program).is_absolute());
    }
}
