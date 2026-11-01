use cloudstream_core::{
    models::*,
    store::{execute_migrations, Migration, Store, CURRENT_SCHEMA_VERSION},
};
use rusqlite::Connection;
use serde_json::json;

const V1_DB_BYTES: &[u8] = include_bytes!("fixtures/v1_database.db");

#[test]
fn version_1_database_fixture_survives_upgrade_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("upgraded.db");

    std::fs::write(&db_path, V1_DB_BYTES).unwrap();

    // Verify initial state before opening with Store
    {
        let conn = Connection::open(&db_path).unwrap();
        let initial_version: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(initial_version, 1, "Fixture must be at schema version 1");
    }

    // Open with Store which runs migrations to CURRENT_SCHEMA_VERSION
    {
        let store = Store::open(&db_path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            CURRENT_SCHEMA_VERSION,
            "Store must be upgraded to CURRENT_SCHEMA_VERSION"
        );

        // Verify v1 repository record
        let repos = store.repositories().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].url, "https://example.com/repo.json");
        assert_eq!(repos[0].manifest.name, "V1 Test Repo");

        // Verify v1 plugin record
        let plugins = store.installed().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "v1-plugin-id");
        assert_eq!(plugins[0].entry.name, "V1 Plugin");
        assert_eq!(plugins[0].runtime, RuntimeKind::Jvm);

        // Verify v1 bookmark record
        let library = store.library().unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].id, "v1-lib-id");
        assert_eq!(library[0].item.name, "V1 Bookmark");

        // Verify v1 history record
        let history = store.history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, "v1-hist-id");
        assert_eq!(history[0].episode.name, "Episode 1");
        assert_eq!(history[0].position, 120.0);

        // Verify default settings created
        let settings = store.get_settings().unwrap();
        assert_eq!(settings.schema_version, 1);
        assert_eq!(
            settings.global.appearance.poster_density,
            PosterDensity::Comfortable
        );
    }

    // Reopen and ensure all records still survive
    {
        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(store.repositories().unwrap().len(), 1);
        assert_eq!(store.installed().unwrap().len(), 1);
        assert_eq!(store.library().unwrap().len(), 1);
        assert_eq!(store.history().unwrap().len(), 1);
    }
}

#[test]
fn failed_migration_rolls_back_transaction_and_preserves_version() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("rollback_test.db");

    std::fs::write(&db_path, V1_DB_BYTES).unwrap();

    let mut conn = Connection::open(&db_path).unwrap();
    let initial_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(initial_version, 1);

    // Define a migration sequence where version 2 fails
    let failing_migrations = vec![
        Migration {
            version: 2,
            description: "Intentionally broken migration",
            run: |tx| {
                tx.execute_batch("CREATE TABLE partial_table (id INT);")?;
                // Inject failure
                anyhow::bail!("Simulated failure in migration step");
            },
        },
    ];

    let result = execute_migrations(&mut conn, &failing_migrations);
    assert!(result.is_err(), "Migration execution must fail");

    // Check that user_version is still 1 and partial_table was rolled back
    let version_after_failure: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        version_after_failure, 1,
        "user_version must remain 1 after failed migration"
    );

    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='partial_table'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        > 0;
    assert!(
        !table_exists,
        "Table created inside failed migration must have been rolled back"
    );

    // Existing records still intact
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4, "Existing records must survive rollback");
}

#[test]
fn settings_defaults_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("settings.db")).unwrap();

    // Absent settings gets default
    let default_s = store.get_settings().unwrap();
    assert_eq!(default_s.schema_version, 1);
    assert_eq!(default_s.revision, 1);
    assert_eq!(
        default_s.global.appearance.poster_density,
        PosterDensity::Comfortable
    );

    // Updating with invalid posterDensity is rejected
    let bad_update = store.update_settings(
        json!({"appearance": {"posterDensity": "unsupported_density"}}),
        None,
        None,
    );
    assert!(bad_update.is_err());
    let err_msg = bad_update.unwrap_err().to_string();
    assert!(
        err_msg.contains("Invalid poster density"),
        "Error message should mention invalid poster density: {err_msg}"
    );

    // Updating with invalid schemaVersion (0) is rejected
    let bad_schema = store.update_settings(json!({"schemaVersion": 0}), None, None);
    assert!(bad_schema.is_err());

    // Settings remained unchanged after rejected updates
    let unchanged = store.get_settings().unwrap();
    assert_eq!(
        unchanged.global.appearance.poster_density,
        PosterDensity::Comfortable
    );
    assert_eq!(unchanged.revision, 1);
}

#[test]
fn settings_update_persists_across_restart_and_preserves_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("persistence.db");

    {
        let store = Store::open(&db_path).unwrap();

        // Update settings with valid posterDensity AND unknown future fields
        let updated = store
            .update_settings(
                json!({
                    "appearance": {
                        "posterDensity": "compact",
                        "customColorFilter": "blue"
                    },
                    "unknownFutureModule": {
                        "enabled": true,
                        "retryLimit": 5
                    },
                    "topLevelFutureField": "preserved"
                }),
                None,
                None,
            )
            .unwrap();

        assert_eq!(
            updated.global.appearance.poster_density,
            PosterDensity::Compact
        );
        assert_eq!(updated.revision, 2);
    }

    // Reopen store from disk (simulating restart)
    {
        let store = Store::open(&db_path).unwrap();
        let loaded = store.get_settings().unwrap();

        assert_eq!(
            loaded.global.appearance.poster_density,
            PosterDensity::Compact
        );
        assert_eq!(loaded.revision, 2);

        // Unknown future fields must be preserved
        assert_eq!(
            loaded.extra.get("topLevelFutureField").unwrap(),
            "preserved"
        );
        assert_eq!(
            loaded
                .extra
                .get("unknownFutureModule")
                .unwrap()
                .get("enabled")
                .unwrap(),
            true
        );
        assert_eq!(
            loaded
                .global
                .appearance
                .extra
                .get("customColorFilter")
                .unwrap(),
            "blue"
        );

        // Now perform an unrelated update (changing poster density back to comfortable)
        let updated2 = store
            .update_settings(
                json!({"appearance": {"posterDensity": "comfortable"}}),
                None,
                None,
            )
            .unwrap();

        assert_eq!(
            updated2.global.appearance.poster_density,
            PosterDensity::Comfortable
        );
        assert_eq!(updated2.revision, 3);

        // Unknown future fields must STILL be preserved after unrelated update!
        assert_eq!(
            updated2.extra.get("topLevelFutureField").unwrap(),
            "preserved"
        );
        assert_eq!(
            updated2
                .extra
                .get("unknownFutureModule")
                .unwrap()
                .get("enabled")
                .unwrap(),
            true
        );
        assert_eq!(
            updated2
                .global
                .appearance
                .extra
                .get("customColorFilter")
                .unwrap(),
            "blue"
        );
    }
}

#[test]
fn profile_scoped_settings_updates() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("profiles.db")).unwrap();

    // Update profile settings
    let updated = store
        .update_settings(
            json!({"appearance": {"posterDensity": "compact"}}),
            Some("profile"),
            Some("kids-profile"),
        )
        .unwrap();

    // Global appearance remains default
    assert_eq!(
        updated.global.appearance.poster_density,
        PosterDensity::Comfortable
    );

    // Profile appearance has compact
    let profile = updated.profiles.get("kids-profile").unwrap();
    assert_eq!(
        profile.appearance.as_ref().unwrap().poster_density,
        PosterDensity::Compact
    );
}
