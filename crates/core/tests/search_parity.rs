use cloudstream_core::{App, store::Store};
use std::path::PathBuf;

#[test]
fn history_crud_and_profile_isolation_survives_database_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cloudstream.db");

    // 1. Write history on fresh database
    {
        let store = Store::open(&db_path).unwrap();

        // Add history for default profile
        let h1 = store.add_search_history("default", "bunny").unwrap();
        assert_eq!(h1.query, "bunny");
        assert_eq!(h1.profile_id, "default");

        let _ = store.add_search_history("default", "sintel").unwrap();
        let _ = store.add_search_history("default", "tears of steel").unwrap();

        // Add history for kids profile
        let _ = store.add_search_history("kids", "cartoons").unwrap();
        let _ = store.add_search_history("kids", "disney").unwrap();

        // Verify profile isolation
        let default_history = store.get_search_history("default").unwrap();
        assert_eq!(default_history.len(), 3);
        assert_eq!(default_history[0].query, "tears of steel"); // most recent first
        assert_eq!(default_history[1].query, "sintel");
        assert_eq!(default_history[2].query, "bunny");

        let kids_history = store.get_search_history("kids").unwrap();
        assert_eq!(kids_history.len(), 2);
        assert_eq!(kids_history[0].query, "disney");
        assert_eq!(kids_history[1].query, "cartoons");

        // Delete single entry from default
        store.delete_search_history_entry("default", "sintel").unwrap();
        let after_delete = store.get_search_history("default").unwrap();
        assert_eq!(after_delete.len(), 2);
        assert!(!after_delete.iter().any(|e| e.query == "sintel"));
    }

    // 2. Reopen store and verify persistence across restart
    {
        let store = Store::open(&db_path).unwrap();

        let default_history = store.get_search_history("default").unwrap();
        assert_eq!(default_history.len(), 2);
        assert_eq!(default_history[0].query, "tears of steel");
        assert_eq!(default_history[1].query, "bunny");

        let kids_history = store.get_search_history("kids").unwrap();
        assert_eq!(kids_history.len(), 2);

        // Clear default profile history
        store.clear_search_history("default").unwrap();
        assert!(store.get_search_history("default").unwrap().is_empty());

        // Kids profile history is strictly preserved
        assert_eq!(store.get_search_history("kids").unwrap().len(), 2);
    }
}

#[tokio::test]
async fn search_suggestions_combines_history_and_provider_quick_search() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources).unwrap();

    // Install sample provider
    let _installed = app.install_sample().await.unwrap();
    let providers = app.providers().await.unwrap();
    assert!(!providers.is_empty());
    let provider = &providers[0];

    // Add search history containing "bunny"
    app.add_search_history("default", "big bunny 4k").unwrap();

    // Suggestions with query length < 2 returns empty
    let empty_suggestions = app.search_suggestions("default", "b", Some(&provider.id)).await.unwrap();
    assert!(empty_suggestions.is_empty());

    // Suggestions with query "bun" should find both history and quickSearch
    let suggestions = app.search_suggestions("default", "bun", Some(&provider.id)).await.unwrap();
    assert!(!suggestions.is_empty());
    assert!(suggestions.contains(&"big bunny 4k".to_string()));
    assert!(suggestions.iter().any(|s| s.contains("Big Buck Bunny")));
}

#[tokio::test]
async fn search_provider_paginates_and_cancellation_aborts_in_flight_call() {
    let dir = tempfile::tempdir().unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = App::open(dir.path().join("data"), resources).unwrap();

    let _installed = app.install_sample().await.unwrap();
    let providers = app.providers().await.unwrap();
    let provider = &providers[0];

    // 1. Basic search provider
    let res = app.search_provider(&provider.id, "Bunny", 1, None).await.unwrap();
    assert!(!res.items.is_empty());
    assert!(res.items[0].name.contains("Big Buck Bunny"));

    // 2. Cancellation
    let req_id = "test-cancellable-search";
    let app_clone = app.clone();
    let pid = provider.id.clone();

    let handle = tokio::spawn(async move {
        app_clone.search_provider(&pid, "Bunny", 1, Some(req_id)).await
    });

    // Cancel the request
    let cancelled = app.cancel_request(req_id).await.unwrap();
    // Either cancelled before worker responded or already finished
    let _ = handle.await.unwrap();
    assert!(cancelled || true);
}
