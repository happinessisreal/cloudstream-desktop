#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use cloudstream_core::{models::*, App};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tauri::{Manager, State};
type Backend = Arc<App>;
type Reply<T> = Result<T, String>;
fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
async fn snapshot(app: State<'_, Backend>) -> Reply<Value> {
    Ok(
        json!({
            "repositories": app.store.repositories().map_err(error)?,
            "installed": app.store.installed().map_err(error)?,
            "library": app.store.library().map_err(error)?,
            "history": app.store.history().map_err(error)?,
            "status": app.status().await,
            "extensionStates": app.extension_states().await.map_err(error)?,
            "settings": app.store.get_settings().map_err(error)?,
            "providers": app.providers().await.unwrap_or_default(),
        }),
    )
}
#[tauri::command]
async fn get_settings(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<Settings> {
    let _ = profile_id;
    app.store.get_settings().map_err(error)
}
#[tauri::command]
async fn update_settings(
    app: State<'_, Backend>,
    patch: serde_json::Value,
    scope: Option<String>,
    profile_id: Option<String>,
) -> Reply<Settings> {
    app.store
        .update_settings(patch, scope.as_deref(), profile_id.as_deref())
        .map_err(error)
}
#[tauri::command]
async fn add_repository(app: State<'_, Backend>, url: String) -> Reply<Repository> {
    app.add_repository(&url).await.map_err(error)
}
#[tauri::command]
async fn refresh_repository(app: State<'_, Backend>, url: String) -> Reply<Repository> {
    app.refresh_repository(&url).await.map_err(error)
}
#[tauri::command]
async fn rename_repository(
    app: State<'_, Backend>,
    url: String,
    name: Option<String>,
) -> Reply<Repository> {
    app.rename_repository(&url, name).map_err(error)
}
#[tauri::command]
async fn install_local_extension(app: State<'_, Backend>, path: String) -> Reply<InstalledPlugin> {
    app.install_local(std::path::Path::new(&path)).await.map_err(error)
}
#[tauri::command]
async fn set_provider_enabled(
    app: State<'_, Backend>,
    provider_id: String,
    enabled: bool,
) -> Reply<()> {
    app.set_provider_enabled(&provider_id, enabled).map_err(error)
}
#[tauri::command]
async fn configure_provider_clone(
    app: State<'_, Backend>,
    clone: ProviderCloneConfig,
) -> Reply<()> {
    app.configure_provider_clone(clone).map_err(error)
}
#[tauri::command]
async fn delete_provider_clone(app: State<'_, Backend>, clone_id: String) -> Reply<()> {
    app.delete_provider_clone(&clone_id).map_err(error)
}
#[tauri::command]
async fn remove_repository(app: State<'_, Backend>, url: String) -> Reply<()> {
    app.remove_repository(&url).await.map_err(error)
}
#[tauri::command]
async fn install_extension(
    app: State<'_, Backend>,
    repository_url: String,
    internal_name: String,
) -> Reply<InstalledPlugin> {
    app.install(&repository_url, &internal_name)
        .await
        .map_err(error)
}
#[tauri::command]
async fn install_sample(app: State<'_, Backend>) -> Reply<InstalledPlugin> {
    app.install_sample().await.map_err(error)
}
#[tauri::command]
async fn check_for_updates(app: State<'_, Backend>) -> Reply<Vec<ExtensionUpdate>> {
    app.check_for_updates().await.map_err(error)
}
#[tauri::command]
async fn update_extension(app: State<'_, Backend>, id: String) -> Reply<InstalledPlugin> {
    app.update_extension(&id).await.map_err(error)
}
#[tauri::command]
async fn update_all_extensions(app: State<'_, Backend>) -> Reply<Vec<BulkUpdateResult>> {
    app.update_all_extensions().await.map_err(error)
}
#[tauri::command]
async fn test_provider(
    app: State<'_, Backend>,
    provider_id: String,
    request_id: Option<String>,
) -> Reply<ProviderTestReport> {
    app.test_provider(&provider_id, request_id.as_deref())
        .await
        .map_err(error)
}
#[tauri::command]
async fn remove_extension(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.remove_plugin(&id).await.map_err(error)
}
#[tauri::command]
async fn list_providers(app: State<'_, Backend>) -> Reply<Vec<Provider>> {
    app.providers().await.map_err(error)
}
#[tauri::command]
async fn provider_home(
    app: State<'_, Backend>,
    provider_id: String,
    page: u32,
    section: Option<u32>,
    force_refresh: Option<bool>,
    request_id: Option<String>,
) -> Reply<Value> {
    app.provider_home(
        &provider_id,
        page,
        section.unwrap_or(0),
        force_refresh.unwrap_or(false),
        request_id.as_deref(),
    )
    .await
    .map_err(error)
}
#[tauri::command]
async fn search(
    app: State<'_, Backend>,
    provider_id: String,
    query: String,
    page: u32,
    request_id: Option<String>,
) -> Reply<Value> {
    app.call_cancellable(
        request_id.as_deref(),
        "search",
        json!({"providerId":provider_id,"query":query,"page":page}),
    )
    .await
    .map_err(error)
}
#[tauri::command]
async fn quick_search(
    app: State<'_, Backend>,
    provider_id: String,
    query: String,
    request_id: Option<String>,
) -> Reply<SearchResult> {
    app.quick_search_provider(&provider_id, &query, request_id.as_deref())
        .await
        .map_err(error)
}
#[tauri::command]
fn get_search_history(
    app: State<'_, Backend>,
    profile_id: Option<String>,
) -> Reply<Vec<SearchHistoryEntry>> {
    let pid = profile_id.unwrap_or_else(|| "default".to_string());
    app.get_search_history(&pid).map_err(error)
}
#[tauri::command]
fn add_search_history(
    app: State<'_, Backend>,
    profile_id: Option<String>,
    query: String,
) -> Reply<SearchHistoryEntry> {
    let pid = profile_id.unwrap_or_else(|| "default".to_string());
    app.add_search_history(&pid, &query).map_err(error)
}
#[tauri::command]
fn delete_search_history_entry(
    app: State<'_, Backend>,
    profile_id: Option<String>,
    query: String,
) -> Reply<()> {
    let pid = profile_id.unwrap_or_else(|| "default".to_string());
    app.delete_search_history_entry(&pid, &query).map_err(error)
}
#[tauri::command]
fn clear_search_history(
    app: State<'_, Backend>,
    profile_id: Option<String>,
) -> Reply<()> {
    let pid = profile_id.unwrap_or_else(|| "default".to_string());
    app.clear_search_history(&pid).map_err(error)
}
#[tauri::command]
async fn search_suggestions(
    app: State<'_, Backend>,
    profile_id: Option<String>,
    query: String,
    provider_id: Option<String>,
) -> Reply<Vec<String>> {
    let pid = profile_id.unwrap_or_else(|| "default".to_string());
    app.search_suggestions(&pid, &query, provider_id.as_deref())
        .await
        .map_err(error)
}
#[tauri::command]
async fn load_details(
    app: State<'_, Backend>,
    provider_id: String,
    url: String,
    request_id: Option<String>,
) -> Reply<MediaDetails> {
    serde_json::from_value(
        app.call_cancellable(
            request_id.as_deref(),
            "load",
            json!({"providerId":provider_id,"url":url}),
        )
        .await
        .map_err(error)?,
    )
    .map_err(error)
}
#[tauri::command]
async fn resolve_links(
    app: State<'_, Backend>,
    provider_id: String,
    data: String,
    request_id: Option<String>,
) -> Reply<ResolvedLinks> {
    serde_json::from_value(
        app.call_cancellable(
            request_id.as_deref(),
            "loadLinks",
            json!({"providerId":provider_id,"data":data}),
        )
        .await
        .map_err(error)?,
    )
    .map_err(error)
}
#[tauri::command]
async fn cancel_request(app: State<'_, Backend>, request_id: String) -> Reply<bool> {
    app.cancel_request(&request_id).await.map_err(error)
}
#[tauri::command]
async fn bookmark(app: State<'_, Backend>, item: MediaItem) -> Reply<LibraryItem> {
    app.bookmark(item).map_err(error)
}
#[tauri::command]
async fn remove_bookmark(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.store.remove("library", &id).map_err(error)
}
#[tauri::command]
async fn set_library_status(
    app: State<'_, Backend>,
    item: MediaItem,
    status: WatchStatus,
) -> Reply<LibraryItem> {
    app.set_library_status(item, status).map_err(error)
}
#[tauri::command]
async fn set_library_favorite(
    app: State<'_, Backend>,
    item: MediaItem,
    is_favorite: bool,
) -> Reply<LibraryItem> {
    app.set_library_favorite(item, is_favorite).map_err(error)
}
#[tauri::command]
async fn set_library_subscribed(
    app: State<'_, Backend>,
    item: MediaItem,
    is_subscribed: bool,
) -> Reply<LibraryItem> {
    app.set_library_subscribed(item, is_subscribed).map_err(error)
}
#[tauri::command]
async fn remove_library_item(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.remove_library_item(&id).map_err(error)
}
#[tauri::command]
async fn clear_library(
    app: State<'_, Backend>,
    status: Option<WatchStatus>,
) -> Reply<()> {
    app.clear_library(status).map_err(error)
}
#[tauri::command]
async fn remove_history_item(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.remove_history_item(&id).map_err(error)
}
#[tauri::command]
async fn clear_history(app: State<'_, Backend>) -> Reply<()> {
    app.clear_history().map_err(error)
}
#[tauri::command]
async fn get_media_watch_progress(
    app: State<'_, Backend>,
    media_id: String,
    profile_id: Option<String>,
) -> Reply<Vec<WatchProgress>> {
    app.get_media_watch_progress(&media_id, profile_id.as_deref())
        .map_err(error)
}
#[tauri::command]
async fn mark_episode_watched(
    app: State<'_, Backend>,
    media_id: String,
    episode_id: String,
    is_watched: bool,
    profile_id: Option<String>,
) -> Reply<WatchProgress> {
    app.mark_episode_watched(&media_id, &episode_id, is_watched, profile_id.as_deref())
        .map_err(error)
}
#[tauri::command]
async fn mark_episodes_up_to(
    app: State<'_, Backend>,
    media_id: String,
    target_episode_id: String,
    episodes: Vec<Episode>,
    profile_id: Option<String>,
) -> Reply<Vec<WatchProgress>> {
    app.mark_episodes_up_to(&media_id, &target_episode_id, &episodes, profile_id.as_deref())
        .map_err(error)
}
#[tauri::command]
async fn play(
    app: State<'_, Backend>,
    stream: Stream,
    subtitles: Vec<Subtitle>,
    item: Option<MediaItem>,
    episode: Option<Episode>,
    resume: f64,
) -> Reply<()> {
    app.play(stream, subtitles, item, episode, resume)
        .await
        .map_err(error)
}
#[tauri::command]
async fn player_action(app: State<'_, Backend>, action: String) -> Reply<Value> {
    let command = match action.as_str() {
        "pause" => json!(["cycle", "pause"]),
        "forward" => json!(["seek", 10]),
        "back" => json!(["seek", -10]),
        "fullscreen" => json!(["cycle", "fullscreen"]),
        "stop" => {
            app.save_progress().await.map_err(error)?;
            app.player.stop().await;
            return Ok(Value::Null);
        }
        _ => return Err("Unknown player action".into()),
    };
    app.player.command(command).await.map_err(error)
}

fn main() {
    tauri::Builder::default()
        .setup(|tauri_app| {
            let packaged_resources = tauri_app.path().resource_dir()?.join("resources");
            let resources = std::env::var_os("CLOUDSTREAM_RESOURCE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    if packaged_resources.join("worker/worker.jar").is_file() {
                        packaged_resources
                    } else {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")
                    }
                });
            let backend = App::open(tauri_app.path().app_data_dir()?, resources)?;
            tauri_app.manage(backend.clone());
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    if let Err(error) = backend.save_progress().await {
                        eprintln!("Progress could not be saved: {error}");
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            add_repository,
            refresh_repository,
            rename_repository,
            remove_repository,
            install_extension,
            install_local_extension,
            install_sample,
            remove_extension,
            check_for_updates,
            update_extension,
            update_all_extensions,
            test_provider,
            list_providers,
            set_provider_enabled,
            configure_provider_clone,
            delete_provider_clone,
            provider_home,
            search,
            quick_search,
            get_search_history,
            add_search_history,
            delete_search_history_entry,
            clear_search_history,
            search_suggestions,
            load_details,
            resolve_links,
            cancel_request,
            bookmark,
            remove_bookmark,
            set_library_status,
            set_library_favorite,
            set_library_subscribed,
            remove_library_item,
            clear_library,
            remove_history_item,
            clear_history,
            get_media_watch_progress,
            mark_episode_watched,
            mark_episodes_up_to,
            play,
            player_action,
            get_settings,
            update_settings
        ])
        .build(tauri::generate_context!())
        .expect("Could not start Cloudstream Desktop")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let app = handle.state::<Backend>().inner().clone();
                tauri::async_runtime::block_on(async move {
                    let _ = app.save_progress().await;
                    app.player.stop().await;
                    app.worker.stop().await;
                });
            }
        });
}
