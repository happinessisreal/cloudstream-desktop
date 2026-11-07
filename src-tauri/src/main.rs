#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use cloudstream_core::{models::*, App};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tauri::{Emitter, Manager, State};
type Backend = Arc<App>;
type Reply<T> = Result<T, String>;
type DownloadServiceHandle = Arc<DownloadService>;
/// Set once the user has agreed to quit while downloads or playback are running.
#[derive(Default)]
struct ExitConfirmed(std::sync::atomic::AtomicBool);
/// The backup file the user picked, kept here (up to the app's size limit) so the UI only handles
/// an opaque id, never the file's contents.
struct PendingImport { id: String, name: String, bytes: Vec<u8> }
type PendingImportState = Arc<tokio::sync::Mutex<Option<PendingImport>>>;
struct DownloadService { manager: tokio::sync::Mutex<Option<Arc<cloudstream_core::download_manager::DownloadManager>>> }
impl DownloadService {
    async fn ensure(&self, app: &Arc<App>, handle: &tauri::AppHandle) -> Result<Arc<cloudstream_core::download_manager::DownloadManager>, String> {
        let mut guard = self.manager.lock().await;
        if let Some(manager) = guard.as_ref() { return Ok(manager.clone()); }
        let manager = cloudstream_core::download_manager::DownloadManager::start(app, 2).map_err(error)?;
        let mut events = manager.subscribe();
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => { let _ = handle.emit("download_event", event); },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { let _ = handle.emit("download_refresh", ()); },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        *guard = Some(manager.clone());
        Ok(manager)
    }
    async fn shutdown(&self) {
        let manager = self.manager.lock().await.take();
        if let Some(manager) = manager { manager.shutdown().await; }
    }
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
async fn snapshot(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<Value> {
    let pid = profile_id.as_deref().unwrap_or("default");
    Ok(
        json!({
            "repositories": app.store.repositories().map_err(error)?,
            "installed": app.store.installed().map_err(error)?,
            "library": app.store.library(Some(pid)).map_err(error)?,
            "history": app.store.history(Some(pid)).map_err(error)?,
            "status": app.status().await,
            "extensionStates": app.extension_states().await.map_err(error)?,
            "settings": app.store.get_settings().map_err(error)?,
            "providers": app.providers().await.unwrap_or_default(),
            "profiles": app.list_profiles().map_err(error)?,
            "activeProfileId": pid,
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
    let settings = app
        .store
        .update_settings(patch, scope.as_deref(), profile_id.as_deref())
        .map_err(error)?;
    app.apply_player_settings(&settings).await;
    Ok(settings)
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
async fn install_torrentio(app: State<'_, Backend>) -> Reply<InstalledPlugin> {
    app.install_torrentio().await.map_err(error)
}
#[tauri::command]
async fn get_custom_trackers(app: State<'_, Backend>) -> Reply<Vec<String>> {
    app.get_custom_trackers().map_err(error)
}
#[tauri::command]
async fn save_custom_trackers(app: State<'_, Backend>, trackers: Vec<String>) -> Reply<()> {
    app.save_custom_trackers(&trackers).map_err(error)
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
    profile_id: Option<String>,
) -> Reply<MediaDetails> {
    app.load_details(profile_id.as_deref().unwrap_or("default"), &provider_id, &url, request_id.as_deref()).await.map_err(error)
}
#[tauri::command]
async fn get_webview_status(app: State<'_, Backend>) -> Reply<WebViewReport> {
    app.get_webview_status().await.map_err(error)
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
    profile_id: Option<String>,
) -> Reply<LibraryItem> {
    app.set_library_status(item, status, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn set_library_favorite(
    app: State<'_, Backend>,
    item: MediaItem,
    is_favorite: bool,
    profile_id: Option<String>,
) -> Reply<LibraryItem> {
    app.set_library_favorite(item, is_favorite, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn set_library_subscribed(
    app: State<'_, Backend>,
    item: MediaItem,
    is_subscribed: bool,
    profile_id: Option<String>,
) -> Reply<LibraryItem> {
    app.set_library_subscribed(item, is_subscribed, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn check_subscriptions(app: State<'_, Backend>, profile_id: Option<String>, force: Option<bool>) -> Reply<cloudstream_core::subscriptions::SubscriptionCheckReport> {
    app.inner().check_subscriptions(profile_id.as_deref().unwrap_or("default"), force.unwrap_or(true)).await.map_err(error)
}

#[tauri::command]
async fn acknowledge_subscription_updates(app: State<'_, Backend>, profile_id: Option<String>, updates: Vec<cloudstream_core::subscriptions::SubscriptionUpdate>) -> Reply<()> {
    let profile = profile_id.as_deref().unwrap_or("default");
    for update in updates { app.store.acknowledge_subscription_unread(profile, &update).map_err(error)?; }
    Ok(())
}

#[tauri::command]
async fn deliver_subscription_alerts(app: State<'_, Backend>, handle: tauri::AppHandle, profile_id: Option<String>) -> Reply<cloudstream_core::subscriptions::AlertDeliveryReport> {
    use cloudstream_core::{entry::EntryAction, notify};
    let profile = profile_id.unwrap_or_else(|| "default".into());
    let backend = app.inner().clone();
    app.deliver_subscription_alerts(&profile, |alert| {
        let backend = backend.clone();
        let handle = handle.clone();
        async move {
            let count = alert.episode_count();
            let body = format!("{count} new episode{} available", if count == 1 { " is" } else { "s are" });
            // Clicking "Open" opens the title in the running app. Looked up now, so the click
            // needs nothing but the answer.
            let target = backend.store.get_library_item(&alert.media_id, Some(&alert.profile_id)).ok().flatten().map(|entry| (entry.item.provider_id, entry.item.url));
            let notification = notify::Notification {
                title: alert.title.clone(),
                body,
                icon: "dev.cloudstream.desktop".into(),
                action: target.is_some().then(|| ("open".to_string(), "Open".to_string())),
            };
            notify::send("notify-send", &notification, notify::Timing::default(), move |_| {
                if let Some((provider_id, url)) = target {
                    if backend.entries.push(EntryAction::OpenMedia { provider_id, url }) {
                        let _ = handle.emit("entry_pending", ());
                    }
                    focus_main_window(&handle);
                }
            })
            .await
        }
    })
    .await
    .map_err(error)
}

#[tauri::command]
async fn remove_library_item(
    app: State<'_, Backend>,
    id: String,
    profile_id: Option<String>,
) -> Reply<()> {
    app.remove_library_item(&id, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn clear_library(
    app: State<'_, Backend>,
    status: Option<WatchStatus>,
    profile_id: Option<String>,
) -> Reply<()> {
    app.clear_library(status, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn remove_history_item(
    app: State<'_, Backend>,
    id: String,
    profile_id: Option<String>,
) -> Reply<()> {
    app.remove_history_item(&id, profile_id.as_deref()).map_err(error)
}
#[tauri::command]
async fn clear_history(
    app: State<'_, Backend>,
    profile_id: Option<String>,
) -> Reply<()> {
    app.clear_history(profile_id.as_deref()).map_err(error)
}

#[tauri::command]
async fn list_profiles(app: State<'_, Backend>) -> Reply<Vec<Profile>> {
    app.list_profiles().map_err(error)
}

#[tauri::command]
async fn get_profile(app: State<'_, Backend>, id: String) -> Reply<Option<Profile>> {
    app.get_profile(&id).map_err(error)
}

#[tauri::command]
async fn create_profile(app: State<'_, Backend>, input: CreateProfileInput) -> Reply<Profile> {
    app.create_profile(input).map_err(error)
}

#[tauri::command]
async fn update_profile(
    app: State<'_, Backend>,
    id: String,
    input: UpdateProfileInput,
) -> Reply<Profile> {
    app.update_profile(&id, input).map_err(error)
}

#[tauri::command]
async fn delete_profile(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.delete_profile(&id).map_err(error)
}

#[tauri::command]
async fn verify_profile_pin(app: State<'_, Backend>, id: String, pin: String) -> Reply<bool> {
    app.verify_profile_pin(&id, &pin).map_err(error)
}

#[tauri::command]
async fn switch_profile(
    app: State<'_, Backend>,
    target_profile_id: String,
    pin: Option<String>,
) -> Reply<()> {
    app.switch_profile(&target_profile_id, pin.as_deref()).await.map_err(error)
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
    profile_id: Option<String>,
    queue: Option<PlaybackQueue>,
    mirrors: Option<Vec<Stream>>,
) -> Reply<String> {
    app.play_with_mirrors(stream, mirrors.unwrap_or_default(), subtitles, item, episode, resume, profile_id.as_deref(), queue)
        .await
        .map_err(error)
}

#[tauri::command]
async fn get_playback_queue(app: State<'_, Backend>) -> Reply<Option<PlaybackQueue>> {
    Ok(app.player.queue().await)
}

#[tauri::command]
async fn set_playback_queue(app: State<'_, Backend>, queue: Option<PlaybackQueue>) -> Reply<()> {
    app.set_playback_queue(queue).await.map_err(error)
}

#[tauri::command]
async fn get_player_state(app: State<'_, Backend>) -> Reply<PlayerState> {
    Ok(app.player.state().await)
}

#[tauri::command]
async fn player_command(app: State<'_, Backend>, command: PlayerCommand) -> Reply<PlayerState> {
    app.player_command(command).await.map_err(error)
}

#[tauri::command]
async fn player_skip_markers(app: State<'_, Backend>, session_id: String) -> Reply<Vec<cloudstream_core::skip::SkipMarker>> {
    app.player_skip_markers(&session_id).await.map_err(error)
}

#[tauri::command]
async fn player_skip_interval(app: State<'_, Backend>, session_id: String, start: f64, end: f64) -> Reply<()> {
    if !app.store.get_settings().map_err(error)?.global.skip.aniskip { return Err("Skip times are turned off".into()); }
    app.player.skip_interval(&session_id, start, end).await.map_err(error)
}

#[tauri::command]
async fn begin_speed_hold(app: State<'_, Backend>, session_id: String) -> Reply<()> {
    let speed = app.store.get_settings().map_err(error)?.global.player.hold_speed;
    app.player.begin_speed_hold(&session_id, speed).await.map_err(error)
}

#[tauri::command]
async fn end_speed_hold(app: State<'_, Backend>, session_id: String) -> Reply<()> {
    app.player.end_speed_hold(&session_id).await.map_err(error)
}

#[tauri::command]
async fn choose_subtitle_font(app: State<'_, Backend>) -> Reply<Option<cloudstream_core::fonts::ImportedFont>> {
    let file = rfd::AsyncFileDialog::new().set_title("Choose subtitle font")
        .add_filter("Font files", &["ttf", "otf"]).pick_file().await;
    let Some(file) = file else { return Ok(None); };
    app.import_subtitle_font(file.path()).map(Some).map_err(error)
}

#[tauri::command]
async fn subtitle_font_preview(app: State<'_, Backend>, id: String) -> Reply<String> {
    app.subtitle_font_preview(&id).map_err(error)
}

#[tauri::command]
async fn choose_local_subtitle(app: State<'_, Backend>, session_id: String) -> Reply<Option<cloudstream_core::SubtitleFileSelection>> {
    let state = app.player.state().await;
    if !state.active || state.session_id.as_deref() != Some(session_id.as_str()) {
        return Err("Playback changed. Start a video before choosing a subtitle.".into());
    }
    let file = rfd::AsyncFileDialog::new()
        .set_title("Choose subtitle file")
        .add_filter("Subtitle files", &["srt", "vtt", "ass", "ssa", "ttml", "dfxp", "smi", "sami", "zip"])
        .pick_file().await;
    let Some(file) = file else { return Ok(None); };
    app.prepare_local_subtitle(file.path(), &session_id).await.map(Some).map_err(error)
}

#[tauri::command]
async fn prepare_provider_subtitle_archive(app: State<'_, Backend>, session_id: String, index: usize) -> Reply<cloudstream_core::SubtitleFileSelection> {
    app.prepare_provider_subtitle_archive(&session_id, index).await.map_err(error)
}

#[tauri::command]
async fn select_archived_subtitle(app: State<'_, Backend>, id: String, index: usize) -> Reply<()> {
    app.select_archived_subtitle(&id, index).await.map_err(error)
}

#[tauri::command]
async fn cancel_archived_subtitle(app: State<'_, Backend>, id: String) -> Reply<()> {
    app.cancel_archived_subtitle(&id).await;
    Ok(())
}

#[tauri::command]
fn download_jobs(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<Vec<cloudstream_core::download_queue::DownloadJob>> {
    app.store.download_jobs(Some(profile_id.as_deref().unwrap_or("default"))).map_err(error)
}

#[tauri::command]
fn download_watch_progress(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<std::collections::HashMap<String, cloudstream_core::models::WatchProgress>> {
    let profile = profile_id.as_deref().unwrap_or("default");
    let mut result = std::collections::HashMap::new();
    for job in app.store.download_jobs(Some(profile)).map_err(error)? {
        if job.state == cloudstream_core::download_queue::DownloadState::Completed {
            if let Some(progress) = app.store.get_watch_progress(profile, &job.episode_id).map_err(error)? {
                result.insert(job.id, progress);
            }
        }
    }
    Ok(result)
}

#[tauri::command]
async fn download_subtitle_warnings(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<std::collections::HashMap<String, usize>> {
    let jobs = app.store.download_jobs(Some(profile_id.as_deref().unwrap_or("default"))).map_err(error)?;
    tokio::task::spawn_blocking(move || {
        let mut result = std::collections::HashMap::new();
        for job in jobs.into_iter().filter(|job| job.state == cloudstream_core::download_queue::DownloadState::Completed) {
            let missing = if job.extension == cloudstream_core::downloads::HLS_EXTENSION {
                cloudstream_core::hls_download::read_manifest(&job.destination, job.destination_identity, &job.id).ok().flatten().map(|manifest| manifest.missing_external_subtitles)
            } else {
                cloudstream_core::direct_subtitles::read(&job.destination, job.destination_identity, &job.id, &job.extension).ok().flatten().map(|(_, missing)| missing)
            };
            if let Some(missing) = missing.filter(|missing| *missing > 0) { result.insert(job.id, missing); }
        }
        result
    }).await.map_err(error)
}

#[tauri::command]
async fn download_health(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<std::collections::HashMap<String, String>> {
    let jobs = app.store.download_jobs(Some(profile_id.as_deref().unwrap_or("default"))).map_err(error)?;
    tokio::task::spawn_blocking(move || {
        jobs.into_iter().filter(|job| job.state == cloudstream_core::download_queue::DownloadState::Completed).map(|job| {
            let status = match cloudstream_core::download_manager::download_is_healthy(&job) {
                Ok(true) => "available", Ok(false) => "damaged", Err(_) => "unavailable",
            };
            (job.id, status.to_string())
        }).collect()
    }).await.map_err(error)
}

#[tauri::command]
async fn repair_download(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, id: String) -> Reply<cloudstream_core::download_queue::DownloadJob> {
    service.ensure(app.inner(), &handle).await?.repair(profile_id.as_deref().unwrap_or("default"), &id).await.map_err(error)
}

#[tauri::command]
async fn play_download(app: State<'_, Backend>, id: String, profile_id: Option<String>) -> Reply<String> {
    app.play_download(&id, profile_id.as_deref().unwrap_or("default")).await.map_err(error)
}

#[tauri::command]
async fn enqueue_download(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, item: MediaItem, episode: Episode, extension: String,
    policy: cloudstream_core::download_queue::DownloadSourcePolicy) -> Reply<Option<cloudstream_core::download_queue::DownloadJob>> {
    let manager = service.ensure(app.inner(), &handle).await?;
    let Some(folder) = rfd::AsyncFileDialog::new().set_title("Choose a folder for this download").pick_folder().await else { return Ok(None) };
    let request = cloudstream_core::download_queue::DownloadRequest { item, episode, destination: folder.path().to_owned(), extension, policy };
    manager.enqueue(profile_id.as_deref().unwrap_or("default"), request).map(Some).map_err(error)
}

/// Enqueue several episodes of one title with a single folder choice. Duplicates are reported,
/// not re-added, and the batch is bounded so one click cannot flood the queue.
#[tauri::command]
async fn enqueue_downloads(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, item: MediaItem, episodes: Vec<Episode>, extension: String,
    policy: cloudstream_core::download_queue::DownloadSourcePolicy) -> Reply<Option<Value>> {
    const BATCH_LIMIT: usize = 500;
    if episodes.is_empty() { return Err("Choose at least one episode".into()); }
    if episodes.len() > BATCH_LIMIT { return Err(format!("Choose at most {BATCH_LIMIT} episodes at a time")); }
    let manager = service.ensure(app.inner(), &handle).await?;
    let Some(folder) = rfd::AsyncFileDialog::new().set_title("Choose a folder for these downloads").pick_folder().await else { return Ok(None) };
    let profile = profile_id.as_deref().unwrap_or("default");
    let requests = episodes.into_iter().map(|episode| cloudstream_core::download_queue::DownloadRequest {
        item: item.clone(), episode, destination: folder.path().to_owned(), extension: extension.clone(), policy: policy.clone(),
    }).collect();
    let summary = manager.enqueue_many(profile, requests).map_err(error)?;
    Ok(Some(serde_json::to_value(summary).map_err(error)?))
}

#[tauri::command]
async fn control_download(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, id: String, action: cloudstream_core::download_queue::DownloadAction) -> Reply<cloudstream_core::download_queue::DownloadJob> {
    service.ensure(app.inner(), &handle).await?.control(profile_id.as_deref().unwrap_or("default"), &id, action).await.map_err(error)
}

#[tauri::command]
async fn reorder_downloads(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, ids: Vec<String>) -> Reply<()> {
    let manager = service.ensure(app.inner(), &handle).await?;
    app.store.reorder_downloads(profile_id.as_deref().unwrap_or("default"), &ids).map_err(error)?;
    manager.wake();
    Ok(())
}

#[tauri::command]
async fn remove_download(app: State<'_, Backend>, service: State<'_, DownloadServiceHandle>, handle: tauri::AppHandle,
    profile_id: Option<String>, id: String, delete_file: bool) -> Reply<cloudstream_core::downloads::RemovalReport> {
    service.ensure(app.inner(), &handle).await?.remove(profile_id.as_deref().unwrap_or("default"), &id, delete_file).await.map_err(error)
}

#[tauri::command]
async fn export_backup(app: State<'_, Backend>, sections: Option<Vec<String>>, profile_ids: Option<Vec<String>>) -> Reply<Option<Value>> {
    let stamp = {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let days = secs / 86_400;
        // Civil date from days since the epoch (no date crate needed for a file name).
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02}")
    };
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_title("Save a backup")
        .set_file_name(format!("cloudstream-backup-{stamp}.json"))
        .add_filter("Backup", &["json"])
        .save_file()
        .await
    else { return Ok(None) };
    let options = cloudstream_core::backup::ExportOptions { sections, profile_ids };
    let backup = app.export_backup_to(file.path(), &options).map_err(error)?;
    Ok(Some(json!({
        "path": file.path().display().to_string(),
        "sections": backup.sections,
        "profiles": backup.data.profiles.len(),
        "library": backup.data.library.len(),
        "history": backup.data.history.len(),
        "progress": backup.data.progress.len(),
    })))
}

#[tauri::command]
async fn choose_backup_file(pending: State<'_, PendingImportState>) -> Reply<Option<Value>> {
    let Some(file) = rfd::AsyncFileDialog::new().set_title("Choose a backup to import").add_filter("Backup", &["json", "txt"]).pick_file().await else { return Ok(None) };
    let path = file.path().to_owned();
    let size = std::fs::metadata(&path).map_err(error)?.len();
    if size as usize > cloudstream_core::backup::MAX_BACKUP_BYTES {
        return Err(format!("That file is larger than {} MiB, which this app does not accept", cloudstream_core::backup::MAX_BACKUP_BYTES / (1024 * 1024)));
    }
    let bytes = std::fs::read(&path).map_err(error)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "backup".into());
    static NEXT_IMPORT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = format!("import-{nanos:x}-{}", NEXT_IMPORT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    *pending.lock().await = Some(PendingImport { id: id.clone(), name: name.clone(), bytes });
    Ok(Some(json!({ "importId": id, "name": name, "size": size })))
}

async fn take_pending(pending: &PendingImportState, import_id: &str) -> Result<(String, Vec<u8>), String> {
    let guard = pending.lock().await;
    match guard.as_ref() {
        Some(p) if p.id == import_id => Ok((p.name.clone(), p.bytes.clone())),
        _ => Err("Choose the backup file again".into()),
    }
}

#[tauri::command]
async fn preview_import(app: State<'_, Backend>, pending: State<'_, PendingImportState>, import_id: String, options: cloudstream_core::backup::ImportOptions) -> Reply<cloudstream_core::backup::ImportPreview> {
    let (_, bytes) = take_pending(pending.inner(), &import_id).await?;
    app.preview_import(&bytes, &options).await.map_err(error)
}

#[tauri::command]
async fn apply_import(app: State<'_, Backend>, pending: State<'_, PendingImportState>, import_id: String, options: cloudstream_core::backup::ImportOptions, token: String) -> Reply<cloudstream_core::backup::ImportReport> {
    let (_, bytes) = take_pending(pending.inner(), &import_id).await?;
    let report = app.apply_import(&bytes, &options, &token).await.map_err(error)?;
    *pending.lock().await = None;
    Ok(report)
}

#[tauri::command]
async fn discard_import(pending: State<'_, PendingImportState>) -> Reply<()> {
    *pending.lock().await = None;
    Ok(())
}

#[tauri::command]
fn list_unresolved_imports(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<Vec<Value>> {
    let profile = profile_id.unwrap_or_else(|| "default".into());
    Ok(app.store.list::<Value>("unresolved_import").map_err(error)?.into_iter()
        .filter(|v| v.get("profileId").and_then(Value::as_str) == Some(profile.as_str()))
        .map(|v| json!({ "id": v["recordId"], "name": v["name"], "apiName": v["apiName"], "url": v["url"], "reason": v["reason"] }))
        .collect())
}

#[tauri::command]
async fn resolve_unresolved_imports(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<usize> {
    app.resolve_unresolved_imports(profile_id.as_deref().unwrap_or("default")).await.map_err(error)
}

#[tauri::command]
fn backup_status(app: State<'_, Backend>) -> Reply<cloudstream_core::backup_schedule::BackupStatus> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    app.backup_status_with_next(now).map_err(error)
}

#[tauri::command]
async fn run_backup_now(app: State<'_, Backend>) -> Reply<cloudstream_core::backup_schedule::BackupRun> {
    app.run_backup(&cloudstream_core::backup_schedule::SystemClock).await.map_err(error)
}

#[tauri::command]
async fn choose_folder(title: String) -> Reply<Option<String>> {
    Ok(rfd::AsyncFileDialog::new().set_title(title).pick_folder().await.map(|f| f.path().display().to_string()))
}

#[tauri::command]
fn take_entry_actions(app: State<'_, Backend>) -> Reply<Value> {
    let (actions, errors) = app.take_entries();
    Ok(json!({ "actions": actions, "errors": errors }))
}

#[tauri::command]
async fn play_local_file(app: State<'_, Backend>, path: String, profile_id: Option<String>) -> Reply<String> {
    app.play_local_file(&path, profile_id.as_deref()).await.map_err(error)
}

#[tauri::command]
fn list_external_players() -> Reply<Vec<cloudstream_core::external_player::DetectedPlayer>> {
    Ok(cloudstream_core::external_player::detect_players())
}

#[tauri::command]
fn open_in_external_player(app: State<'_, Backend>, stream: Stream, title: String, start: f64, subtitles: Vec<Subtitle>) -> Reply<Value> {
    use cloudstream_core::external_player::{launch, plan_launch, LaunchRequest};
    let settings = app.store.get_settings().map_err(error)?.global.external_player;
    let plan = plan_launch(&settings, &LaunchRequest { stream: &stream, title: &title, start_seconds: start, subtitles: &subtitles }).map_err(error)?;
    launch(&plan).map_err(error)?;
    Ok(json!({ "program": plan.program.display().to_string(), "warnings": plan.warnings }))
}

fn focus_main_window(handle: &tauri::AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
async fn diagnostics_summary(app: State<'_, Backend>) -> Reply<cloudstream_core::diagnostics::DiagnosticsReport> {
    app.diagnostics_report().await.map_err(error)
}

#[tauri::command]
async fn export_diagnostics(app: State<'_, Backend>) -> Reply<Option<String>> {
    let report = app.diagnostics_report().await.map_err(error)?;
    let bytes = report.to_json().map_err(error)?;
    let Some(file) = rfd::AsyncFileDialog::new().set_title("Save diagnostics").set_file_name("cloudstream-diagnostics.json").add_filter("Diagnostics", &["json"]).save_file().await else { return Ok(None) };
    std::fs::write(file.path(), bytes).map_err(error)?;
    Ok(Some(file.path().display().to_string()))
}

#[tauri::command]
fn clear_diagnostics(app: State<'_, Backend>) -> Reply<()> {
    app.diagnostics.clear();
    Ok(())
}

#[tauri::command]
async fn check_app_update(app: State<'_, Backend>) -> Reply<cloudstream_core::updates::UpdateCheck> {
    app.check_for_app_update().await.map_err(error)
}

/// Opens the release-notes page from the last signed feed in the system browser. Nothing else
/// can be opened this way.
#[tauri::command]
fn open_release_notes(app: State<'_, Backend>) -> Reply<()> {
    let url = app.last_release_notes.lock().map_err(error)?.clone().ok_or("Check for updates first")?;
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "Could not open a web browser. Copy the address from the update details instead.".to_string())
}

/// The only addresses the app opens in a browser. The interface sends an id, never an address.
const LINKS: [(&str, &str); 6] = [
    ("project", "https://github.com/happinessisreal/cloudstream-desktop"),
    ("issues", "https://github.com/happinessisreal/cloudstream-desktop/issues"),
    ("upstream", "https://github.com/recloudstream/cloudstream"),
    ("docs", "https://recloudstream.github.io/csdocs/"),
    ("community", "https://discord.gg/5Hus6fM"),
    ("extension-policy", "https://github.com/recloudstream"),
];

#[tauri::command]
fn list_links() -> Reply<Vec<Value>> {
    Ok(LINKS.iter().map(|(id, url)| json!({ "id": id, "url": url })).collect())
}

#[tauri::command]
fn open_link(id: String) -> Reply<()> {
    let (_, url) = LINKS.iter().find(|(known, _)| *known == id).ok_or("Unknown link")?;
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| format!("Could not open a web browser. The address is {url}"))
}

/// Licence texts compiled into the app, so they are always available offline and cannot drift
/// from the build.
#[tauri::command]
fn get_legal_text(name: String) -> Reply<String> {
    match name.as_str() {
        "license" => Ok(include_str!("../../LICENSE").to_string()),
        "notices" => Ok(include_str!("../../NOTICE.md").to_string()),
        "third-party" => Ok(include_str!("../../THIRD_PARTY_LICENSES.md").to_string()),
        _ => Err("Unknown document".into()),
    }
}

/// Quit after the user confirmed in the interface. Downloads stop cleanly and resume next start.
#[tauri::command]
fn quit_app(handle: tauri::AppHandle, confirmed: State<'_, Arc<ExitConfirmed>>) -> Reply<()> {
    confirmed.0.store(true, std::sync::atomic::Ordering::SeqCst);
    handle.exit(0);
    Ok(())
}

/// A profile the request may act for: an existing profile, or the built-in default.
fn known_profile(app: &Backend, profile_id: Option<String>) -> Result<String, String> {
    let id = profile_id.unwrap_or_else(|| "default".into());
    if id == "default" || app.store.get_profile(&id).map_err(error)?.is_some() { Ok(id) } else { Err("That profile no longer exists".into()) }
}

#[tauri::command]
async fn subtitle_services(app: State<'_, Backend>, profile_id: Option<String>) -> Reply<cloudstream_core::subtitle_online::ServicesView> {
    let profile = known_profile(&app, profile_id)?;
    app.subtitle_services(&profile).await.map_err(error)
}

#[tauri::command]
async fn connect_subtitle_service(app: State<'_, Backend>, profile_id: Option<String>, provider: String, fields: std::collections::BTreeMap<String, String>) -> Reply<cloudstream_core::subtitle_online::ServiceView> {
    let profile = known_profile(&app, profile_id)?;
    app.connect_subtitle_service(&profile, &provider, fields).await.map_err(error)
}

#[tauri::command]
async fn disconnect_subtitle_service(app: State<'_, Backend>, profile_id: Option<String>, provider: String) -> Reply<()> {
    let profile = known_profile(&app, profile_id)?;
    app.disconnect_subtitle_service(&profile, &provider).await.map_err(error)
}

#[tauri::command]
async fn search_online_subtitles(app: State<'_, Backend>, session_id: String, options: Option<cloudstream_core::subtitle_online::SearchOptions>) -> Reply<cloudstream_core::subtitle_online::SearchView> {
    app.search_online_subtitles(&session_id, options.unwrap_or_default()).await.map_err(error)
}

#[tauri::command]
async fn use_online_subtitle(app: State<'_, Backend>, search_id: String, index: usize) -> Reply<cloudstream_core::SubtitleFileSelection> {
    app.use_online_subtitle(&search_id, index).await.map_err(error)
}

#[tauri::command]
async fn metadata_for(app: State<'_, Backend>, details: MediaDetails, locale: String) -> Reply<cloudstream_core::MetadataResult> {
    let media_id = compute_media_id(&details.item.provider_id, &details.item.url, details.item.unique_url.as_deref());
    app.metadata_for(&media_id, &details, &locale).await.map_err(error)
}

#[tauri::command]
async fn metadata_search(app: State<'_, Backend>, title: String, year: Option<i32>) -> Reply<Value> {
    let (candidates, problems) = app.metadata_search(&title, year).await.map_err(error)?;
    Ok(json!({ "candidates": candidates, "problems": problems }))
}

#[tauri::command]
fn set_metadata_link(app: State<'_, Backend>, item: MediaItem, service: String, external_id: String, label: Option<String>, year: Option<i32>) -> Reply<cloudstream_core::metadata::MetadataLink> {
    let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
    app.set_metadata_link(&media_id, &service, &external_id, label, year).map_err(error)
}

#[tauri::command]
fn remove_metadata_link(app: State<'_, Backend>, item: MediaItem, service: String, sync_data: std::collections::BTreeMap<String, String>) -> Reply<()> {
    let media_id = compute_media_id(&item.provider_id, &item.url, item.unique_url.as_deref());
    app.remove_metadata_link(&media_id, &service, &sync_data).map_err(error)
}

#[tauri::command]
async fn player_capabilities(app: State<'_, Backend>) -> Reply<PlayerCapabilities> {
    Ok(app.player.capabilities().await)
}

#[tauri::command]
async fn rank_streams(app: State<'_, Backend>, streams: Vec<Stream>, profile_id: Option<String>, session_id: Option<String>) -> Reply<Vec<RankedStream>> {
    Ok(app.rank_streams_for_session(&streams, profile_id.as_deref(), session_id.as_deref()).await)
}

#[tauri::command]
async fn switch_stream(app: State<'_, Backend>, stream: Stream, subtitles: Option<Vec<Subtitle>>) -> Reply<String> {
    app.switch_stream(stream, subtitles).await.map_err(error)
}

#[tauri::command]
async fn player_action(app: State<'_, Backend>, action: String) -> Reply<Value> {
    let command = match action.as_str() {
        "play" => json!(["set_property", "pause", false]),
        "pause" => json!(["set_property", "pause", true]),
        "toggle_pause" => json!(["cycle", "pause"]),
        "forward" => json!(["seek", 10]),
        "back" => json!(["seek", -10]),
        "fullscreen" => json!(["cycle", "fullscreen"]),
        "next" => {
            let res = app.advance_queue_and_play(true).await.map_err(error)?;
            return Ok(json!({ "advanced": res }));
        }
        "prev" | "previous" => {
            let res = app.advance_queue_and_play(false).await.map_err(error)?;
            return Ok(json!({ "advanced": res }));
        }
        "stop" => {
            app.stop_playback().await;
            return Ok(Value::Null);
        }
        _ => return Err("Unknown player action".into()),
    };
    app.player.command(command).await.map_err(error)
}

fn main() {
    use cloudstream_core::instance::{absolutize_arguments, acquire, forward, runtime_directory, Role};
    // Links and files passed on the command line (a clicked link, "open with") reach the app that
    // is already running instead of starting a second one.
    let cwd = std::env::current_dir().unwrap_or_default();
    let arguments = absolutize_arguments(&std::env::args().skip(1).collect::<Vec<_>>(), &cwd);
    let primary = match runtime_directory().and_then(|dir| acquire(&dir).map(|role| (dir, role))) {
        Ok((dir, Role::Secondary)) => match forward(&dir, &arguments) {
            Ok(()) => std::process::exit(0),
            Err(message) => {
                eprintln!("Cloudstream Desktop is already running but did not respond: {message:#}");
                std::process::exit(1);
            }
        },
        Ok((_, Role::Primary(primary))) => Some(primary),
        Err(message) => {
            eprintln!("Opening links in the running app is unavailable: {message:#}");
            None
        }
    };
    tauri::Builder::default()
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let handle = window.app_handle();
                if handle.state::<Arc<ExitConfirmed>>().0.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let backend = handle.state::<Backend>();
                let playing = tauri::async_runtime::block_on(backend.player.state()).active;
                let downloads = backend.store.download_jobs(None).map(|jobs| jobs.iter().filter(|job| job.state.active()).count()).unwrap_or(0);
                if playing || downloads > 0 {
                    // Ask first. The interface answers with `quit_app` or leaves the window open.
                    api.prevent_close();
                    let _ = handle.emit("confirm_exit", json!({ "playing": playing, "downloads": downloads }));
                }
            }
        })
        .setup(move |tauri_app| {
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
            let downloads = Arc::new(DownloadService { manager: tokio::sync::Mutex::new(None) });
            tauri_app.manage(downloads.clone());
            tauri_app.manage(PendingImportState::default());
            tauri_app.manage(Arc::new(ExitConfirmed::default()));
            // Cold start: links given on the command line wait until the interface is ready.
            backend.submit_entry_arguments(&arguments);
            if let Some(primary) = primary {
                let entry_backend = backend.clone();
                let entry_handle = tauri_app.handle().clone();
                primary.serve(move |args| {
                    entry_backend.submit_entry_arguments(&args);
                    let _ = entry_handle.emit("entry_pending", ());
                    focus_main_window(&entry_handle);
                });
            }
            // Media keys and desktop widgets (MPRIS). Missing session bus just means no media keys.
            let mpris_backend = backend.clone();
            let mpris_handle = tauri_app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use cloudstream_core::mpris::{MprisRequest, MprisService, MprisState};
                let (tx, mut requests) = tokio::sync::mpsc::unbounded_channel();
                let service = match MprisService::start(tx).await {
                    Ok(service) => service,
                    Err(message) => {
                        eprintln!("Media keys are unavailable: {message:#}");
                        return;
                    }
                };
                let mut events = mpris_backend.player.subscribe();
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
                let mut dirty = true;
                loop {
                    tokio::select! {
                        request = requests.recv() => match request {
                            Some(MprisRequest::Raise) => focus_main_window(&mpris_handle),
                            Some(request) => {
                                let opens = matches!(request, MprisRequest::OpenUri(_));
                                // A control with nothing to act on (no video) is simply ignored.
                                let _ = mpris_backend.handle_mpris(request).await;
                                if opens { let _ = mpris_handle.emit("entry_pending", ()); }
                            }
                            None => break,
                        },
                        event = events.recv() => match event {
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => dirty = true,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        },
                        _ = tick.tick() => if dirty {
                            dirty = false;
                            let state = mpris_backend.player.state().await;
                            let _ = service.update(MprisState::from_player(&state)).await;
                        },
                    }
                }
            });
            // Scheduled backups run only while the app is open, with a catch-up shortly after start.
            let scheduler_backend = backend.clone();
            tauri::async_runtime::spawn(async move {
                let _handle = cloudstream_core::App::start_backup_scheduler(&scheduler_backend);
                std::future::pending::<()>().await;
            });
            let download_backend = backend.clone();
            let download_handle = tauri_app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(message) = downloads.ensure(&download_backend, &download_handle).await {
                    let _ = download_handle.emit("download_event", cloudstream_core::download_queue::DownloadEvent::QueueError { message });
                }
            });
            let player_backend = backend.clone();
            let app_handle = tauri_app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = player_backend.player.subscribe();
                while let Ok(event) = rx.recv().await {
                    let _ = app_handle.emit("player_event", &event);
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
            install_torrentio,
            get_custom_trackers,
            save_custom_trackers,
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
            check_subscriptions,
            acknowledge_subscription_updates,
            deliver_subscription_alerts,
            remove_library_item,
            clear_library,
            remove_history_item,
            clear_history,
            list_profiles,
            get_profile,
            create_profile,
            update_profile,
            delete_profile,
            verify_profile_pin,
            switch_profile,
            get_media_watch_progress,
            mark_episode_watched,
            mark_episodes_up_to,
            play,
            get_playback_queue,
            set_playback_queue,
            get_webview_status,
            get_player_state,
            player_action,
            player_command,
            player_skip_markers,
            player_skip_interval,
            begin_speed_hold,
            end_speed_hold,
            choose_local_subtitle,
            choose_subtitle_font,
            subtitle_font_preview,
            select_archived_subtitle,
            prepare_provider_subtitle_archive,
            cancel_archived_subtitle,
            player_capabilities,
            download_jobs,
            download_watch_progress,
            download_subtitle_warnings,
            download_health,
            repair_download,
            play_download,
            enqueue_download,
            enqueue_downloads,
            control_download,
            reorder_downloads,
            remove_download,
            export_backup,
            backup_status,
            metadata_for,
            metadata_search,
            subtitle_services,
            connect_subtitle_service,
            disconnect_subtitle_service,
            search_online_subtitles,
            use_online_subtitle,
            set_metadata_link,
            remove_metadata_link,
            list_links,
            open_link,
            get_legal_text,
            quit_app,
            diagnostics_summary,
            export_diagnostics,
            clear_diagnostics,
            check_app_update,
            open_release_notes,
            take_entry_actions,
            play_local_file,
            list_external_players,
            open_in_external_player,
            run_backup_now,
            choose_folder,
            choose_backup_file,
            preview_import,
            apply_import,
            discard_import,
            list_unresolved_imports,
            resolve_unresolved_imports,
            rank_streams,
            switch_stream,
            get_settings,
            update_settings
        ])
        .build(tauri::generate_context!())
        .expect("Could not start Cloudstream Desktop")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                let app = handle.state::<Backend>().inner().clone();
                let downloads = handle.state::<DownloadServiceHandle>().inner().clone();
                tauri::async_runtime::block_on(async move {
                    downloads.shutdown().await;
                    let _ = app.save_progress().await;
                    app.stop_playback().await;
                    app.worker.stop().await;
                });
            }
        });
}
