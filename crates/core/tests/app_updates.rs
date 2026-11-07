//! App update checks (T32): only a correctly signed feed is believed, and nothing is installed.
use base64::Engine;
use cloudstream_core::updates::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use std::path::Path;

fn engine() -> base64::engine::GeneralPurpose { base64::engine::general_purpose::STANDARD }
fn key(seed: u8) -> SigningKey { SigningKey::from_bytes(&[seed; 32]) }
fn public(seed: u8) -> [u8; 32] { key(seed).verifying_key().to_bytes() }

fn package(kind: &str, arch: &str) -> serde_json::Value {
    json!({ "kind": kind, "arch": arch, "url": format!("https://downloads.example/cloudstream.{kind}"), "sha256": "ab".repeat(32), "size": 52_428_800u64 })
}

fn payload() -> serde_json::Value {
    json!({ "schema": 1, "channels": {
        "stable": { "version": "0.3.0", "published": "2026-11-01", "notesUrl": "https://example.org/notes/0.3.0", "packages": [package("deb", "x86_64"), package("appimage", "x86_64"), package("deb", "aarch64")] },
        "beta":   { "version": "0.4.0-beta.1", "published": "2026-11-05", "notesUrl": "https://example.org/notes/0.4.0-beta.1", "packages": [package("deb", "x86_64")] },
    }})
}

fn sign(payload: &serde_json::Value, signer: &SigningKey) -> Vec<u8> {
    let bytes = serde_json::to_vec(payload).unwrap();
    let signature = signer.sign(&bytes);
    serde_json::to_vec(&json!({ "payload": engine().encode(&bytes), "signature": engine().encode(signature.to_bytes()) })).unwrap()
}

fn settings(channel: UpdateChannel) -> UpdateSettings { UpdateSettings { channel, feed_url: Some("https://updates.example/feed.json".into()), ..Default::default() } }

async fn run(body: Vec<u8>, settings: &UpdateSettings, current: &str, exe: &str) -> anyhow::Result<UpdateCheck> {
    check(settings, Some(public(1)), current, Path::new(exe), |_url| async move { Ok(body) }).await
}

#[tokio::test]
async fn a_correctly_signed_newer_release_is_offered_with_this_machines_packages_only() {
    let body = sign(&payload(), &key(1));
    let UpdateCheck::Available { release, managed_by_package_manager } = run(body, &settings(UpdateChannel::Stable), "0.2.0", "/home/me/cloudstream").await.unwrap() else { panic!("expected an update") };
    assert_eq!(release.version, "0.3.0");
    assert!(!managed_by_package_manager);
    assert!(release.packages.iter().all(|p| p.arch == std::env::consts::ARCH), "{:?}", release.packages);
    assert_eq!(release.notes_url, "https://example.org/notes/0.3.0");
    assert!(release.packages.iter().all(|p| p.sha256.len() == 64), "the signed checksum is shown so a download can be verified");
}

#[tokio::test]
async fn a_feed_that_is_unsigned_wrongly_signed_or_altered_is_never_believed() {
    let settings = settings(UpdateChannel::Stable);
    // Signed by someone else.
    let wrong_key = sign(&payload(), &key(2));
    let error = run(wrong_key, &settings, "0.2.0", "/x").await.unwrap_err().to_string();
    assert!(error.contains("signature is not valid"), "{error}");
    // Altered after signing: a higher version swapped in.
    let signed = sign(&payload(), &key(1));
    let mut envelope: serde_json::Value = serde_json::from_slice(&signed).unwrap();
    let mut forged = payload();
    forged["channels"]["stable"]["version"] = json!("9.9.9");
    envelope["payload"] = json!(engine().encode(serde_json::to_vec(&forged).unwrap()));
    assert!(run(serde_json::to_vec(&envelope).unwrap(), &settings, "0.2.0", "/x").await.unwrap_err().to_string().contains("signature is not valid"));
    // No signature at all, garbage, truncated signature.
    for body in [br#"{"payload":"e30="}"#.to_vec(), b"not json".to_vec(), serde_json::to_vec(&json!({ "payload": "e30=", "signature": "AAAA" })).unwrap()] {
        assert!(run(body, &settings, "0.2.0", "/x").await.is_err());
    }
}

