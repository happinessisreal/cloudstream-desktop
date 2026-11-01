use cloudstream_core::models::*;
use serde_json::json;

#[test]
fn provider_enriched_deserialization_and_unknown_fields_preservation() {
    let payload = json!({
        "id": "sample:provider",
        "pluginId": "sample",
        "name": "Sample Provider",
        "lang": "en",
        "mainUrl": "https://example.com",
        "hasMainPage": true,
        "usesWebView": false,
        "supportedTypes": ["Movie", "TvSeries", "Anime"],
        "capabilities": {
            "hasMainPage": true,
            "hasQuickSearch": true,
            "hasDownloadSupport": true,
            "hasChromecastSupport": false,
            "usesWebView": false,
            "instantLinkLoading": true,
            "sequentialMainPage": false,
            "sequentialMainPageDelayMs": 1500,
            "sequentialMainPageScrollDelayMs": 500,
            "vpnStatus": "MightBeNeeded",
            "providerType": "DirectProvider",
            "supportedSyncNames": ["Trakt", "Mal"],
            "supportedTypes": ["Movie", "TvSeries", "Anime"],
            "futureCapability": "supported"
        },
        "timeouts": {
            "searchTimeoutMs": 15000,
            "quickSearchTimeoutMs": 8000,
            "getMainPageTimeoutMs": 20000,
            "loadTimeoutMs": 25000,
            "loadLinksTimeoutMs": 30000
        },
        "futureTopLevel": 42
    });

    let provider: Provider = serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(provider.id, "sample:provider");
    assert_eq!(provider.name, "Sample Provider");
    assert!(provider.has_main_page);
    assert!(!provider.uses_web_view);

    let caps = provider.capabilities.as_ref().expect("Capabilities must be present");
    assert!(caps.has_main_page);
    assert!(caps.has_quick_search);
    assert!(caps.has_download_support);
    assert!(!caps.has_chromecast_support);
    assert_eq!(caps.vpn_status.as_deref(), Some("MightBeNeeded"));
    assert_eq!(caps.sequential_main_page_delay_ms, Some(1500));
    assert_eq!(
        caps.extra.get("futureCapability"),
        Some(&json!("supported"))
    );

    let timeouts = provider.timeouts.as_ref().expect("Timeouts must be present");
    assert_eq!(timeouts.search_timeout_ms, Some(15000));
    assert_eq!(timeouts.load_links_timeout_ms, Some(30000));

    assert_eq!(provider.extra.get("futureTopLevel"), Some(&json!(42)));

    // Round-trip check: ensure serialized json retains fields
    let serialized = serde_json::to_value(&provider).unwrap();
    assert_eq!(serialized["id"], "sample:provider");
    assert_eq!(
        serialized["capabilities"]["futureCapability"],
        json!("supported")
    );
    assert_eq!(serialized["futureTopLevel"], json!(42));
}

#[test]
fn old_provider_payload_without_capabilities_loads_cleanly() {
    let legacy_payload = json!({
        "id": "old:source",
        "pluginId": "old",
        "name": "Legacy Source",
        "lang": "en",
        "mainUrl": "https://legacy.example",
        "hasMainPage": true,
        "usesWebView": false,
        "supportedTypes": ["Movie"]
    });

    let provider: Provider = serde_json::from_value(legacy_payload).unwrap();
    assert_eq!(provider.id, "old:source");
    assert!(provider.capabilities.is_none());
    assert!(provider.timeouts.is_none());
    assert!(provider.has_main_page);
    assert!(!provider.uses_web_view);
}

#[test]
fn media_item_enriched_deserialization_and_compatibility() {
    let enriched = json!({
        "providerId": "sample:anime",
        "name": "Demon Slayer",
        "url": "https://example.com/ds",
        "posterUrl": "https://example.com/ds.jpg",
        "posterHeaders": { "Referer": "https://example.com" },
        "mediaType": "Anime",
        "year": 2019,
        "uniqueUrl": "https://example.com/ds-unique",
        "mediaId": "sample:anime::hash123",
        "score": 8.7,
        "rating": 8.7,
        "quality": "HD",
        "id": 999,
        "otherName": "Kimetsu no Yaiba",
        "episodesCount": 26,
        "dubStatus": "both",
        "dubEpisodes": {
            "dubbed": 26,
            "subbed": 26
        },
        "customExtensionField": "preserved"
    });

    let item: MediaItem = serde_json::from_value(enriched).unwrap();
    assert_eq!(item.name, "Demon Slayer");
    assert_eq!(item.year, Some(2019));
    assert_eq!(item.score, Some(8.7));
    assert_eq!(item.quality.as_deref(), Some("HD"));
    assert_eq!(item.id, Some(999));
    assert_eq!(item.other_name.as_deref(), Some("Kimetsu no Yaiba"));
    assert_eq!(item.episodes_count, Some(26));
    assert_eq!(
        item.extra.get("customExtensionField"),
        Some(&json!("preserved"))
    );

    let dub_eps = item.dub_episodes.expect("dubEpisodes present");
    assert_eq!(dub_eps.get("dubbed"), Some(&26));
    assert_eq!(dub_eps.get("subbed"), Some(&26));

    // Old minimal payload loads without error
    let legacy = json!({
        "providerId": "sample:p",
        "name": "Film",
        "url": "https://example.com/film"
    });
    let legacy_item: MediaItem = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy_item.name, "Film");
    assert!(legacy_item.score.is_none());
    assert!(legacy_item.year.is_none());
    assert!(legacy_item.dub_episodes.is_none());
}

