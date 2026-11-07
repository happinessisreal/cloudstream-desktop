//! Desktop backup/restore (T25): format, safety, conflict handling and rollback. Uses temporary
//! databases only.
use cloudstream_core::{backup::*, models::*, App};
use serde_json::{json, Value};
use std::sync::Arc;

fn app(dir: &std::path::Path) -> Arc<App> {
    App::open(dir.join("data"), dir.join("resources")).unwrap()
}

fn item(name: &str, url: &str) -> MediaItem {
    MediaItem { provider_id: "sample:Provider".into(), name: name.into(), url: url.into(), media_type: Some("Movie".into()), year: Some(2020), ..Default::default() }
}

/// A profile with PIN, library (status + favorite), history, progress, a disabled provider and
/// custom settings, so a round trip has something of every kind to preserve.
fn populate(app: &App) -> (String, MediaItem) {
    let kids = app.store.create_profile(CreateProfileInput { name: "Kids".into(), avatar: Some("popcorn".into()), pin: Some("4321".into()) }).unwrap();
    let movie = item("Big Buck Bunny", "https://example.test/bunny");
    app.store.set_library_status(movie.clone(), WatchStatus::Watching, Some(&kids.id)).unwrap();
    app.store.set_library_favorite(movie.clone(), true, Some(&kids.id)).unwrap();
    app.store.set_library_status(item("Sintel", "https://example.test/sintel"), WatchStatus::PlanToWatch, Some("default")).unwrap();
    let media_id = compute_media_id(&movie.provider_id, &movie.url, None);
    let episode = Episode { name: "Full movie".into(), data: "{\"opaque\":\"日本語\"}".into(), ..Default::default() };
    let mut with_id = movie.clone();
    with_id.media_id = Some(media_id.clone());
    let episode_id = compute_episode_id(&media_id, &episode, None, None);
    app.store
        .save_watch_entry(&WatchEntry { id: episode_id, item: with_id, episode, position: 321.5, duration: 600.0, watched_at: 1_700_000_000_000 }, &kids.id)
        .unwrap();
    app.store.set_provider_disabled("sample:Disabled", true).unwrap();
    app.store.update_settings(json!({ "autoSelectProfile": false, "playback": { "autoNext": false } }), Some("global"), None).unwrap();
    (kids.id, movie)
}

fn snapshot(app: &App, kids: &str) -> Value {
    json!({
        "kids": app.store.library(Some(kids)).unwrap().iter().map(|l| (l.id.clone(), l.watch_status, l.is_favorite)).collect::<Vec<_>>(),
        "default": app.store.library(Some("default")).unwrap().iter().map(|l| (l.id.clone(), l.watch_status)).collect::<Vec<_>>(),
        "history": app.store.history(Some(kids)).unwrap().iter().map(|h| (h.id.clone(), h.position)).collect::<Vec<_>>(),
    })
}

fn export_bytes(app: &App, dir: &std::path::Path, options: &ExportOptions) -> Vec<u8> {
    let path = dir.join("out").join("backup.json");
    app.export_backup_to(&path, options).unwrap();
    std::fs::read(path).unwrap()
}