#[tokio::test]
async fn unsafe_or_malformed_contents_are_rejected_even_when_correctly_signed() {
    let settings = settings(UpdateChannel::Stable);
    let signed = |mutate: &dyn Fn(&mut serde_json::Value)| { let mut p = payload(); mutate(&mut p); sign(&p, &key(1)) };
    let bad: Vec<(&str, Vec<u8>)> = vec![
        ("https://", signed(&|p| p["channels"]["stable"]["packages"][0]["url"] = json!("http://downloads.example/a.deb"))),
        ("user name or password", signed(&|p| p["channels"]["stable"]["packages"][0]["url"] = json!("https://u:p@downloads.example/a.deb"))),
        ("https://", signed(&|p| p["channels"]["stable"]["notesUrl"] = json!("javascript:alert(1)"))),
        ("SHA-256", signed(&|p| p["channels"]["stable"]["packages"][0]["sha256"] = json!("nothex"))),
        ("unknown package type", signed(&|p| p["channels"]["stable"]["packages"][0]["kind"] = json!("apk"))),
        ("invalid size", signed(&|p| p["channels"]["stable"]["packages"][0]["size"] = json!(0))),
        ("invalid version", signed(&|p| p["channels"]["stable"]["version"] = json!("latest"))),
        ("newer format", signed(&|p| p["schema"] = json!(2))),
    ];
    for (needle, body) in bad {
        let error = run(body, &settings, "0.2.0", "/x").await.unwrap_err().to_string();
        assert!(error.contains(needle), "'{error}' should mention '{needle}'");
    }
    let oversized = vec![b' '; MAX_FEED_BYTES + 1];
    assert!(run(oversized, &settings, "0.2.0", "/x").await.unwrap_err().to_string().contains("larger than expected"));
}

#[tokio::test]
async fn versions_channels_and_skipping_decide_what_is_offered() {
    let body = sign(&payload(), &key(1));
    let stable = settings(UpdateChannel::Stable);
    assert_eq!(run(body.clone(), &stable, "0.3.0", "/x").await.unwrap(), UpdateCheck::UpToDate { current: "0.3.0".into() }, "the same version is not an update");
    assert_eq!(run(body.clone(), &stable, "1.0.0", "/x").await.unwrap(), UpdateCheck::UpToDate { current: "1.0.0".into() }, "an older release is never offered as a downgrade");
    // Stable users never see a beta; beta users see the newest of beta and stable.
    let UpdateCheck::Available { release, .. } = run(body.clone(), &stable, "0.2.0", "/x").await.unwrap() else { panic!() };
    assert_eq!(release.version, "0.3.0");
    let UpdateCheck::Available { release, .. } = run(body.clone(), &settings(UpdateChannel::Beta), "0.2.0", "/x").await.unwrap() else { panic!() };
    assert_eq!(release.version, "0.4.0-beta.1");
    assert_eq!(run(body.clone(), &settings(UpdateChannel::Beta), "0.4.0-beta.1", "/x").await.unwrap(), UpdateCheck::UpToDate { current: "0.4.0-beta.1".into() });
    // A pre-release is older than its release.
    assert!(matches!(run(body.clone(), &stable, "0.3.0-rc.1", "/x").await.unwrap(), UpdateCheck::Available { .. }));
    // Skipping hides exactly that version, not the next one.
    let skipping = UpdateSettings { skipped_version: Some("0.3.0".into()), ..settings(UpdateChannel::Stable) };
    assert_eq!(run(body.clone(), &skipping, "0.2.0", "/x").await.unwrap(), UpdateCheck::Skipped { version: "0.3.0".into() });
    let mut newer = payload();
    newer["channels"]["stable"]["version"] = json!("0.3.1");
    let UpdateCheck::Available { release, .. } = run(sign(&newer, &key(1)), &skipping, "0.2.0", "/x").await.unwrap() else { panic!() };
    assert_eq!(release.version, "0.3.1");
    // No release on a channel.
    let mut only_stable = payload();
    only_stable["channels"].as_object_mut().unwrap().remove("stable");
    assert_eq!(run(sign(&only_stable, &key(1)), &stable, "0.2.0", "/x").await.unwrap(), UpdateCheck::NoRelease { channel: UpdateChannel::Stable });
}

