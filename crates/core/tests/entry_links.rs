//! Deep links, shared links and command-line arguments (T27). Strict parsing, no side effects.
use cloudstream_core::entry::*;
use std::io::Write;

fn ok(input: &str) -> EntryAction {
    parse_entry(input).unwrap_or_else(|e| panic!("{input} should parse: {e}"))
}
fn refused(input: &str, needle: &str) {
    let error = parse_entry(input).err().unwrap_or_else(|| panic!("{input:?} should be refused")).to_string();
    assert!(error.contains(needle), "'{error}' should mention '{needle}' for {input:?}");
}
fn b64(text: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[test]
fn repository_links_become_add_repository_offers() {
    assert_eq!(ok("https://cs.repo/?raw.githubusercontent.com/me/repo/main/repo.json"), EntryAction::AddRepository { input: "https://raw.githubusercontent.com/me/repo/main/repo.json".into() });
    assert_eq!(ok("cloudstreamrepo://example.test/repo.json"), EntryAction::AddRepository { input: "cloudstreamrepo://example.test/repo.json".into() });
    assert_eq!(ok("CLOUDSTREAMREPO://!shortcode"), EntryAction::AddRepository { input: "CLOUDSTREAMREPO://!shortcode".into() }, "short codes go to the repository resolver");
    refused("https://cs.repo/", "no address");
    refused("https://cs.repo/?", "no address");
    refused("cloudstreamrepo://", "no address");
    refused("cloudstreamrepo://exa mple", "not valid");
    refused("https://cs.repo/?user:pw@host.test/x.json", "credentials");
}

#[test]
fn search_links_are_decoded_and_bounded() {
    assert_eq!(ok("cloudstreamsearch://big%20buck+bunny"), EntryAction::Search { query: "big buck bunny".into() });
    assert_eq!(ok("cloudstreamsearch://%E6%97%A5%E6%9C%AC%E8%AA%9E"), EntryAction::Search { query: "日本語".into() });
    refused("cloudstreamsearch://", "empty");
    refused("cloudstreamsearch://%ZZ", "invalid %");
    refused("cloudstreamsearch://%FF%FE", "UTF-8");
    refused("cloudstreamsearch://a%00b", "control");
    refused(&format!("cloudstreamsearch://{}", "a".repeat(201)), "longer than 200");
}

#[test]
fn player_links_carry_a_validated_address_and_a_name() {
    // The address is itself percent-encoded once; a space inside it must be encoded again (%2520).
    let encoded = "https%3A%2F%2Fexample.test%2Fa%2520b%2Fvideo.mp4%3Ftoken%3Dx%26y%3D1";
    assert_eq!(ok(&format!("cloudstreamplayer://{encoded}?name=My%20Movie")), EntryAction::PlayUrl { url: "https://example.test/a%20b/video.mp4?token=x&y=1".to_string(), name: Some("My Movie".into()) });
    refused("cloudstreamplayer://https%3A%2F%2Fexample.test%2Fa%20b.mp4", "not a valid web address");
    assert_eq!(ok("cloudstreamplayer://https%3A%2F%2Fexample.test%2Fv.mp4"), EntryAction::PlayUrl { url: "https://example.test/v.mp4".into(), name: None });
    refused("cloudstreamplayer://javascript%3Aalert(1)", "not usable");
    refused("cloudstreamplayer://file%3A%2F%2F%2Fetc%2Fpasswd", "not usable");
    refused("cloudstreamplayer://ftp%3A%2F%2Fexample.test%2Fv.mp4", "not usable");
    refused("cloudstreamplayer://", "not usable");
}

#[test]
fn shared_links_name_a_source_and_a_page() {
    let link = format!("csshare:{}?{}", b64("Blender Open Movies"), b64("https://www.blender.org/movie/1?x=a b".replace(' ', "%20").as_str()));
    assert_eq!(ok(&link), EntryAction::OpenTitle { provider_name: "Blender Open Movies".into(), url: "https://www.blender.org/movie/1?x=a%20b".into() });
    refused("csshare:abc", "incomplete");
    refused(&format!("csshare:{}?{}", b64("Src"), b64("javascript:alert(1)")), "not usable");
    refused(&format!("csshare:{}?{}", b64(""), b64("https://x.test/a")), "empty");
    refused("csshare:!!!?!!!", "not valid");
}

#[test]
fn continue_watching_links_never_guess_an_android_item() {
    assert_eq!(ok("cloudstreamcontinuewatching://latest"), EntryAction::ResumeLatest);
    refused("cloudstreamcontinuewatching://123456", "Android app");
    refused("cloudstreamcontinuewatching://-42", "Android app");
    refused("cloudstreamcontinuewatching://abc", "not valid");
}

#[test]
fn web_addresses_are_offered_for_confirmation_not_acted_on() {
    assert_eq!(ok("https://example.test/title/1"), EntryAction::OpenUrl { url: "https://example.test/title/1".into() });
    assert_eq!(ok("  http://example.test/a.mp4  "), EntryAction::OpenUrl { url: "http://example.test/a.mp4".into() });
    refused("https://user:pass@example.test/", "credentials");
    refused("https://exa mple.test/", "not a valid web address");
}

#[test]
fn unsafe_unsupported_and_oversized_input_is_refused_with_a_reason() {
    refused("", "empty");
    refused("   ", "empty");
    refused("javascript:alert(1)", "not supported");
    refused("data:text/html,<script>", "not supported");
    refused("ftp://example.test/a.mp4", "not supported");
    refused("cloudstreamapp:oauth?code=1", "sign-in");
    refused("magnet:?xt=urn:btih:abc", "torrent backend");
    refused("https://example.test/\u{7}bell", "control");
    refused(&format!("https://example.test/{}", "a".repeat(MAX_ARGUMENT_BYTES)), "too long");
    refused("relative/video.mp4", "full path");
}

#[test]
fn local_files_are_resolved_checked_and_limited_to_media() {
    let dir = tempfile::tempdir().unwrap();
    let movie = dir.path().join("My Movie 日本語 (2020).mkv");
    std::fs::File::create(&movie).unwrap().write_all(b"x").unwrap();
    let canonical = std::fs::canonicalize(&movie).unwrap().display().to_string();
    assert_eq!(ok(movie.to_str().unwrap()), EntryAction::PlayFile { path: canonical.clone() });
    // file:// URIs are percent-decoded, so paths with spaces and non-ASCII names work.
    let uri = format!("file://{}", movie.to_str().unwrap().replace(' ', "%20").replace('日', "%E6%97%A5").replace('本', "%E6%9C%AC").replace('語', "%E8%AA%9E"));
    assert_eq!(ok(&uri), EntryAction::PlayFile { path: canonical.clone() });
    // A link to a media file resolves to the real file, so the confirmed path is what plays.
    let link = dir.path().join("shortcut.mp4");
    std::os::unix::fs::symlink(&movie, &link).unwrap();
    assert_eq!(ok(link.to_str().unwrap()), EntryAction::PlayFile { path: canonical });
    // Names that look like shell syntax are just names.
    let odd = dir.path().join("a'; rm -rf ~; echo '.mp4");
    std::fs::write(&odd, b"x").unwrap();
    assert!(matches!(ok(odd.to_str().unwrap()), EntryAction::PlayFile { .. }));

    let text = dir.path().join("notes.txt");
    std::fs::write(&text, b"x").unwrap();
    refused(text.to_str().unwrap(), "not a video or audio file");
    refused(dir.path().join("missing.mkv").to_str().unwrap(), "does not exist");
    let folder = dir.path().join("folder.mp4");
    std::fs::create_dir(&folder).unwrap();
    refused(folder.to_str().unwrap(), "not a regular file");
    refused("/etc/passwd", "not a video or audio file");
}

#[test]
fn command_line_batches_report_each_problem_and_skip_flags() {
    let dir = tempfile::tempdir().unwrap();
    let movie = dir.path().join("a.mp4");
    std::fs::write(&movie, b"x").unwrap();
    let args = vec![
        "--some-webview-flag".to_string(),
        "cloudstreamsearch://one".to_string(),
        "bogus:thing".to_string(),
        movie.display().to_string(),
        "cloudstreamrepo://".to_string(),
    ];
    let parsed = parse_arguments(&args);
    assert_eq!(parsed.actions.len(), 2);
    assert_eq!(parsed.errors.len(), 2);
    assert!(parsed.errors[0].contains("bogus:"));
}

#[test]
fn the_queue_keeps_order_drops_repeats_is_bounded_and_hands_over_once() {
    let queue = EntryQueue::default();
    let search = |n: usize| EntryAction::Search { query: format!("q{n}") };
    assert!(queue.push(search(1)));
    assert!(!queue.push(search(1)), "a link delivered twice acts once");
    assert!(queue.push(search(2)));
    assert_eq!(queue.take(), vec![search(1), search(2)]);
    assert!(queue.take().is_empty(), "handed over exactly once");
    for n in 0..MAX_QUEUED_ACTIONS {
        assert!(queue.push(search(n)));
    }
    assert!(!queue.push(search(999)), "a flood of links cannot grow the queue without bound");
    assert_eq!(queue.len(), MAX_QUEUED_ACTIONS);
}

#[test]
fn the_packaged_desktop_file_registers_exactly_the_schemes_the_parser_handles() {
    let template = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src-tauri/linux/cloudstream-desktop.desktop")).unwrap();
    let mime_line = template.lines().find(|l| l.starts_with("MimeType=")).expect("a MimeType line");
    let registered: Vec<&str> = mime_line["MimeType=".len()..].split(';').filter(|m| !m.is_empty()).collect();
    for scheme in HANDLED_SCHEMES {
        assert!(registered.contains(&format!("x-scheme-handler/{scheme}").as_str()), "{scheme} is parsed but not registered");
    }
    for mime in &registered {
        if let Some(scheme) = mime.strip_prefix("x-scheme-handler/") {
            assert!(HANDLED_SCHEMES.contains(&scheme), "{scheme} is registered but the parser does not handle it");
        }
    }
    assert!(template.lines().any(|l| l.starts_with("Exec=") && l.ends_with(" %U")), "links and files arrive as arguments");
    // The deb bundle must actually use the template.
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src-tauri/tauri.conf.json")).unwrap()).unwrap();
    assert_eq!(config["bundle"]["linux"]["deb"]["desktopTemplate"], "linux/cloudstream-desktop.desktop");
    // Media types listed must be ones the app will actually open.
    for mime in registered.iter().filter(|m| m.starts_with("video/") || m.starts_with("audio/")) {
        assert!(!mime.contains('*'), "{mime}");
    }
}

#[test]
fn the_internal_open_media_action_cannot_be_produced_by_any_external_text() {
    for attempt in ["open_media", "open_media:p:u", "{\"type\":\"open_media\",\"providerId\":\"p\",\"url\":\"u\"}", "cloudstreamopen://p/u"] {
        assert!(parse_entry(attempt).is_err(), "{attempt}");
    }
}
