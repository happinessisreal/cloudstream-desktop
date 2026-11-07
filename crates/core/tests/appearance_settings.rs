//! Appearance preferences (T30): typed, validated, persistent, and compatible with old data.
use cloudstream_core::{models::*, App};
use serde_json::json;

#[test]
fn defaults_keep_the_approved_design_and_old_settings_still_load() {
    let defaults = AppearanceSettings::default();
    assert_eq!((defaults.theme, defaults.accent, defaults.poster_scale, defaults.title_placement), (Theme::Dark, Accent::Indigo, 100, TitlePlacement::Below));
    assert!(defaults.show_badges && defaults.motion == MotionPreference::System && defaults.navigation == NavigationMode::Pointer);
    // Settings saved before these options existed load with the defaults filled in.
    let old: AppearanceSettings = serde_json::from_value(json!({ "posterDensity": "compact", "searchSuggestions": false, "futureOption": 7 })).unwrap();
    assert_eq!(old.poster_density, PosterDensity::Compact);
    assert!(!old.search_suggestions);
    assert_eq!((old.theme, old.accent, old.poster_scale, old.show_badges), (Theme::Dark, Accent::Indigo, 100, true));
    assert_eq!(old.extra["futureOption"], 7, "unknown fields survive");
}

#[test]
fn choices_persist_across_a_restart_and_bad_values_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    {
        let app = App::open(dir.path().join("data"), dir.path().join("res")).unwrap();
        let saved = app.store.update_settings(json!({ "appearance": { "theme": "black", "accent": "violet", "posterScale": 125, "titlePlacement": "overlay", "showBadges": false, "motion": "reduced", "navigation": "spatial" } }), Some("global"), None).unwrap();
        assert_eq!(saved.global.appearance.theme, Theme::Black);
        let bad = |patch: serde_json::Value, needle: &str| {
            let error = app.store.update_settings(json!({ "appearance": patch }), Some("global"), None).unwrap_err().to_string();
            assert!(error.contains(needle), "'{error}' should mention '{needle}'");
        };
        bad(json!({ "posterScale": 79 }), "between 80% and 140%");
        bad(json!({ "posterScale": 141 }), "between 80% and 140%");
        bad(json!({ "theme": "light" }), "light");
        bad(json!({ "accent": "neon" }), "neon");
        bad(json!({ "titlePlacement": "sideways" }), "sideways");
        bad(json!({ "navigation": "telepathy" }), "telepathy");
        // A rejected change leaves the saved values exactly as they were.
        assert_eq!(app.store.get_settings().unwrap().global.appearance.poster_scale, 125);
    }
    let reopened = App::open(dir.path().join("data"), dir.path().join("res")).unwrap();
    let appearance = reopened.store.get_settings().unwrap().global.appearance;
    assert_eq!((appearance.theme, appearance.accent, appearance.poster_scale), (Theme::Black, Accent::Violet, 125));
    assert_eq!((appearance.title_placement, appearance.show_badges, appearance.motion, appearance.navigation), (TitlePlacement::Overlay, false, MotionPreference::Reduced, NavigationMode::Spatial));
}

#[test]
fn language_source_languages_and_setup_state_are_validated_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    {
        let app = App::open(data.clone(), dir.path().join("res")).unwrap();
        // A new install has not completed setup; the interface language follows the system.
        let defaults = app.store.get_settings().unwrap().global;
        assert!(!defaults.setup.completed && defaults.provider_languages.is_empty());
        assert_eq!(defaults.appearance.locale, "system");
        let bad = |patch: serde_json::Value, needle: &str| {
            let error = app.store.update_settings(patch, Some("global"), None).unwrap_err().to_string();
            assert!(error.contains(needle), "'{error}' should mention '{needle}'");
        };
        for tag in ["EN", "e", "english", "pt_BR", "de-", "x".repeat(21).as_str(), "../etc", "en us"] {
            bad(json!({ "appearance": { "locale": tag } }), "not a valid language tag");
        }
        bad(json!({ "providerLanguages": ["en", "system"] }), "language tags");
        bad(json!({ "providerLanguages": ["en", "Japanese"] }), "language tags");
        bad(json!({ "providerLanguages": (0..41).map(|i| format!("a{}", (b'a' + (i % 26) as u8) as char)).collect::<Vec<_>>() }), "language tags");
        for tag in ["system", "de", "pt-BR", "zh-TW", "ar"] {
            app.store.update_settings(json!({ "appearance": { "locale": tag } }), Some("global"), None).unwrap();
        }
        app.store.update_settings(json!({ "appearance": { "locale": "ar" }, "providerLanguages": ["en", "ja"], "setup": { "completed": true, "skipped": false, "completedAtMs": 123, "version": 1 } }), Some("global"), None).unwrap();
    }
    let reopened = App::open(data, dir.path().join("res")).unwrap();
    let global = reopened.store.get_settings().unwrap().global;
    assert_eq!((global.appearance.locale.as_str(), global.provider_languages.clone()), ("ar", vec!["en".to_string(), "ja".to_string()]));
    assert_eq!((global.setup.completed, global.setup.skipped, global.setup.completed_at_ms, global.setup.version), (true, false, Some(123), 1));
    // Redoing setup only rewrites preferences: library and repositories are untouched by settings.
    reopened.store.update_settings(json!({ "setup": { "completed": false } }), Some("global"), None).unwrap();
    assert!(!reopened.store.get_settings().unwrap().global.setup.completed);
    assert_eq!(reopened.store.get_settings().unwrap().global.provider_languages, vec!["en".to_string(), "ja".to_string()]);
}

#[test]
fn locale_tags_follow_the_documented_shape() {
    for ok in ["system", "en", "de", "pt-BR", "zh-TW", "fil", "sr-Latn", "es-419"] {
        assert!(valid_locale_tag(ok), "{ok}");
    }
    for bad in ["", "E", "english", "EN", "en_US", "en--US", "-en", "en-", "a1", "x".repeat(25).as_str()] {
        assert!(!valid_locale_tag(bad), "{bad:?}");
    }
}