#[test]
fn media_details_enriched_fields_round_trip() {
    let details_json = json!({
        "item": {
            "providerId": "sample:p",
            "name": "Sample Movie",
            "url": "https://example.com/movie",
            "year": 2024,
            "uniqueUrl": "https://example.com/movie-unique"
        },
        "plot": "An exciting plot overview.",
        "tags": ["Action", "Sci-Fi"],
        "duration": 124,
        "uniqueUrl": "https://example.com/movie-unique",
        "backgroundPosterUrl": "https://example.com/banner.jpg",
        "logoUrl": "https://example.com/logo.png",
        "contentRating": "PG-13",
        "comingSoon": false,
        "syncData": {
            "tmdb": "12345",
            "imdb": "tt9876543"
        },
        "trailers": [
            {
                "extractorUrl": "https://example.com/trailer.mp4",
                "referer": "https://example.com",
                "raw": false,
                "headers": { "User-Agent": "SampleClient" }
            }
        ],
        "actors": [
            {
                "actor": {
                    "name": "Jane Doe",
                    "image": "https://example.com/jane.jpg"
                },
                "role": "Main",
                "roleString": "Protagonist",
                "voiceActor": {
                    "name": "Voice Actor",
                    "image": null
                }
            }
        ],
        "recommendations": [
            {
                "providerId": "sample:p",
                "name": "Recommended Film",
                "url": "https://example.com/rec1",
                "year": 2023
            }
        ],
        "episodes": [
            {
                "name": "Full Movie",
                "data": "https://example.com/movie.mp4",
                "season": null,
                "episode": null,
                "score": 8.9,
                "rating": 8.9,
                "date": 1700000000000i64,
                "runTime": 7440,
                "dubStatus": "none"
            }
        ],
        "dubGroups": {},
        "seasons": [
            {
                "season": null,
                "name": null,
                "episodes": [
                    {
                        "name": "Full Movie",
                        "data": "https://example.com/movie.mp4"
                    }
                ]
            }
        ]
    });

    let details: MediaDetails = serde_json::from_value(details_json.clone()).unwrap();
    assert_eq!(details.plot.as_deref(), Some("An exciting plot overview."));
    assert_eq!(details.duration, Some(124));
    assert_eq!(
        details.unique_url.as_deref(),
        Some("https://example.com/movie-unique")
    );
    assert_eq!(
        details.background_poster_url.as_deref(),
        Some("https://example.com/banner.jpg")
    );
    assert_eq!(
        details.logo_url.as_deref(),
        Some("https://example.com/logo.png")
    );
    assert_eq!(details.content_rating.as_deref(), Some("PG-13"));
    assert_eq!(details.coming_soon, Some(false));
    assert_eq!(details.sync_data.get("tmdb").map(String::as_str), Some("12345"));
    assert_eq!(details.trailers.len(), 1);
    assert_eq!(details.actors.len(), 1);
    assert_eq!(details.actors[0].actor.name, "Jane Doe");
    assert_eq!(details.recommendations.len(), 1);
    assert_eq!(details.episodes.len(), 1);
    assert_eq!(details.episodes[0].run_time, Some(7440));
    assert_eq!(details.episodes[0].score, Some(8.9));

    // Old v1 MediaDetails without rich fields deserializes cleanly
    let legacy_details = json!({
        "item": {
            "providerId": "p",
            "name": "Old Movie",
            "url": "https://example.com/old"
        },
        "plot": "Old plot",
        "tags": ["Classic"],
        "episodes": [
            {
                "name": "Old Ep",
                "data": "data"
            }
        ],
        "duration": 90
    });
    let legacy: MediaDetails = serde_json::from_value(legacy_details).unwrap();
    assert_eq!(legacy.item.name, "Old Movie");
    assert!(legacy.trailers.is_empty());
    assert!(legacy.actors.is_empty());
    assert!(legacy.sync_data.is_empty());
    assert!(legacy.unique_url.is_none());
}

