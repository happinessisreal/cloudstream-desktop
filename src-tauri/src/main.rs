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
            "settings": app.store.get_settings().map_err(error)?
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
    app.repositories.add(&app.store, &url).await.map_err(error)
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
    request_id: Option<String>,
) -> Reply<Value> {
    app.call_cancellable(
        request_id.as_deref(),
        "getMainPage",
        json!({"providerId":provider_id,"page":page,"section":section.unwrap_or(0)}),
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
            remove_repository,
            install_extension,
            install_sample,
            remove_extension,
            list_providers,
            provider_home,
            search,
            load_details,
            resolve_links,
            cancel_request,
            bookmark,
            remove_bookmark,
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
