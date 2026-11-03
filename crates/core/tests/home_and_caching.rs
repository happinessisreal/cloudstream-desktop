use cloudstream_core::{models::*, App};
use serde_json::json;
use std::path::PathBuf;

#[tokio::test]
async fn multi_row_provider_retains_named_lists_with_independent_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let plugin = app.install_sample().await.unwrap();
    assert_eq!(plugin.entry.name, "Blender Open Movies");

    let providers = app.providers().await.unwrap();
    let provider = providers
        .iter()
        .find(|p| p.name == "Blender Open Movies")
        .expect("Provider should be registered");

    // 1. Query section 0 ("Open movies") -> returns 1 list: "Open movies"
    let home_s0 = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();

    let rows_s0 = home_s0.get("rows").and_then(|r| r.as_array()).unwrap();
    assert_eq!(rows_s0.len(), 1);
    assert_eq!(rows_s0[0]["name"].as_str().unwrap(), "Open movies");
    assert_eq!(rows_s0[0]["horizontalImages"].as_bool().unwrap(), true);
    assert_eq!(rows_s0[0]["orientation"].as_str().unwrap(), "horizontal");
    assert_eq!(rows_s0[0]["sectionIndex"].as_u64().unwrap(), 0);
    assert_eq!(rows_s0[0]["listIndex"].as_u64().unwrap(), 0);
    assert!(home_s0["hasMoreSections"].as_bool().unwrap());
    assert_eq!(home_s0["nextSection"].as_u64().unwrap(), 1);

    // 2. Query section 1 ("Community picks") -> returns 2 distinct named lists
    let home_s1 = app
        .provider_home(&provider.id, 1, 1, false, None)
        .await
        .unwrap();

    let rows_s1 = home_s1.get("rows").and_then(|r| r.as_array()).unwrap();
    assert_eq!(rows_s1.len(), 2, "Expected 2 lists from section 1 without flattening");
    assert_eq!(rows_s1[0]["name"].as_str().unwrap(), "Popular open titles");
    assert_eq!(rows_s1[0]["horizontalImages"].as_bool().unwrap(), false);
    assert_eq!(rows_s1[0]["orientation"].as_str().unwrap(), "vertical");
    assert_eq!(rows_s1[0]["sectionIndex"].as_u64().unwrap(), 1);
    assert_eq!(rows_s1[0]["listIndex"].as_u64().unwrap(), 0);

    assert_eq!(rows_s1[1]["name"].as_str().unwrap(), "Classic animations");
    assert_eq!(rows_s1[1]["horizontalImages"].as_bool().unwrap(), false);
    assert_eq!(rows_s1[1]["orientation"].as_str().unwrap(), "vertical");
    assert_eq!(rows_s1[1]["sectionIndex"].as_u64().unwrap(), 1);
    assert_eq!(rows_s1[1]["listIndex"].as_u64().unwrap(), 1);

    // Verify row page cursor for section 1 page 2
    let home_s1_p2 = app
        .provider_home(&provider.id, 2, 1, false, None)
        .await
        .unwrap();
    let rows_s1_p2 = home_s1_p2.get("rows").and_then(|r| r.as_array()).unwrap();
    assert_eq!(rows_s1_p2.len(), 2);
    assert_eq!(rows_s1_p2[0]["page"].as_u64().unwrap(), 2);
    assert_eq!(rows_s1_p2[1]["page"].as_u64().unwrap(), 2);

    app.worker.stop().await;
}

#[tokio::test]
async fn home_caching_and_source_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    app.install_sample().await.unwrap();
    let providers = app.providers().await.unwrap();
    let provider = &providers[0];

    // 1. Initial call should be a cache miss (cached: false)
    let first = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(first["cached"].as_bool(), Some(false));

    // 2. Second identical call should be a cache hit (cached: true)
    let second = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(second["cached"].as_bool(), Some(true));
    assert_eq!(first["rows"], second["rows"]);

    // 3. Force refresh should bypass cache and update (cached: false)
    let refreshed = app
        .provider_home(&provider.id, 1, 0, true, None)
        .await
        .unwrap();
    assert_eq!(refreshed["cached"].as_bool(), Some(false));

    // 4. Source isolation: Configure a clone provider
    let clone_cfg = ProviderCloneConfig {
        id: "clone:Blender Mirror".into(),
        parent_provider_id: provider.id.clone(),
        name: "Blender Mirror".into(),
        custom_url: "https://mirror.blender.org".into(),
        credentials_handle: None,
        enabled: true,
        extra: std::collections::BTreeMap::new(),
    };
    app.configure_provider_clone(clone_cfg).unwrap();

    // Querying clone source MUST NOT return Source A's cached entry
    let clone_home = app
        .provider_home("clone:Blender Mirror", 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(
        clone_home["cached"].as_bool(),
        Some(false),
        "Source B (clone) must NOT use Source A's cache"
    );

    app.worker.stop().await;
}

#[tokio::test]
async fn cache_invalidation_on_source_and_extension_changes() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    let plugin = app.install_sample().await.unwrap();
    let providers = app.providers().await.unwrap();
    let provider = &providers[0];

    // Warm cache
    let first = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(first["cached"].as_bool(), Some(false));

    let second = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(second["cached"].as_bool(), Some(true));

    // Invalidate via provider enable/disable toggle
    app.set_provider_enabled(&provider.id, false).unwrap();

    // Cache entry for that provider is invalidated
    let post_disable = app
        .provider_home(&provider.id, 1, 0, false, None)
        .await
        .unwrap();
    assert_eq!(
        post_disable["cached"].as_bool(),
        Some(false),
        "Cache must be invalidated when provider setting changes"
    );

    // Test extension removal invalidation
    app.set_provider_enabled(&provider.id, true).unwrap();
    let _ = app.provider_home(&provider.id, 1, 0, false, None).await;
    app.remove_plugin(&plugin.id).await.unwrap();

    // After remove, cache is completely purged
    assert!(app.home_cache.lock().unwrap().is_empty());

    app.worker.stop().await;
}

#[tokio::test]
async fn unresolved_errors_are_never_cached_as_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources.clone()).unwrap();

    // Non-existent provider call
    let result = app
        .provider_home("non_existent:Provider", 1, 0, false, None)
        .await;
    assert!(result.is_err(), "Expected error for invalid provider");

    // Cache must remain completely empty
    let cache = app.home_cache.lock().unwrap();
    assert_eq!(
        cache.len(),
        0,
        "Failed calls must not poison cache with empty entries"
    );

    app.worker.stop().await;
}

#[tokio::test]
async fn profile_settings_persists_selected_source_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");

    {
        let app = App::open(data_dir.clone(), resources.clone()).unwrap();
        // Update profile settings with selected provider
        let patch = json!({
            "selectedProvider": "sample:Blender Open Movies"
        });
        let updated = app
            .store
            .update_settings(patch, None, Some("default"))
            .unwrap();

        let profile = updated.profiles.get("default").unwrap();
        assert_eq!(
            profile.selected_provider.as_deref(),
            Some("sample:Blender Open Movies")
        );
        app.worker.stop().await;
    }

    // Reopen store from disk
    {
        let app = App::open(data_dir.clone(), resources.clone()).unwrap();
        let loaded = app.store.get_settings().unwrap();
        let profile = loaded.profiles.get("default").unwrap();
        assert_eq!(
            profile.selected_provider.as_deref(),
            Some("sample:Blender Open Movies"),
            "Selected provider must survive database restart"
        );
        app.worker.stop().await;
    }
}