#[tokio::test]
async fn a_build_without_a_channel_says_so_and_never_fetches() {
    let fetched = std::sync::atomic::AtomicBool::new(false);
    let fetch = |_url: String| { fetched.store(true, std::sync::atomic::Ordering::SeqCst); async { Ok(Vec::new()) } };
    let no_key = check(&settings(UpdateChannel::Stable), None, "0.2.0", Path::new("/x"), fetch).await.unwrap();
    assert!(matches!(&no_key, UpdateCheck::NotConfigured { reason } if reason.contains("signing key")), "{no_key:?}");
    let fetch = |_url: String| { fetched.store(true, std::sync::atomic::Ordering::SeqCst); async { Ok(Vec::new()) } };
    let no_feed = check(&UpdateSettings::default(), Some(public(1)), "0.2.0", Path::new("/x"), fetch).await.unwrap();
    assert!(matches!(&no_feed, UpdateCheck::NotConfigured { reason } if reason.contains("No update channel")), "{no_feed:?}");
    assert!(!fetched.load(std::sync::atomic::Ordering::SeqCst), "nothing is fetched without a configured channel");
    // The feed address itself must be HTTPS.
    let http = UpdateSettings { feed_url: Some("http://updates.example/feed.json".into()), ..Default::default() };
    assert!(http.validate().is_err());
    assert!(check(&http, Some(public(1)), "0.2.0", Path::new("/x"), |_| async { Ok(Vec::new()) }).await.is_err());
    assert!(fetch_feed("http://example.org/feed").await.unwrap_err().to_string().contains("https://"));
}

#[test]
fn system_folder_installs_are_left_to_the_package_manager() {
    for managed in ["/usr/bin/cloudstream-desktop", "/opt/cloudstream/cloudstream-desktop", "/usr/lib/cloudstream-desktop/app"] {
        assert!(installed_by_package_manager(Path::new(managed)), "{managed}");
    }
    for portable in ["/home/me/Downloads/cloudstream", "/usr/local/bin/cloudstream", "/tmp/x"] {
        assert!(!installed_by_package_manager(Path::new(portable)), "{portable}");
    }
}

#[test]
fn update_settings_are_validated_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let app = cloudstream_core::App::open(dir.path().join("data"), dir.path().join("res")).unwrap();
    let bad = |patch: serde_json::Value, needle: &str| {
        let error = app.store.update_settings(json!({ "updates": patch }), Some("global"), None).unwrap_err().to_string();
        assert!(error.contains(needle), "'{error}' should mention '{needle}'");
    };
    bad(json!({ "feedUrl": "ftp://x.example/feed" }), "https://");
    bad(json!({ "feedUrl": "https://u:p@x.example/feed" }), "user name");
    bad(json!({ "skippedVersion": "soon" }), "valid version");
    bad(json!({ "channel": "nightly" }), "nightly");
    let ok = app.store.update_settings(json!({ "updates": { "channel": "beta", "feedUrl": "https://updates.example/feed.json", "skippedVersion": "1.2.3" } }), Some("global"), None).unwrap();
    assert_eq!(ok.global.updates.channel, UpdateChannel::Beta);
    assert_eq!(app.store.get_settings().unwrap().global.updates.skipped_version.as_deref(), Some("1.2.3"));
}