#[tokio::test]
async fn export_then_restore_round_trips_everything_except_secrets() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = app(source_dir.path());
    let (kids_id, _) = populate(&source);
    let bytes = export_bytes(&source, source_dir.path(), &ExportOptions::default());

    // Secrets and machine state never reach the file.
    let text = String::from_utf8(bytes.clone()).unwrap();
    for forbidden in ["pin_hash", "pin_salt", "4321", "download_job", "\"plugin\""] {
        assert!(!text.contains(forbidden), "backup must not contain {forbidden}");
    }
    let parsed: BackupFile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!((parsed.format.as_str(), parsed.version), (BACKUP_FORMAT, BACKUP_VERSION));
    assert_eq!(parsed.schema_version, source.store.schema_version().unwrap());
    assert!(parsed.source_revision >= 1);
    assert_eq!(parsed.data.profiles.len(), 2);

    let target_dir = tempfile::tempdir().unwrap();
    let target = app(target_dir.path());
    let options = ImportOptions::default();
    let preview = target.preview_import(&bytes, &options).await.unwrap();
    assert_eq!(preview.source, ImportSource::Desktop);
    let section = |name: &str| preview.report.sections.iter().find(|s| s.section == name).unwrap_or_else(|| panic!("no {name} section")).clone();
    assert_eq!((section("profiles").new, section("profiles").unchanged), (1, 1), "Kids is new, default already exists");
    assert_eq!(section("library").new, 2);
    assert_eq!(section("progress").new, 1);
    // A preview changes nothing.
    assert_eq!(target.store.list_profiles().unwrap().len(), 1);
    assert!(target.store.library(Some(&kids_id)).unwrap().is_empty());

    let report = target.apply_import(&bytes, &options, &preview.token).await.unwrap();
    assert_eq!(report.sections.iter().map(|s| (s.section.clone(), s.new)).collect::<Vec<_>>(), preview.report.sections.iter().map(|s| (s.section.clone(), s.new)).collect::<Vec<_>>(), "apply does what the preview said");
    assert_eq!(snapshot(&target, &kids_id), snapshot(&source, &kids_id));
    let restored = target.store.list_profiles().unwrap().into_iter().find(|p| p.id == kids_id).unwrap();
    assert_eq!((restored.name.as_str(), restored.avatar.as_str(), restored.has_pin), ("Kids", "popcorn", false), "the PIN is not carried over");
    assert!(!target.store.provider_is_enabled("sample:Disabled").unwrap());
    let settings = target.store.get_settings().unwrap();
    assert!(!settings.global.playback.auto_next);
    assert!(!settings.global.auto_select_profile);
    assert!(target.store.get_watch_progress(&kids_id, &target.store.history(Some(&kids_id)).unwrap()[0].id).unwrap().is_some_and(|p| p.position_seconds == 321.5));

    // The pre-import safety copy exists and is a valid backup of the state before the import.
    let safety = std::path::PathBuf::from(report.pre_import_backup.clone().unwrap());
    assert!(safety.starts_with(target_dir.path().join("data").join("backups")));
    let safety_file: BackupFile = serde_json::from_slice(&std::fs::read(safety).unwrap()).unwrap();
    assert_eq!(safety_file.data.profiles.len(), 1, "the copy holds the data from before the import");

    // Re-importing the same file changes nothing and reports it as unchanged.
    let again = target.preview_import(&bytes, &options).await.unwrap();
    assert!(again.report.sections.iter().all(|s| s.new == 0 && s.replaced == 0), "{:?}", again.report.sections);
}

#[tokio::test]
async fn damaged_newer_oversized_and_foreign_files_are_refused_before_any_change() {
    let dir = tempfile::tempdir().unwrap();
    let a = app(dir.path());
    populate(&a);
    let good = export_bytes(&a, dir.path(), &ExportOptions::default());
    let target_dir = tempfile::tempdir().unwrap();
    let target = app(target_dir.path());
    let before = target.store.export_backup(&ExportOptions::default(), "t").unwrap().data;
    let options = ImportOptions::default();
    let refused = |bytes: Vec<u8>, needle: &'static str| {
        let target = &target;
        let options = &options;
        async move {
            let error = target.preview_import(&bytes, options).await.err().unwrap_or_else(|| panic!("expected '{needle}' to be refused")).to_string();
            assert!(error.contains(needle), "'{error}' should mention '{needle}'");
        }
    };
    refused(Vec::new(), "empty").await;
    refused(b"not json at all".to_vec(), "not a valid backup").await;
    refused(b"[1,2,3]".to_vec(), "not a backup").await;
    refused(br#"{"hello":"world"}"#.to_vec(), "not a Cloudstream").await;
    // A single edited byte in the data breaks the checksum.
    let mut edited: Value = serde_json::from_slice(&good).unwrap();
    edited["data"]["library"][0]["value"]["isFavorite"] = json!(true);
    refused(serde_json::to_vec(&edited).unwrap(), "checksum").await;
    let mut newer: Value = serde_json::from_slice(&good).unwrap();
    newer["version"] = json!(BACKUP_VERSION + 1);
    refused(serde_json::to_vec(&newer).unwrap(), "newer version").await;
    let mut newer_schema: Value = serde_json::from_slice(&good).unwrap();
    newer_schema["schemaVersion"] = json!(999);
    refused(serde_json::to_vec(&newer_schema).unwrap(), "database version").await;
    refused(vec![b' '; MAX_BACKUP_BYTES + 1], "larger than").await;
    let after = target.store.export_backup(&ExportOptions::default(), "t").unwrap().data;
    assert_eq!(before, after, "refused files must not change anything");
}

