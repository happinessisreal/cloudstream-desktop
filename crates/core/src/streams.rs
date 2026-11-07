//! Stream selection policy and playback timeline adapters (T16).
//!
//! Everything here is pure so that ranking and timeline construction can be tested with
//! deterministic fixtures. Streams are never rewritten: ranking returns indexes into the caller's
//! slice so URLs, headers, referers and opaque extractor data round-trip untouched.

use crate::models::*;
use crate::repository::network_url;
use anyhow::{bail, Result};
use std::cmp::Reverse;
use std::collections::HashSet;

/// Internal marker for a file the user opened from their own disk.
pub const LOCAL_FILE_KIND: &str = "local_file";

/// Upstream's `Qualities.Unknown` value. It is a sentinel, not a 400p stream.
pub const UNKNOWN_QUALITY_SENTINEL: i32 = 400;

/// Resolution buckets and Android's default priorities (`Qualities.defaultPriority`).
const KNOWN_QUALITIES: [(i32, i32); 8] = [
    (144, 0),
    (240, 2),
    (360, 3),
    (480, 4),
    (720, 5),
    (1080, 6),
    (1440, 7),
    (2160, 8),
];

/// Android gives `Qualities.Unknown` a default priority of 4.
pub const UNKNOWN_QUALITY_PRIORITY: i32 = 4;

/// A link with a real, positive resolution. `0` means Auto/adaptive and `400` is upstream's
/// "unknown" sentinel; neither implies a resolution.
pub fn is_known_quality(quality: i32) -> bool {
    quality > 0 && quality != UNKNOWN_QUALITY_SENTINEL
}

/// The upstream label for a quality value: `Auto`, `Unknown`, `4K` or `{n}p`.
pub fn quality_label(quality: i32) -> String {
    if quality == 0 {
        "Auto".into()
    } else if !is_known_quality(quality) {
        "Unknown".into()
    } else if quality == 2160 {
        "4K".into()
    } else {
        format!("{quality}p")
    }
}

/// The priority bucket used for ranking. Unknown values stay `unknown`; known values (even odd
/// ones such as 540) rank with the nearest standard bucket, lower on a tie.
pub fn quality_bucket(quality: i32) -> String {
    if !is_known_quality(quality) {
        return "unknown".into();
    }
    KNOWN_QUALITIES
        .iter()
        .min_by_key(|(value, _)| (value - quality).abs())
        .map(|(value, _)| value.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub fn default_quality_priority(bucket: &str) -> i32 {
    if bucket == "unknown" {
        return UNKNOWN_QUALITY_PRIORITY;
    }
    bucket
        .parse::<i32>()
        .ok()
        .and_then(|value| KNOWN_QUALITIES.iter().find(|(v, _)| *v == value))
        .map(|(_, priority)| *priority)
        .unwrap_or(UNKNOWN_QUALITY_PRIORITY)
}

pub fn quality_priority(profile: &SourceProfile, quality: i32) -> i32 {
    let bucket = quality_bucket(quality);
    profile
        .quality_priorities
        .get(&bucket)
        .copied()
        .unwrap_or_else(|| default_quality_priority(&bucket))
}

/// The name a link is ranked by: the extractor/source name, falling back to the display name.
pub fn source_name(stream: &Stream) -> &str {
    stream
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(stream.name.trim())
}

/// Identity of one mirror within a playback attempt. URLs may carry expiring tokens, so this is
/// only used inside a single playback context, never persisted.
pub fn stream_key(stream: &Stream) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        stream.source.as_deref().unwrap_or(""),
        stream.name,
        stream.url
    )
}

pub fn active_stream(stream: &Stream) -> ActiveStream {
    ActiveStream {
        name: stream.name.clone(),
        source: stream.source.clone(),
        quality: stream.quality,
        quality_label: quality_label(stream.quality),
        kind: stream.kind.clone(),
        key: stream_key(stream),
    }
}

const LANGUAGES: [(&str, &str); 24] = [
    ("en", "english"),
    ("ja", "japanese"),
    ("es", "spanish"),
    ("fr", "french"),
    ("de", "german"),
    ("it", "italian"),
    ("pt", "portuguese"),
    ("ru", "russian"),
    ("ko", "korean"),
    ("zh", "chinese"),
    ("ar", "arabic"),
    ("hi", "hindi"),
    ("tr", "turkish"),
    ("nl", "dutch"),
    ("pl", "polish"),
    ("sv", "swedish"),
    ("id", "indonesian"),
    ("th", "thai"),
    ("vi", "vietnamese"),
    ("he", "hebrew"),
    ("fa", "persian"),
    ("bn", "bengali"),
    ("uk", "ukrainian"),
    ("el", "greek"),
];

