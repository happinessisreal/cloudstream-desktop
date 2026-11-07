//! The behaviour every adapter must share, checked with scripted replies shaped like each
//! service's documented answers. These prove the adapters' reading of those shapes and their
//! handling of refusals; they do not prove the live services still answer this way.
use super::http::fake::Fake;
use super::*;
use serde_json::json;

fn query() -> SubtitleQuery {
    SubtitleQuery { title: "The Show".into(), year: Some(2011), season: Some(2), episode: Some(5), imdb_id: Some("tt0944947".into()), tmdb_id: None, languages: vec!["en".into(), "fr".into()], hearing_impaired: HearingImpaired::Include }
}

fn movie() -> SubtitleQuery {
    SubtitleQuery { title: "The Film".into(), year: Some(1999), imdb_id: Some("tt0133093".into()), languages: vec!["en".into()], ..Default::default() }
}

fn creds(pairs: &[(&str, &str)]) -> Credentials {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

const SRT: &[u8] = b"1\n00:00:01,000 --> 00:00:02,000\nHello\n";
const ZIP: &[u8] = b"PK\x03\x04\x14\x00rest-of-archive";

fn json_reply(status: u16, body: serde_json::Value) -> HttpResponse {
    HttpResponse::new(status, body.to_string().into_bytes()).with_header("content-type", "application/json")
}

fn problem_of<T: std::fmt::Debug>(result: Result<T, ProviderProblem>) -> ProviderProblem {
    result.expect_err("expected a problem")
}

// ---------------------------------------------------------------------------------------------
// OpenSubtitles
// ---------------------------------------------------------------------------------------------

mod opensubtitles {
    use super::*;
    use crate::subtitle_providers::opensubtitles::OpenSubtitles;

    fn item(file_id: u64, language: &str, episode: i64, hi: bool, release: &str) -> serde_json::Value {
        json!({"id": file_id.to_string(), "type": "subtitle", "attributes": {
            "subtitle_id": file_id.to_string(), "language": language, "download_count": 100 + file_id, "hearing_impaired": hi,
            "ai_translated": false, "machine_translated": false, "foreign_parts_only": false, "release": release,
            "uploader": {"uploader_id": 5, "name": "alice"},
            "feature_details": {"feature_id": 9, "feature_type": "Episode", "year": 2011, "title": format!("Episode {episode}"), "season_number": 2, "episode_number": episode, "parent_title": "The Show", "imdb_id": 123},
            "files": [{"file_id": file_id, "cd_number": 1, "file_name": "x.srt"}]}})
    }

    fn keys() -> Credentials {
        creds(&[("apiKey", "KEY-123"), ("username", "ann"), ("password", "hunter2"), ("token", "TOKEN-A")])
    }

    #[tokio::test]
    async fn search_sends_sorted_lowercase_parameters_and_keeps_only_the_requested_episode() {
        let http = Fake::default();
        http.reply("/subtitles?", json_reply(200, json!({"total_pages": 1, "data": [item(1001, "en", 5, false, "The.Show.S02E05.720p"), item(1002, "en", 6, false, "wrong episode"), item(1003, "fr", 5, true, "The.Show.S02E05.FR")]})));
        let mut c = keys();
        let found = OpenSubtitles.search(&http, &mut c, &query()).await.unwrap();
        assert_eq!(found.iter().map(|x| (x.id.as_str(), x.language.as_str(), x.hearing_impaired)).collect::<Vec<_>>(), [("1001", "en", false), ("1003", "fr", true)], "episode 6 is dropped, English ranks first");
        assert_eq!(found[0].matched_title.as_deref(), Some("The Show"), "the series title is shown, so a wrong series is visible");
        assert_eq!((found[0].season, found[0].episode, found[0].uploader.as_deref()), (Some(2), Some(5), Some("alice")));
        let sent = http.requests();
        let url = url::Url::parse(&sent[0].url).unwrap();
        let names: Vec<String> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "the service requires alphabetical order");
        assert!(url.query_pairs().any(|(k, v)| k == "parent_imdb_id" && v == "944947"), "an episode is searched by its series ID: {url}");
        assert!(url.query_pairs().any(|(k, v)| k == "languages" && v == "en,fr") && url.query_pairs().any(|(k, v)| k == "type" && v == "episode"));
        assert!(!url.query_pairs().any(|(k, _)| k == "query"), "known IDs are used instead of the title text");
        assert!(sent[0].headers.contains(&("api-key".into(), "KEY-123".into())));
        assert!(!sent[0].url.contains("KEY-123"), "the key is a header, not part of the address");
    }

    #[tokio::test]
    async fn movies_without_ids_are_searched_by_title_and_year() {
        let http = Fake::default();
        http.reply("/subtitles?", json_reply(200, json!({"data": []})));
        let mut q = movie();
        q.imdb_id = None;
        let found = OpenSubtitles.search(&http, &mut keys(), &q).await.unwrap();
        assert!(found.is_empty(), "no results is an ordinary empty answer");
        let url = url::Url::parse(&http.requests()[0].url).unwrap();
        assert!(url.query_pairs().any(|(k, v)| k == "query" && v == "the film") && url.query_pairs().any(|(k, v)| k == "year" && v == "1999") && url.query_pairs().any(|(k, v)| k == "type" && v == "movie"));
    }

    #[tokio::test]
    async fn refusals_stay_distinct_from_empty_results() {
        let cases: Vec<(HttpResponse, ProviderProblem)> = vec![
            (json_reply(401, json!({})), ProviderProblem::SignInRejected),
            (json_reply(403, json!({})), ProviderProblem::SignInRejected),
            (json_reply(429, json!({})).with_header("retry-after", "12"), ProviderProblem::RateLimited { retry_after_secs: Some(12) }),
            (json_reply(503, json!({})), ProviderProblem::Unavailable),
            (HttpResponse::new(200, b"<html>maintenance</html>".to_vec()), ProviderProblem::BadResponse),
            (json_reply(200, json!({"unexpected": true})), ProviderProblem::BadResponse),
        ];
        for (reply, expected) in cases {
            let http = Fake::default();
            http.reply("/subtitles?", reply);
            assert_eq!(problem_of(OpenSubtitles.search(&http, &mut keys(), &query()).await), expected);
        }
        let http = Fake::default();
        http.on("/subtitles?", |_| Err(ProviderProblem::Unavailable));
        assert_eq!(problem_of(OpenSubtitles.search(&http, &mut keys(), &query()).await), ProviderProblem::Unavailable);
        assert_eq!(problem_of(OpenSubtitles.search(&Fake::default(), &mut Credentials::new(), &query()).await), ProviderProblem::NotConnected);
    }

    #[tokio::test]
    async fn connecting_signs_in_and_keeps_the_token_and_the_host_the_service_names() {
        let http = Fake::default();
        http.reply("/login", json_reply(200, json!({"user": {"allowed_downloads": 20}, "token": "JWT-1", "status": 200, "base_url": "vip-api.opensubtitles.com"})));
        let saved = OpenSubtitles.connect(&http, &creds(&[("apiKey", " KEY ")  , ("username", "ann"), ("password", "pw"), ("ignored", "x")])).await.unwrap();
        assert_eq!(saved, creds(&[("apiKey", "KEY"), ("username", "ann"), ("password", "pw"), ("token", "JWT-1"), ("baseUrl", "vip-api.opensubtitles.com")]));
        let body: serde_json::Value = serde_json::from_slice(http.requests()[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body, json!({"username": "ann", "password": "pw"}));

        // A host outside the service's domain is not followed.
        let http = Fake::default();
        http.reply("/login", json_reply(200, json!({"token": "JWT-2", "base_url": "evil.example"})));
        let saved = OpenSubtitles.connect(&http, &creds(&[("apiKey", "K"), ("username", "ann"), ("password", "pw")])).await.unwrap();
        assert!(!saved.contains_key("baseUrl"));

        for (status, expected) in [(400, ProviderProblem::SignInRejected), (401, ProviderProblem::SignInRejected), (403, ProviderProblem::SignInRejected), (429, ProviderProblem::RateLimited { retry_after_secs: None })] {
            let http = Fake::default();
            http.reply("/login", json_reply(status, json!({"message": "nope"})));
            assert_eq!(problem_of(OpenSubtitles.connect(&http, &creds(&[("apiKey", "K"), ("username", "ann"), ("password", "bad")])).await), expected);
        }
        assert_eq!(problem_of(OpenSubtitles.connect(&Fake::default(), &creds(&[("apiKey", "K")])).await), ProviderProblem::NotConnected, "all three details are required");
    }

    fn picked() -> Candidate {
        Candidate { provider: "opensubtitles".into(), id: "1001".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn download_uses_the_token_and_sends_no_key_to_the_file_address() {
        let http = Fake::default();
        http.reply("/download", json_reply(200, json!({"link": "https://dl.opensubtitles.com/en/download/x.srt", "file_name": "x.srt", "requests": 3, "remaining": 17})));
        http.reply("dl.opensubtitles.com", HttpResponse::new(200, SRT.to_vec()));
        let mut c = keys();
        let got = OpenSubtitles.download(&http, &mut c, &picked()).await.unwrap();
        assert_eq!((got.bytes.as_slice(), got.remaining, got.file_name.as_str()), (SRT, Some(17), "x.srt"));
        let sent = http.requests();
        assert!(sent[0].headers.contains(&("authorization".into(), "Bearer TOKEN-A".into())));
        let body: serde_json::Value = serde_json::from_slice(sent[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["file_id"], 1001);
        assert!(!sent[1].headers.iter().any(|(n, _)| n == "api-key" || n == "authorization"), "the file host never receives the key or token");
    }

    #[tokio::test]
    async fn an_expired_token_is_refreshed_once_and_the_new_one_is_returned_for_saving() {
        let http = Fake::default();
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = attempts.clone();
        http.on("/download", move |request| {
            let first = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            let fresh = request.headers.iter().any(|(n, v)| n == "authorization" && v == "Bearer TOKEN-B");
            Ok(if first && !fresh { json_reply(401, json!({"message": "expired"})) } else { json_reply(200, json!({"link": "https://dl.opensubtitles.com/a.srt", "file_name": "a.srt"})) })
        });
        http.reply("/login", json_reply(200, json!({"token": "TOKEN-B"})));
        http.reply("dl.opensubtitles.com", HttpResponse::new(200, SRT.to_vec()));
        let mut c = keys();
        OpenSubtitles.download(&http, &mut c, &picked()).await.unwrap();
        assert_eq!(c["token"], "TOKEN-B");
        assert_eq!(http.count("/login"), 1);

        // If the saved password no longer works, that is reported instead of looping.
        let http = Fake::default();
        http.reply("/download", json_reply(401, json!({})));
        http.reply("/login", json_reply(401, json!({})));
        assert_eq!(problem_of(OpenSubtitles.download(&http, &mut keys(), &picked()).await), ProviderProblem::SignInRejected);
        assert_eq!(http.count("/login"), 1);
    }

    #[tokio::test]
    async fn download_limits_and_odd_replies_are_reported_as_what_they_are() {
        let cases: Vec<(HttpResponse, ProviderProblem)> = vec![
            (json_reply(406, json!({"message": "You have downloaded your allowed 20 subtitles for today", "remaining": 0})), ProviderProblem::QuotaExhausted { retry_after_secs: None }),
            (json_reply(429, json!({})).with_header("retry-after", "30"), ProviderProblem::RateLimited { retry_after_secs: Some(30) }),
            (json_reply(200, json!({"link": "http://dl.opensubtitles.com/insecure.srt"})), ProviderProblem::BadResponse),
            (json_reply(200, json!({"link": "https://evil.example/x.srt"})), ProviderProblem::BadResponse),
            (json_reply(200, json!({"nothing": 1})), ProviderProblem::BadResponse),
        ];
        for (reply, expected) in cases {
            let http = Fake::default();
            http.reply("/download", reply);
            assert_eq!(problem_of(OpenSubtitles.download(&http, &mut keys(), &picked()).await), expected);
        }
        let http = Fake::default();
        http.reply("/download", json_reply(200, json!({"link": "https://dl.opensubtitles.com/a.srt"})));
        http.reply("dl.opensubtitles.com", HttpResponse::new(200, b"<html>please log in</html>".to_vec()));
        assert_eq!(problem_of(OpenSubtitles.download(&http, &mut keys(), &picked()).await), ProviderProblem::BadResponse, "an error page is never saved as a subtitle");
        let mut no_session = creds(&[("apiKey", "K")]);
        assert_eq!(problem_of(OpenSubtitles.download(&Fake::default(), &mut no_session, &picked()).await), ProviderProblem::NotConnected);
    }
}

// ---------------------------------------------------------------------------------------------
// Subdl
// ---------------------------------------------------------------------------------------------

mod subdl {
    use super::*;
    use crate::subtitle_providers::subdl::Subdl;

    fn sub(url: &str, language: &str, season: i64, episode: Option<i64>, hi: bool, full: bool, release: &str) -> serde_json::Value {
        json!({"release_name": release, "name": format!("SUBDL.com::{release}"), "url": url, "language": language, "releases": [release], "author": "bob", "hi": hi,
            "season": season, "episode": episode, "full_season": full, "subtitlePage": "/subtitle/sd1/the-show"})
    }

    fn found() -> serde_json::Value {
        json!({"status": true, "results": [{"sd_id": 1, "type": "tv", "name": "The Show", "imdb_id": "tt0944947", "year": 2011}], "subtitles": [
            sub("/subtitle/1-1.zip", "EN", 2, Some(5), false, false, "The.Show.S02E05.720p"),
            sub("/subtitle/1-2.zip", "EN", 2, Some(6), false, false, "The.Show.S02E06.720p"),
            sub("/subtitle/1-3.zip", "EN", 2, None, false, true, "The.Show.S02.COMPLETE"),
            sub("/subtitle/1-4.zip", "FR", 2, Some(5), true, false, "The.Show.S02E05.FR.SDH"),
            sub("/subtitle/1-5.zip", "BR_PT", 2, Some(5), false, false, "The.Show.S02E05.PT"),
        ]})
    }

    fn keys() -> Credentials {
        creds(&[("apiKey", "SUBDL-KEY")])
    }

    #[tokio::test]
    async fn search_reads_labels_marks_packs_and_drops_other_episodes() {
        let http = Fake::default();
        http.reply("/api/v1/subtitles", json_reply(200, found()));
        let got = Subdl.search(&http, &mut keys(), &query()).await.unwrap();
        let summary: Vec<_> = got.iter().map(|c| (c.id.as_str(), c.language.as_str(), c.pack, c.hearing_impaired)).collect();
        assert_eq!(summary, [("/subtitle/1-1.zip", "en", false, false), ("/subtitle/1-3.zip", "en", true, false), ("/subtitle/1-4.zip", "fr", false, true), ("/subtitle/1-5.zip", "pt-br", false, false)], "episode 6 is dropped; the pack follows the exact episode");
        assert_eq!(got[0].matched_title.as_deref(), Some("The Show"));
        assert_eq!(got[0].uploader.as_deref(), Some("bob"));
        let url = url::Url::parse(&http.requests()[0].url).unwrap();
        let get = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
        assert_eq!((get("type").as_deref(), get("imdb_id").as_deref(), get("season_number").as_deref(), get("episode_number").as_deref(), get("languages").as_deref()), (Some("tv"), Some("tt0944947"), Some("2"), Some("5"), Some("EN,FR")));
        assert_eq!(get("film_name"), None, "a known ID is used instead of the title");
    }

    #[tokio::test]
    async fn the_service_saying_nothing_matches_is_an_empty_answer_not_an_error() {
        for reply in [json_reply(200, json!({"status": false, "error": "Sorry, we can't find that film"})), json_reply(404, json!({"error": "not found"})), json_reply(200, json!({"status": true, "results": [], "subtitles": []}))] {
            let http = Fake::default();
            http.reply("/api/v1/subtitles", reply);
            assert_eq!(Subdl.search(&http, &mut keys(), &query()).await.unwrap(), Vec::<Candidate>::new());
        }
    }

    #[tokio::test]
    async fn plan_limits_quotas_and_bad_keys_are_distinct() {
        let cases: Vec<(HttpResponse, ProviderProblem)> = vec![
            (json_reply(402, json!({"error": "paid_api_required"})), ProviderProblem::PaidPlanRequired),
            (json_reply(403, json!({"error": "Invalid API key"})), ProviderProblem::SignInRejected),
            (json_reply(403, json!({})), ProviderProblem::Blocked),
            (json_reply(401, json!({})), ProviderProblem::SignInRejected),
            (json_reply(429, json!({"error": "rate_limit"})).with_header("retry-after", "12"), ProviderProblem::RateLimited { retry_after_secs: Some(12) }),
            (json_reply(429, json!({"error": "daily_limit"})), ProviderProblem::QuotaExhausted { retry_after_secs: None }),
            (json_reply(429, json!({"error": "api_download_limit_exceeded"})), ProviderProblem::QuotaExhausted { retry_after_secs: None }),
            (json_reply(429, json!({"error": "service_busy"})), ProviderProblem::Unavailable),
            (json_reply(500, json!({})), ProviderProblem::Unavailable),
            (json_reply(200, json!({"status": false, "error": "Invalid api_key"})), ProviderProblem::SignInRejected),
            (HttpResponse::new(200, b"<html>".to_vec()), ProviderProblem::BadResponse),
            (json_reply(200, json!({"status": true})), ProviderProblem::BadResponse),
        ];
        for (reply, expected) in cases {
            let http = Fake::default();
            http.reply("/api/v1/subtitles", reply);
            assert_eq!(problem_of(Subdl.search(&http, &mut keys(), &query()).await), expected);
        }
        assert_eq!(problem_of(Subdl.search(&Fake::default(), &mut Credentials::new(), &query()).await), ProviderProblem::NotConnected);
    }

    #[tokio::test]
    async fn connecting_proves_the_key_with_one_search_and_keeps_only_the_key() {
        let http = Fake::default();
        http.reply("/api/v1/subtitles", json_reply(200, found()));
        assert_eq!(Subdl.connect(&http, &creds(&[("apiKey", "  K1  "), ("extra", "x")])).await.unwrap(), creds(&[("apiKey", "K1")]));
        assert_eq!(http.requests().len(), 1);
        for (reply, expected) in [(json_reply(403, json!({"error": "invalid key"})), ProviderProblem::SignInRejected), (json_reply(402, json!({"error": "paid_api_required"})), ProviderProblem::PaidPlanRequired)] {
            let http = Fake::default();
            http.reply("/api/v1/subtitles", reply);
            assert_eq!(problem_of(Subdl.connect(&http, &creds(&[("apiKey", "K")])).await), expected, "a key that cannot be used is not shown as connected");
        }
        assert_eq!(problem_of(Subdl.connect(&Fake::default(), &Credentials::new()).await), ProviderProblem::NotConnected);
    }

    #[tokio::test]
    async fn download_keeps_the_key_in_a_header_and_refuses_paths_outside_the_service() {
        let http = Fake::default();
        http.reply("dl.subdl.com", HttpResponse::new(200, ZIP.to_vec()));
        let candidate = Candidate { provider: "subdl".into(), id: "/subtitle/1-1.zip".into(), ..Default::default() };
        let got = Subdl.download(&http, &mut keys(), &candidate).await.unwrap();
        assert_eq!(got.bytes, ZIP);
        let sent = &http.requests()[0];
        assert_eq!(sent.url, "https://dl.subdl.com/subtitle/1-1.zip");
        assert!(sent.headers.contains(&("x-api-key".into(), "SUBDL-KEY".into())) && !sent.url.contains("SUBDL-KEY"));
        for bad in ["/other/1.zip", "/subtitle/../x", "https://evil.example/subtitle/1.zip", ""] {
            let c = Candidate { id: bad.into(), ..candidate.clone() };
            assert_eq!(problem_of(Subdl.download(&Fake::default(), &mut keys(), &c).await), ProviderProblem::BadResponse, "{bad}");
        }
        for (reply, expected) in [
            (json_reply(402, json!({"error": "paid_api_required"})), ProviderProblem::PaidPlanRequired),
            (json_reply(429, json!({"error": "daily_limit"})), ProviderProblem::QuotaExhausted { retry_after_secs: None }),
            (json_reply(404, json!({})), ProviderProblem::NotFound),
            (HttpResponse::new(200, b"<html>Not allowed</html>".to_vec()), ProviderProblem::BadResponse),
        ] {
            let http = Fake::default();
            http.reply("dl.subdl.com", reply);
            assert_eq!(problem_of(Subdl.download(&http, &mut keys(), &candidate).await), expected);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// SubSource
// ---------------------------------------------------------------------------------------------

mod subsource {
    use super::*;
    use crate::subtitle_providers::subsource::SubSource;

    fn keys() -> Credentials {
        creds(&[("apiKey", "SS-KEY")])
    }

    fn titles() -> serde_json::Value {
        json!({"data": [{"movieId": 78, "title": "The Show Returns", "alternateTitle": "", "releaseYear": 2011}, {"movieId": 77, "title": "The Show", "alternateTitle": "", "releaseYear": 2011}]})
    }

    fn subs() -> serde_json::Value {
        json!({"success": true, "data": [
            {"subtitleId": 501, "link": "/subtitle/x/english/501", "language": "English", "releaseInfo": "The.Show.S02E05.720p", "contributors": [{"id": 3, "displayname": "carol"}], "uploaderId": 3, "foreignParts": false, "commentary": "", "hearingImpaired": false},
            {"subtitleId": 502, "link": "/subtitle/x/english/502", "language": "English", "releaseInfo": "The.Show.S02E06.720p", "contributors": [], "uploaderId": 9, "foreignParts": false, "commentary": "", "hearingImpaired": false},
            {"subtitleId": 503, "link": "/subtitle/x/english/503", "language": "English", "releaseInfo": "The.Show.S02.COMPLETE", "contributors": [], "uploaderId": 9, "foreignParts": false, "commentary": "SDH", "hearingImpaired": false},
            {"subtitleId": 504, "link": "/subtitle/x/english/504", "language": "English", "releaseInfo": "The.Show.S02E05.720p", "contributors": [], "uploaderId": 9, "foreignParts": true, "commentary": "non-SDH forced", "hearingImpaired": false},
        ]})
    }

    #[tokio::test]
    async fn the_title_is_found_exactly_then_its_subtitles_are_listed_per_language() {
        let http = Fake::default();
        http.reply("movies/search", json_reply(200, titles()));
        http.reply("/subtitles?", json_reply(200, subs()));
        let mut q = query();
        q.imdb_id = None;
        q.languages = vec!["en".into()];
        let got = SubSource.search(&http, &mut keys(), &q).await.unwrap();
        assert_eq!(got.iter().map(|c| (c.id.as_str(), c.pack, c.hearing_impaired, c.forced)).collect::<Vec<_>>(), [("501", false, false, false), ("504", false, false, true), ("503", true, true, false)], "episode 6 dropped; the commentary decides SDH and forced");
        assert_eq!(got[0].matched_title.as_deref(), Some("The Show"), "the exact title, not 'The Show Returns'");
        assert_eq!(got[0].uploader.as_deref(), Some("carol"));
        let listing = url::Url::parse(&http.requests().iter().find(|r| r.url.contains("/subtitles?")).unwrap().url).unwrap();
        let get = |k: &str| listing.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
        assert_eq!((get("movieId").as_deref(), get("language").as_deref(), get("seasonNumber").as_deref(), get("episodeNumber").as_deref()), (Some("77"), Some("english"), Some("2"), Some("5")));
        assert!(http.requests().iter().all(|r| r.headers.contains(&("x-api-key".into(), "SS-KEY".into())) && !r.url.contains("SS-KEY")));
    }

    #[tokio::test]
    async fn a_similar_title_or_a_different_year_is_never_substituted() {
        let http = Fake::default();
        http.reply("movies/search", json_reply(200, titles()));
        http.reply("/subtitles?", json_reply(200, subs()));
        let mut wrong_title = movie();
        wrong_title.imdb_id = None;
        wrong_title.title = "The Sho".into();
        assert!(SubSource.search(&http, &mut keys(), &wrong_title).await.unwrap().is_empty());
        let mut wrong_year = movie();
        wrong_year.imdb_id = None;
        wrong_year.title = "The Show".into();
        wrong_year.year = Some(1980);
        assert!(SubSource.search(&http, &mut keys(), &wrong_year).await.unwrap().is_empty());
        assert_eq!(http.count("/subtitles?"), 0, "no subtitles are listed for a title that was not identified");
    }

    #[tokio::test]
    async fn an_imdb_id_is_tried_first_and_unknown_languages_are_not_guessed() {
        let http = Fake::default();
        http.reply("searchType=imdb", json_reply(200, json!({"data": [{"movieId": 99, "title": "Totally Different Name", "releaseYear": 2011}]})));
        http.reply("/subtitles?", json_reply(200, json!({"success": true, "data": []})));
        assert!(SubSource.search(&http, &mut keys(), &query()).await.unwrap().is_empty());
        assert!(http.requests().iter().any(|r| r.url.contains("movieId=99")), "the ID match is trusted over the name");
        let mut tlh = query();
        tlh.languages = vec!["tlh".into()];
        assert_eq!(problem_of(SubSource.search(&Fake::default(), &mut keys(), &tlh).await), ProviderProblem::NotSupported);
    }

    #[tokio::test]
    async fn refusals_are_distinct_and_connect_checks_the_key() {
        let cases: Vec<(HttpResponse, ProviderProblem)> = vec![
            (json_reply(401, json!({})), ProviderProblem::SignInRejected),
            (json_reply(403, json!({})), ProviderProblem::SignInRejected),
            (json_reply(429, json!({"retryAfter": 20})), ProviderProblem::RateLimited { retry_after_secs: Some(20) }),
            (json_reply(500, json!({})), ProviderProblem::Unavailable),
            (HttpResponse::new(200, b"nope".to_vec()), ProviderProblem::BadResponse),
        ];
        for (reply, expected) in cases {
            let http = Fake::default();
            http.reply("movies/search", reply.clone());
            assert_eq!(problem_of(SubSource.search(&http, &mut keys(), &query()).await), expected);
            assert_eq!(problem_of(SubSource.connect(&http, &keys()).await), expected);
        }
        let http = Fake::default();
        http.reply("movies/search", json_reply(200, titles()));
        assert_eq!(SubSource.connect(&http, &creds(&[("apiKey", " K ")])).await.unwrap(), creds(&[("apiKey", "K")]));
        assert_eq!(problem_of(SubSource.search(&Fake::default(), &mut Credentials::new(), &query()).await), ProviderProblem::NotConnected);
    }

    #[tokio::test]
    async fn download_returns_the_archive_and_refuses_odd_ids_and_pages() {
        let http = Fake::default();
        http.reply("/subtitles/501/download", HttpResponse::new(200, ZIP.to_vec()));
        let c = Candidate { provider: "subsource".into(), id: "501".into(), ..Default::default() };
        assert_eq!(SubSource.download(&http, &mut keys(), &c).await.unwrap().bytes, ZIP);
        for bad in ["", "5/../1", "abc"] {
            assert_eq!(problem_of(SubSource.download(&Fake::default(), &mut keys(), &Candidate { id: bad.into(), ..c.clone() }).await), ProviderProblem::BadResponse);
        }
        let http = Fake::default();
        http.reply("/subtitles/501/download", HttpResponse::new(200, b"<html>login</html>".to_vec()));
        assert_eq!(problem_of(SubSource.download(&http, &mut keys(), &c).await), ProviderProblem::BadResponse);
        let http = Fake::default();
        http.reply("/subtitles/501/download", json_reply(404, json!({})));
        assert_eq!(problem_of(SubSource.download(&http, &mut keys(), &c).await), ProviderProblem::NotFound);
    }
}

// ---------------------------------------------------------------------------------------------
// Addic7ed
// ---------------------------------------------------------------------------------------------

mod addic7ed {
    use super::*;
    use crate::subtitle_providers::addic7ed::Addic7ed;

    const SHOWS: &str = r#"<table><tr><td class="version"><h3><a href="/show/1111">The Show Returns</a></h3></td></tr>
        <tr><td><h3><a href="/show/1234">The Show (2019)</a></h3></td></tr><tr><td><h3><a href="/show/1235">The Show</a></h3></td></tr></table>"#;

    fn row(season: u32, episode: u32, language: &str, version: &str, status: &str, hi: &str, link: &str) -> String {
        format!(r#"<tr class="epeven completed"><td>{season}</td><td>{episode}</td><td><a href="/serie/The_Show/{season}/{episode}/Title">Title &amp; More</a></td><td class="language">{language}</td><td>{version}</td><td class="newsDate">{status}</td><td>{hi}</td><td></td><td></td><td><a class="buttonDownload" href="{link}">Download</a></td></tr>"#)
    }

    fn episodes() -> String {
        format!(
            "<table>{}{}{}{}{}</table>",
            row(2, 5, "English", "WEB-DL 720p", "Completed", "", "/original/100/0"),
            row(2, 5, "French", "WEB-DL 720p", "Completed", "✔", "/original/101/0"),
            row(2, 5, "English", "HDTV", "45% Completed", "", "/original/102/0"),
            row(2, 6, "English", "WEB-DL 720p", "Completed", "", "/original/103/0"),
            row(2, 5, "Klingon", "WEB-DL", "Completed", "", "//evil.example/x"),
        )
    }

    fn site() -> Fake {
        let http = Fake::default();
        http.reply("shows.php", HttpResponse::new(200, SHOWS.as_bytes().to_vec()).with_header("content-type", "text/html"));
        http.reply("ajax_loadShow.php", HttpResponse::new(200, episodes().into_bytes()).with_header("content-type", "text/html"));
        http
    }

    #[tokio::test]
    async fn the_exact_show_is_found_and_rows_become_labelled_candidates() {
        let http = site();
        let mut q = query();
        q.year = None;
        let got = Addic7ed.search(&http, &mut Credentials::new(), &q).await.unwrap();
        assert_eq!(got.iter().map(|c| (c.id.as_str(), c.language.as_str(), c.hearing_impaired, c.release.as_str())).collect::<Vec<_>>(), [("/original/100/0", "en", false, "WEB-DL 720p"), ("/original/101/0", "fr", true, "WEB-DL 720p")], "unfinished, other-episode and off-site rows are not offered");
        assert_eq!(got[0].matched_title.as_deref(), Some("The Show"), "the plain name wins when no year is given");
        let list = http.requests().into_iter().find(|r| r.url.contains("ajax_loadShow.php")).unwrap();
        assert!(list.url.contains("show=1235") && list.url.contains("season=2"));
        assert!(list.headers.contains(&("x-requested-with".into(), "XMLHttpRequest".into())) && list.headers.iter().any(|(n, v)| n == "referer" && v.ends_with("/show/1235")));
        let mut with_year = query();
        with_year.year = Some(2019);
        let got = Addic7ed.search(&site(), &mut Credentials::new(), &with_year).await.unwrap();
        assert_eq!(got[0].matched_title.as_deref(), Some("The Show (2019)"), "the year picks between shows of the same name");
    }

    #[tokio::test]
    async fn another_show_or_a_movie_or_a_missing_season_gives_no_substitute() {
        let mut other = query();
        other.title = "The Sho".into();
        assert!(Addic7ed.search(&site(), &mut Credentials::new(), &other).await.unwrap().is_empty());
        assert_eq!(problem_of(Addic7ed.search(&site(), &mut Credentials::new(), &movie()).await), ProviderProblem::NotSupported);
        let mut no_season = query();
        no_season.season = None;
        let http = site();
        assert!(Addic7ed.search(&http, &mut Credentials::new(), &no_season).await.unwrap().is_empty());
        assert_eq!(http.count("ajax_loadShow.php"), 0);
    }

    #[tokio::test]
    async fn bot_checks_bans_and_server_trouble_are_reported_not_hidden() {
        let cases: Vec<(HttpResponse, ProviderProblem)> = vec![
            (HttpResponse::new(503, b"<html><title>Just a moment...</title></html>".to_vec()), ProviderProblem::Blocked),
            (HttpResponse::new(403, b"forbidden".to_vec()), ProviderProblem::Blocked),
            (HttpResponse::new(200, b"<html>cf-chl-bypass</html>".to_vec()), ProviderProblem::Blocked),
            (HttpResponse::new(429, b"".to_vec()).with_header("retry-after", "60"), ProviderProblem::RateLimited { retry_after_secs: Some(60) }),
            (HttpResponse::new(500, b"oops".to_vec()), ProviderProblem::Unavailable),
        ];
        for (reply, expected) in cases {
            let http = Fake::default();
            http.reply("shows.php", reply);
            assert_eq!(problem_of(Addic7ed.search(&http, &mut Credentials::new(), &query()).await), expected);
        }
        let http = site();
        http.reply("ajax_loadShow.php", HttpResponse::new(200, b"<html>Relax, slow down!</html>".to_vec()));
        assert_eq!(problem_of(Addic7ed.search(&http, &mut Credentials::new(), &query()).await), ProviderProblem::RateLimited { retry_after_secs: None });
        assert!(!Addic7ed.needs_connection(), "no details are needed to search");
    }

    #[tokio::test]
    async fn download_sends_the_page_as_referer_and_reads_a_spent_allowance_from_an_html_reply() {
        let http = site();
        let got = Addic7ed.search(&http, &mut Credentials::new(), &query()).await.unwrap();
        let first = got[0].clone();
        let http = Fake::default();
        http.reply("/original/100/0", HttpResponse::new(200, SRT.to_vec()).with_header("content-type", "application/x-subrip"));
        let file = Addic7ed.download(&http, &mut Credentials::new(), &first).await.unwrap();
        assert_eq!(file.bytes, SRT);
        let sent = &http.requests()[0];
        assert_eq!(sent.url, "https://www.addic7ed.com/original/100/0");
        assert!(sent.headers.iter().any(|(n, v)| n == "referer" && v == "https://www.addic7ed.com/serie/The_Show/2/5/Title"));

        let http = Fake::default();
        http.reply("/original/100/0", HttpResponse::new(200, b"<html>You have reached your daily download limit</html>".to_vec()).with_header("content-type", "text/html; charset=utf-8"));
        assert_eq!(problem_of(Addic7ed.download(&http, &mut Credentials::new(), &first).await), ProviderProblem::QuotaExhausted { retry_after_secs: None });
        let http = Fake::default();
        http.reply("/original/100/0", HttpResponse::new(304, b"".to_vec()));
        assert_eq!(problem_of(Addic7ed.download(&http, &mut Credentials::new(), &first).await), ProviderProblem::RateLimited { retry_after_secs: None });
        for bad in ["//evil.example/x", "original/1", "/a/../b", ""] {
            assert_eq!(problem_of(Addic7ed.download(&Fake::default(), &mut Credentials::new(), &Candidate { id: bad.into(), ..first.clone() }).await), ProviderProblem::BadResponse, "{bad}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Across all services
// ---------------------------------------------------------------------------------------------

#[test]
fn every_service_describes_what_it_needs_and_what_it_covers() {
    let providers = all();
    assert_eq!(providers.iter().map(|p| p.info().id).collect::<Vec<_>>(), PROVIDER_IDS);
    for provider in &providers {
        let info = provider.info();
        assert!(info.site.starts_with("https://") && !info.description.is_empty() && info.movies || info.episodes, "{}", info.id);
        assert_eq!(provider.needs_connection(), info.fields.iter().any(|f| f.required));
        for field in &info.fields {
            assert!(!field.label.is_empty() && !field.name.is_empty());
        }
    }
    assert!(by_id("subdl").is_some() && by_id("nope").is_none());
    let secret_fields: Vec<_> = providers.iter().flat_map(|p| p.info().fields).filter(|f| f.secret).map(|f| f.name).collect();
    assert!(secret_fields.contains(&"apiKey") && secret_fields.contains(&"password"), "keys and passwords are marked secret for the form");
}