#[tokio::test]
async fn an_apply_is_bound_to_the_previewed_file_and_choices() {
    let dir = tempfile::tempdir().unwrap();
    let a = app(dir.path());
    populate(&a);
    let bytes = export_bytes(&a, dir.path(), &ExportOptions::default());
    let target_dir = tempfile::tempdir().unwrap();
    let target = app(target_dir.path());
    let options = ImportOptions::default();
    let preview = target.preview_import(&bytes, &options).await.unwrap();
    let other_options = ImportOptions { conflict_policy: ConflictPolicy::Overwrite, ..Default::default() };
    assert!(target.apply_import(&bytes, &other_options, &preview.token).await.unwrap_err().to_string().contains("Preview the import again"));
    let mut tampered = bytes.clone();
    tampered.extend_from_slice(b"\n");
    assert!(target.apply_import(&tampered, &options, &preview.token).await.is_err());
    assert!(target.apply_import(&bytes, &options, "forged").await.is_err());
    assert_eq!(target.store.list_profiles().unwrap().len(), 1, "nothing was applied");
    assert!(!target_dir.path().join("data").join("backups").exists(), "no safety copy for a refused apply");
}

#[tokio::test]
async fn conflict_policies_decide_which_copy_wins() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = app(source_dir.path());
    let movie = item("Conflict", "https://example.test/conflict");
    source.store.set_library_status(movie.clone(), WatchStatus::Completed, None).unwrap();
    let bytes = export_bytes(&source, source_dir.path(), &ExportOptions { sections: Some(vec!["library".into()]), profile_ids: None });
    let media_id = compute_media_id(&movie.provider_id, &movie.url, None);

    let run = |policy: ConflictPolicy, target_newer: bool| {
        let bytes = bytes.clone();
        let movie = movie.clone();
        let media_id = media_id.clone();
        async move {
            let dir = tempfile::tempdir().unwrap();
            let target = app(dir.path());
            target.store.set_library_status(movie, WatchStatus::Dropped, None).unwrap();
            if !target_newer {
                // Make the local copy older than the backup's.
                let mut entry = target.store.get_library_item(&media_id, None).unwrap().unwrap();
                entry.updated_at = Some(1_000);
                entry.saved_at = 1_000;
                target.store.put("library", &format!("default::{media_id}"), &entry).unwrap();
            }
            let options = ImportOptions { conflict_policy: policy, ..Default::default() };
            let preview = target.preview_import(&bytes, &options).await.unwrap();
            let library = preview.report.sections.iter().find(|s| s.section == "library").unwrap().clone();
            target.apply_import(&bytes, &options, &preview.token).await.unwrap();
            (target.store.get_library_item(&media_id, None).unwrap().unwrap().watch_status, library)
        }
    };
    let (status, section) = run(ConflictPolicy::SkipExisting, false).await;
    assert_eq!((status, section.skipped, section.replaced), (WatchStatus::Dropped, 1, 0));
    let (status, section) = run(ConflictPolicy::Overwrite, true).await;
    assert_eq!((status, section.replaced), (WatchStatus::Completed, 1));
    let (status, _) = run(ConflictPolicy::KeepNewest, true).await;
    assert_eq!(status, WatchStatus::Dropped, "the local copy is newer");
    let (status, _) = run(ConflictPolicy::KeepNewest, false).await;
    assert_eq!(status, WatchStatus::Completed, "the backup's copy is newer");
}