/// `(code, english name)` for a user supplied preference such as `en`, `en-US` or `English`.
fn resolve_language(preference: &str) -> Option<(String, String)> {
    let wanted = preference.trim().to_lowercase();
    let primary = wanted.split(['-', '_']).next().unwrap_or("");
    LANGUAGES
        .iter()
        .find(|(code, name)| *code == primary || *name == wanted)
        .map(|(code, name)| (code.to_string(), name.to_string()))
}

/// Whether a stream declares or advertises the preferred language. This is a tie-break only:
/// upstream links carry no language field, so it checks an explicit `lang`/`language`/`langTag`
/// extra field first and then whole-word language names (or upper-case codes such as `[EN]`) in
/// the link's labels. Anything else is "no match", never a guess.
pub fn language_matches(stream: &Stream, preference: &str) -> bool {
    let Some((code, name)) = resolve_language(preference) else {
        return false;
    };
    for key in ["lang", "language", "langTag"] {
        if let Some(value) = stream.extra.get(key).and_then(|v| v.as_str()) {
            let value = value.trim().to_lowercase();
            if value.split(['-', '_']).next() == Some(code.as_str()) || value == name {
                return true;
            }
        }
    }
    let labels = format!("{} {}", stream.name, stream.source.as_deref().unwrap_or(""));
    labels
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            token.to_lowercase() == name
                || (token.len() == 2 && token.chars().all(|c| c.is_ascii_uppercase()) && token.to_lowercase() == code)
        })
}

/// Rank mirrors for the profile. Order is: combined priority (quality + source) descending,
/// preferred-language match, higher known resolution, then source and display name, and finally
/// the provider's own order, so equal inputs always produce the same output.
pub fn rank_streams(
    streams: &[Stream],
    profile: &SourceProfile,
    errored: &HashSet<String>,
) -> Vec<RankedStream> {
    let mut ranked: Vec<(RankedStream, i32, String)> = streams
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let key = stream_key(stream);
            let priority =
                quality_priority(profile, stream.quality) + profile.source_priority(source_name(stream));
            let is_errored = errored.contains(&key);
            let unavailable = crate::drm::unavailable_reason(stream);
            let usable = !(profile.hide_negative_sources && priority < 0)
                && !(profile.hide_error_sources && is_errored)
                && unavailable.is_none();
            let language_match = profile
                .preferred_language
                .as_deref()
                .is_some_and(|lang| language_matches(stream, lang));
            let known_value = if is_known_quality(stream.quality) {
                stream.quality
            } else {
                0
            };
            (
                RankedStream {
                    index,
                    key,
                    priority,
                    usable,
                    quality_label: quality_label(stream.quality),
                    quality_known: is_known_quality(stream.quality),
                    language_match,
                    errored: is_errored,
                    unavailable,
                },
                known_value,
                format!("{}\u{1f}{}", source_name(stream).to_lowercase(), stream.name.to_lowercase()),
            )
        })
        .collect();
    ranked.sort_by_key(|(r, known_value, names)| {
        (
            Reverse(r.priority),
            Reverse(r.language_match),
            Reverse(*known_value),
            names.clone(),
            r.index,
        )
    });
    ranked.into_iter().map(|(r, _, _)| r).collect()
}

/// The stream to start automatically: the best usable mirror that has not already failed.
/// Respect a profile that excludes every mirror instead of selecting a hidden source.
pub fn best_stream_index(ranked: &[RankedStream]) -> Option<usize> {
    ranked
        .iter()
        .find(|r| r.usable && !r.errored)
        .map(|r| r.index)
}

/// Build an mpv EDL timeline for an `ExtractorLinkPlayList`: one entry per segment, in order,
/// with known durations so seeking and the scrubber work across segment boundaries.
pub fn playlist_edl(items: &[PlaylistItem]) -> Result<String> {
    if items.is_empty() {
        bail!("This playlist has no segments to play");
    }
    let mut parts = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        network_url(&item.url)
            .map_err(|e| anyhow::anyhow!("Playlist segment {} is not playable: {e}", position + 1))?;
        // `%N%` length-prefixes the URL, so ',' ';' and '%' inside it need no escaping.
        let mut part = format!("%{}%{}", item.url.len(), item.url);
        if let Some(duration_us) = item.duration_us.filter(|d| *d > 0) {
            part.push_str(&format!(",length={:.6}", duration_us as f64 / 1_000_000.0));
        }
        parts.push(part);
    }
    Ok(format!("edl://{}", parts.join(";")))
}