#[test]
fn discriminated_streams_and_subtitles_deserialization() {
    let direct_stream = json!({
        "name": "Direct 1080p",
        "url": "https://example.com/direct.mp4",
        "referer": "https://example.com",
        "quality": 1080,
        "headers": { "X-Custom": "val" },
        "streamType": "VIDEO",
        "kind": "direct",
        "source": "DirectSource",
        "extractorData": "opaque-extractor-data",
        "audioTracks": [
            {
                "url": "https://example.com/audio-en.aac",
                "headers": { "Accept": "*/*" }
            }
        ]
    });
    let s_direct: Stream = serde_json::from_value(direct_stream).unwrap();
    assert_eq!(s_direct.kind.as_deref(), Some("direct"));
    assert_eq!(s_direct.stream_type, "VIDEO");
    assert_eq!(
        s_direct.extractor_data.as_deref(),
        Some("opaque-extractor-data")
    );
    assert_eq!(s_direct.audio_tracks.len(), 1);
    assert_eq!(
        s_direct.audio_tracks[0].url,
        "https://example.com/audio-en.aac"
    );

    let drm_stream = json!({
        "name": "DASH Encrypted",
        "url": "https://example.com/manifest.mpd",
        "streamType": "DRM",
        "kind": "drm",
        "drm": {
            "uuid": "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",
            "kid": "kid-1234",
            "key": "key-5678",
            "licenseUrl": "https://example.com/drm/license",
            "keyRequestParameters": { "p1": "v1" }
        }
    });
    let s_drm: Stream = serde_json::from_value(drm_stream).unwrap();
    assert_eq!(s_drm.kind.as_deref(), Some("drm"));
    let drm = s_drm.drm.expect("DRM payload must exist");
    assert_eq!(
        drm.uuid.as_deref(),
        Some("edef8ba9-79d6-4ace-a3c8-27dcd51d21ed")
    );
    assert_eq!(drm.kid.as_deref(), Some("kid-1234"));
    assert_eq!(
        drm.license_url.as_deref(),
        Some("https://example.com/drm/license")
    );

    let playlist_stream = json!({
        "name": "Segment Playlist",
        "url": "https://example.com/playlist",
        "streamType": "PLAYLIST",
        "kind": "playlist",
        "playlist": [
            { "url": "https://example.com/part1.ts", "durationUs": 5000000 },
            { "url": "https://example.com/part2.ts", "durationUs": 5000000 }
        ]
    });
    let s_playlist: Stream = serde_json::from_value(playlist_stream).unwrap();
    assert_eq!(s_playlist.kind.as_deref(), Some("playlist"));
    let pl = s_playlist.playlist.expect("Playlist must exist");
    assert_eq!(pl.len(), 2);
    assert_eq!(pl[0].duration_us, Some(5000000));

    let subtitle_json = json!({
        "lang": "English",
        "url": "https://example.com/sub.vtt",
        "headers": { "Accept": "text/vtt" },
        "langTag": "en"
    });
    let sub: Subtitle = serde_json::from_value(subtitle_json).unwrap();
    assert_eq!(sub.lang, "English");
    assert_eq!(sub.lang_tag.as_deref(), Some("en"));
    assert_eq!(sub.headers.get("Accept").map(String::as_str), Some("text/vtt"));

    // Legacy stream without new fields parses with default values
    let legacy_stream = json!({
        "name": "Legacy",
        "url": "https://example.com/video.mp4",
        "referer": "",
        "quality": 720,
        "headers": {},
        "streamType": "VIDEO"
    });
    let s_legacy: Stream = serde_json::from_value(legacy_stream).unwrap();
    assert_eq!(s_legacy.name, "Legacy");
    assert_eq!(s_legacy.quality, 720);
    assert!(s_legacy.kind.is_none());
    assert!(s_legacy.audio_tracks.is_empty());
}

#[test]
fn home_response_with_orientation_and_row_identity() {
    let home_json = json!({
        "rows": [
            {
                "name": "Featured",
                "rowId": "home::0::Featured",
                "horizontalImages": true,
                "orientation": "horizontal",
                "hasNext": true,
                "items": [
                    {
                        "providerId": "sample:p",
                        "name": "Item 1",
                        "url": "https://example.com/1"
                    }
                ]
            },
            {
                "name": "Trending",
                "rowId": "home::1::Trending",
                "horizontalImages": false,
                "orientation": "vertical",
                "hasNext": false,
                "items": []
            }
        ],
        "nextSection": 1,
        "hasMoreSections": false
    });

    let home: HomeResponse = serde_json::from_value(home_json).unwrap();
    assert_eq!(home.rows.len(), 2);
    assert_eq!(home.rows[0].name, "Featured");
    assert_eq!(home.rows[0].row_id.as_deref(), Some("home::0::Featured"));
    assert!(home.rows[0].horizontal_images);
    assert_eq!(home.rows[0].orientation.as_deref(), Some("horizontal"));
    assert!(home.rows[0].has_next);
    assert_eq!(home.rows[0].items.len(), 1);

    assert_eq!(home.rows[1].name, "Trending");
    assert!(!home.rows[1].horizontal_images);
    assert_eq!(home.rows[1].orientation.as_deref(), Some("vertical"));
    assert!(!home.rows[1].has_next);
}