#[tokio::test]
async fn sections_are_selective_and_data_for_missing_profiles_is_never_created() {
    let dir = tempfile::tempdir().unwrap();
    let a = app(dir.path());
    let (kids, _) = populate(&a);
    // Export only the library of the Kids profile.
    let kids_only = export_bytes(&a, dir.path(), &ExportOptions { sections: Some(vec!["library".into()]), profile_ids: Some(vec![kids.clone()]) });
    let parsed: BackupFile = serde_json::from_slice(&kids_only).unwrap();
    assert_eq!((parsed.data.library.len(), parsed.data.progress.len(), parsed.data.history.len(), parsed.data.profiles.len()), (1, 0, 0, 0));
    assert!(parsed.data.settings.is_none());

    // Restoring into an app that has no such profile skips the data and says so.
    let target_dir = tempfile::tempdir().unwrap();
    let target = app(target_dir.path());
    let options = ImportOptions::default();
    let preview = target.preview_import(&kids_only, &options).await.unwrap();
    let library = preview.report.sections.iter().find(|s| s.section == "library").unwrap();
    assert_eq!((library.incoming, library.new, library.skipped), (1, 0, 1));
    target.apply_import(&kids_only, &options, &preview.token).await.unwrap();
    assert_eq!(target.store.list_profiles().unwrap().len(), 1);

    // A full backup restored with only the settings section leaves user data alone.
    let full = export_bytes(&a, dir.path(), &ExportOptions::default());
    let only_settings = ImportOptions { sections: Some(vec!["settings".into()]), ..Default::default() };
    let preview = target.preview_import(&full, &only_settings).await.unwrap();
    assert_eq!(preview.report.sections.len(), 1);
    target.apply_import(&full, &only_settings, &preview.token).await.unwrap();
    assert!(!target.store.get_settings().unwrap().global.playback.auto_next);
    assert!(target.store.library(Some("default")).unwrap().is_empty());
    let error = target.preview_import(&full, &ImportOptions { sections: Some(vec!["bogus".into()]), ..Default::default() }).await.unwrap_err();
    assert!(error.to_string().contains("no 'bogus' section"), "{error}");
}

#[tokio::test]
async fn a_failure_part_way_rolls_back_every_change() {
    let dir = tempfile::tempdir().unwrap();
    let target = app(dir.path());
    let before = target.store.export_backup(&ExportOptions::default(), "t").unwrap().data;
    let mut good_row = serde_json::Map::new();
    good_row.insert("profile_id".into(), json!("default"));
    good_row.insert("media_id".into(), json!("m"));
    good_row.insert("episode_id".into(), json!("e1"));
    good_row.insert("position_seconds".into(), json!(5.0));
    good_row.insert("updated_at_ms".into(), json!(1));
    let mut bad_row = good_row.clone();
    bad_row.insert("episode_id".into(), json!("e2"));
    bad_row.insert("position_seconds".into(), Value::Null); // violates NOT NULL
    let library = BackupRecord {
        id: "default::lib".into(),
        value: json!({ "id": "lib", "item": { "providerId": "p", "name": "N", "url": "u" }, "savedAt": 1 }),
        updated: 1,
    };
    let set = ImportSet {
        new_profiles: vec![NewProfile { key: "x".into(), name: "Will be rolled back".into(), avatar: "default".into() }],
        library: vec![library],
        progress: vec![good_row, bad_row],
        ..Default::default()
    };
    let error = target.store.run_import(&set, &ImportOptions::default(), false).unwrap_err();
    assert!(!error.to_string().is_empty());
    let after = target.store.export_backup(&ExportOptions::default(), "t").unwrap().data;
    assert_eq!(before, after, "profile, library entry and first progress row must all be rolled back");
    // The same set passes a dry run on valid data without leaving anything behind.
    let dry = ImportSet { progress: vec![], ..set };
    let report = target.store.run_import(&dry, &ImportOptions::default(), true).unwrap();
    assert_eq!(report.created_profiles.len(), 1);
    assert_eq!(before, target.store.export_backup(&ExportOptions::default(), "t").unwrap().data);
}

#[test]
fn checksum_is_independent_of_key_order() {
    let a = json!({ "b": 1, "a": { "y": 2, "x": [1, 2] } });
    let b: Value = serde_json::from_str(r#"{ "a": { "x": [1, 2], "y": 2 }, "b": 1 }"#).unwrap();
    assert_eq!(canonical_checksum(&a).unwrap(), canonical_checksum(&b).unwrap());
    assert_ne!(canonical_checksum(&a).unwrap(), canonical_checksum(&json!({ "b": 2 })).unwrap());
}