/// Replace each provider segment with its session-scoped transport URL without changing timing.
pub fn apply_playlist_transport(stream: &mut Stream, urls: &[String]) -> Result<()> {
    let items = stream.playlist.as_mut().ok_or_else(|| anyhow::anyhow!("Playlist has no segments"))?;
    if items.is_empty() || items.len() != urls.len() {
        bail!("The provider media session cannot proxy every playlist segment. Update the worker and reload the sources.");
    }
    for url in urls { network_url(url)?; }
    for (item, url) in items.iter_mut().zip(urls) { item.url = url.clone(); }
    Ok(())
}

/// Whether the stream is a multi-segment playlist that needs the timeline adapter.
pub fn is_playlist(stream: &Stream) -> bool {
    stream.kind.as_deref() == Some("playlist") || stream.playlist.as_ref().is_some_and(|p| !p.is_empty())
}

/// What to hand to mpv for this stream.
pub fn playback_target(stream: &Stream) -> Result<String> {
    // Only the app itself may start a local file (`Player::play_local_file`); a provider that
    // labels its stream this way must not be able to point mpv at the user's files.
    if stream.kind.as_deref() == Some(LOCAL_FILE_KIND) {
        bail!("This stream type is not supported");
    }
    if is_playlist(stream) {
        return playlist_edl(stream.playlist.as_deref().unwrap_or(&[]));
    }
    if stream.url.starts_with("magnet:") {
        if !stream.url.starts_with("magnet:?") {
            bail!("Invalid magnet URI");
        }
        return Ok(stream.url.clone());
    }
    network_url(&stream.url)?;
    Ok(stream.url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(name: &str, source: &str, quality: i32) -> Stream {
        Stream {
            name: name.into(),
            url: format!("https://example.com/{name}"),
            quality,
            source: Some(source.into()),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_quality_is_never_mapped_to_a_resolution() {
        assert_eq!(quality_label(0), "Auto");
        assert_eq!(quality_label(400), "Unknown");
        assert_eq!(quality_label(-1), "Unknown");
        assert_eq!(quality_bucket(0), "unknown");
        assert_eq!(quality_bucket(400), "unknown");
        assert_eq!(quality_bucket(-5), "unknown");
        assert!(!is_known_quality(0));
        assert_eq!(quality_label(2160), "4K");
        assert_eq!(quality_label(540), "540p");
        // 390 must not collapse into the unknown sentinel the way a naive nearest match would.
        assert_eq!(quality_bucket(390), "360");
        assert_eq!(quality_bucket(600), "480", "ties go to the lower bucket");
    }

    #[test]
    fn android_default_priorities_are_preserved() {
        let profile = SourceProfile::default_profile();
        assert_eq!(quality_priority(&profile, 1080), 6);
        assert_eq!(quality_priority(&profile, 144), 0);
        assert_eq!(quality_priority(&profile, 0), 4);
        assert_eq!(profile.source_priority("anything"), 1);
    }

    #[test]
    fn ranking_is_deterministic_and_tie_breaks_do_not_depend_on_input_order() {
        let profile = SourceProfile::default_profile();
        let a = stream("alpha", "Zeta", 720);
        let b = stream("beta", "Alpha", 720);
        let c = stream("gamma", "Mid", 1080);
        let forward = rank_streams(&[a.clone(), b.clone(), c.clone()], &profile, &HashSet::new());
        let backward = rank_streams(&[c.clone(), b.clone(), a.clone()], &profile, &HashSet::new());
        let names = |streams: &[Stream], ranked: &[RankedStream]| {
            ranked.iter().map(|r| streams[r.index].name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names(&[a.clone(), b.clone(), c.clone()], &forward), ["gamma", "beta", "alpha"]);
        assert_eq!(names(&[c, b, a], &backward), ["gamma", "beta", "alpha"]);
    }

    #[test]
    fn source_priority_overrides_resolution_and_unknown_ranks_without_a_resolution() {
        let mut profile = SourceProfile::default_profile();
        profile.source_priorities.insert("fast".into(), 5);
        let streams = [stream("hd", "slow", 1080), stream("sd", "FAST", 360), stream("auto", "slow", 0)];
        let ranked = rank_streams(&streams, &profile, &HashSet::new());
        assert_eq!(ranked[0].index, 1, "360p with +5 source priority beats 1080p");
        let auto = ranked.iter().find(|r| r.index == 2).unwrap();
        assert!(!auto.quality_known);
        assert_eq!(auto.quality_label, "Auto");
        assert_eq!(auto.priority, 4 + 1);
    }

    #[test]
    fn language_is_a_tie_break_not_an_override() {
        let mut profile = SourceProfile::default_profile();
        profile.preferred_language = Some("ja".into());
        let mut explicit = stream("one", "A", 720);
        explicit.extra.insert("language".into(), serde_json::json!("ja-JP"));
        let by_label = stream("Server [JA]", "B", 720);
        let other = stream("Server English", "C", 720);
        let better = stream("Server", "D", 1080);
        let streams = [other, by_label, explicit, better];
        let ranked = rank_streams(&streams, &profile, &HashSet::new());
        let order: Vec<usize> = ranked.iter().map(|r| r.index).collect();
        assert_eq!(order, [3, 2, 1, 0]);
        assert!(ranked[1].language_match && ranked[2].language_match && !ranked[3].language_match);
        // "it" the English word must not be treated as Italian.
        let mut italian = SourceProfile::default_profile();
        italian.preferred_language = Some("it".into());
        assert!(!language_matches(&stream("Play it now", "x", 720), "it"));
        assert!(language_matches(&stream("Server [IT]", "x", 720), "it"));
        let _ = italian;
    }

    #[test]
    fn hidden_and_errored_sources_follow_the_profile() {
        let mut profile = SourceProfile::default_profile();
        profile.source_priorities.insert("bad".into(), -20);
        profile.hide_negative_sources = true;
        profile.hide_error_sources = true;
        let streams = [stream("one", "bad", 1080), stream("two", "ok", 720), stream("three", "ok", 480)];
        let mut errored = HashSet::new();
        errored.insert(stream_key(&streams[1]));
        let ranked = rank_streams(&streams, &profile, &errored);
        assert!(!ranked.iter().find(|r| r.index == 0).unwrap().usable);
        assert!(!ranked.iter().find(|r| r.index == 1).unwrap().usable);
        assert_eq!(best_stream_index(&ranked), Some(2));
        // When everything is hidden, automatic playback must respect the profile.
        errored.insert(stream_key(&streams[2]));
        let all_hidden = rank_streams(&streams, &profile, &errored);
        assert_eq!(best_stream_index(&all_hidden), None);
    }

    #[test]
    fn playlist_edl_keeps_every_segment_in_order_with_durations() {
        let items = vec![
            PlaylistItem {
                url: "https://cdn.example/a,b;c%d.ts?token=1&x=2".into(),
                duration_us: Some(1_500_000),
                extra: Default::default(),
            },
            PlaylistItem {
                url: "https://cdn.example/b.ts".into(),
                duration_us: None,
                extra: Default::default(),
            },
        ];
        let edl = playlist_edl(&items).unwrap();
        assert_eq!(
            edl,
            format!(
                "edl://%{}%{},length=1.500000;%{}%{}",
                items[0].url.len(),
                items[0].url,
                items[1].url.len(),
                items[1].url
            )
        );
    }

    #[test]
    fn playlist_rejects_empty_and_non_http_segments() {
        assert!(playlist_edl(&[]).is_err());
        let bad = vec![PlaylistItem { url: "file:///etc/passwd".into(), duration_us: None, extra: Default::default() }];
        assert!(playlist_edl(&bad).unwrap_err().to_string().contains("segment 1"));
    }

    #[test]
    fn playback_target_uses_the_timeline_for_playlists_only() {
        let direct = stream("d", "s", 720);
        assert_eq!(playback_target(&direct).unwrap(), direct.url);
        let playlist = Stream {
            name: "p".into(),
            url: String::new(),
            kind: Some("playlist".into()),
            playlist: Some(vec![PlaylistItem {
                url: "https://cdn.example/1.ts".into(),
                duration_us: Some(2_000_000),
                extra: Default::default(),
            }]),
            ..Default::default()
        };
        assert!(playback_target(&playlist).unwrap().starts_with("edl://"));
        let empty = Stream { kind: Some("playlist".into()), ..Default::default() };
        assert!(playback_target(&empty).is_err());
    }
}
