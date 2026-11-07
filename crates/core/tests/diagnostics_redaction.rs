//! Diagnostics (T32): useful failures, no secrets, bounded storage.
use cloudstream_core::diagnostics::*;

/// Secrets that must never survive redaction, in the shapes they actually turn up in.
const SECRETS: [&str; 12] = [
    "s3cr3t-t0ken-value-98765",
    "hunter2pass",
    "sessionid-ABCDEF123456",
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payloadpayload.signaturesignature",
    "4321",
    "alice@example.org",
    "0123456789abcdef0123456789abcdef0123456789abcdef",
    "supersecretsigningkey",
    "cookie-value-xyz",
    "Zm9vYmFyYmF6cXV4Zm9vYmFyYmF6cXV4Zm9v",
    "opaque-path-token-0123456789abcdef",
    "urn:btih:aabbccddeeff00112233445566778899aabbccdd",
];

fn assert_clean(text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "'{secret}' survived in: {text}");
    }
}

#[test]
fn addresses_keep_their_host_and_path_but_lose_credentials_queries_and_token_segments() {
    let out = redact("GET https://user:hunter2pass@cdn.example/v/opaque-path-token-0123456789abcdef/video.m3u8?token=s3cr3t-t0ken-value-98765&sig=supersecretsigningkey#frag failed");
    assert_eq!(out, "GET https://cdn.example/v/[redacted]/video.m3u8?[redacted] failed");
    assert_eq!(redact("see http://plain.example/a/b.mp4 now"), "see http://plain.example/a/b.mp4 now", "an address with nothing secret is left readable");
}

#[test]
fn headers_json_fields_bearers_and_identifiers_are_removed() {
    let cases = [
        "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payloadpayload.signaturesignature",
        "Cookie: sessionid-ABCDEF123456; other=1",
        "set-cookie: cookie-value-xyz; Path=/",
        "X-Auth-Token: s3cr3t-t0ken-value-98765",
        r#"{"access_token":"s3cr3t-t0ken-value-98765","user":"alice"}"#,
        r#"{"password": "hunter2pass", "pin": "4321"}"#,
        "login failed for alice@example.org (pin=4321)",
        "api_key=supersecretsigningkey&x=1",
        "hash 0123456789abcdef0123456789abcdef0123456789abcdef seen",
        "payload Zm9vYmFyYmF6cXV4Zm9vYmFyYmF6cXV4Zm9v end",
        "magnet:?xt=urn:btih:aabbccddeeff00112233445566778899aabbccdd&dn=Movie",
        "request bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payloadpayload.signaturesignature failed",
    ];
    for case in cases {
        assert_clean(&redact(case));
    }
    // Ordinary words that merely contain a secret-sounding word are untouched.
    assert_eq!(redact("monkeys like bananas; keyboard layout"), "monkeys like bananas; keyboard layout");
    assert!(redact(r#"{"access_token":"x1","user":"alice"}"#).contains("alice"), "other fields stay, so the message is still useful");
}

#[test]
fn the_home_folder_becomes_a_tilde() {
    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()).filter(|h| h.len() > 1) {
        assert_eq!(redact(&format!("could not open {home}/Videos/film.mkv")), "could not open ~/Videos/film.mkv");
    }
}

#[test]
fn recorded_events_are_redacted_bounded_and_single_line() {
    let diagnostics = Diagnostics::default();
    for secret_line in [
        "download failed: https://cdn.example/f.mp4?token=s3cr3t-t0ken-value-98765",
        "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payloadpayload.signaturesignature",
        "user alice@example.org entered pin=4321",
    ] {
        diagnostics.record("test", secret_line);
    }
    diagnostics.record("test", &format!("{}\nsecond line\u{7}", "x ".repeat(2000)));
    let events = diagnostics.events();
    assert_eq!(events.len(), 4);
    for event in &events {
        assert_clean(&event.message);
        assert!(event.message.chars().count() <= MAX_MESSAGE_CHARS + 1);
        assert!(!event.message.contains('\n') && !event.message.chars().any(char::is_control));
    }
    assert!(events[0].message.contains("download failed: https://cdn.example/f.mp4?[redacted]"), "{}", events[0].message);
    assert!(events[3].message.ends_with('…'));
}

#[test]
fn storage_is_bounded_whatever_is_logged() {
    let diagnostics = Diagnostics::default();
    for n in 0..(MAX_EVENTS * 3) {
        diagnostics.record("flood", &format!("event {n}"));
        diagnostics.record_timing("search", n as u64, "ok");
    }
    let events = diagnostics.events();
    assert_eq!(events.len(), MAX_EVENTS);
    assert_eq!(events.last().unwrap().message, format!("event {}", MAX_EVENTS * 3 - 1), "the newest entries are kept");
    assert_eq!(diagnostics.timings().len(), MAX_TIMINGS);
    diagnostics.clear();
    assert!(diagnostics.events().is_empty() && diagnostics.timings().is_empty());
}

#[test]
fn timings_carry_a_method_and_a_class_never_provider_text() {
    let diagnostics = Diagnostics::default();
    diagnostics.record_timing("loadLinks; rm -rf /", 120, "timeout");
    let timing = &diagnostics.timings()[0];
    assert_eq!(timing.method, "loadLinksrmrf");
    assert_eq!((timing.duration_ms, timing.ok, timing.outcome.as_str()), (120, false, "timeout"));
    assert_eq!(classify_error(&anyhow::anyhow!("Provider took too long: request timed out")), "timeout");
    assert_eq!(classify_error(&anyhow::anyhow!("Request cancelled")), "cancelled");
    assert_eq!(classify_error(&anyhow::anyhow!("login failed token=abc")), "error");
}

#[test]
fn an_exported_report_is_bounded_and_keeps_the_newest_entries() {
    // The live rings are capped at 500 entries, so a real report is far below the limit; this
    // feeds the safety net an oversized one (5000 full-length events, about 2 MB).
    let total = MAX_EVENTS * 10;
    let mut events = Vec::new();
    for n in 0..total {
        events.push(DiagnosticEvent { at_ms: n as u64, category: "x".into(), message: "m".repeat(MAX_MESSAGE_CHARS) });
    }
    let report = DiagnosticsReport {
        generated_at_ms: 1, app_version: "0.1.0".into(), os: "linux".into(), arch: "x86_64".into(), java_available: true, worker_available: true,
        worker_jar_sha256: Some("a".repeat(64)), mpv_version: Some("0.41.0".into()), player_unsupported: vec![], packages: vec![],
        providers: 0, disabled_providers: 0, events, timings: vec![], notes: vec![],
    };
    let bytes = report.to_json().unwrap();
    assert!(bytes.len() <= MAX_REPORT_BYTES);
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed["events"].as_array().unwrap().len() < total, "older events were dropped to fit");
    assert_eq!(parsed["events"].as_array().unwrap().last().unwrap()["atMs"], (total - 1) as u64, "the newest entry survives");
    assert_eq!(parsed["workerJarSha256"], "a".repeat(64), "structured facts such as hashes are kept");
}
