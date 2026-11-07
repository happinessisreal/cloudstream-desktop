//! Bounded queue runner for direct files and HLS folders.
use crate::{download_queue::{DownloadAction, DownloadEvent, DownloadJob, DownloadRequest, DownloadState, QueueLease}, downloads::{self, TransferProgress, TransferResult}, hls_download::{self, ExternalSubtitle, HlsFailure, HlsRequest, HlsResult}, models::{ResolvedLinks, Stream}, streams, App};
use anyhow::{bail, Context, Result};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Weak}, time::{Duration, Instant}};
use tokio::sync::{broadcast, watch, Mutex, Notify};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchSummary { pub added: usize, pub already_queued: usize, pub failed: Vec<String> }

pub struct DownloadManager {
    app: Weak<App>,
    lease: QueueLease,
    max_active: usize,
    controls: Mutex<HashMap<String, watch::Sender<bool>>>,
    wake: Notify,
    stopped: watch::Sender<bool>,
    shutdown: watch::Sender<bool>,
    runner: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    events: broadcast::Sender<DownloadEvent>,
}

/// Check owned output without changing or deleting it. HLS assets are hashed; a direct file is
/// checked against its recorded inode and size.
pub fn download_is_healthy(job: &DownloadJob) -> Result<bool> {
    if job.state != DownloadState::Completed { return Ok(false); }
    if job.extension == downloads::HLS_EXTENSION {
        Ok(hls_download::check_folder(&job.destination, job.destination_identity, &job.id, true)?.healthy())
    } else {
        let valid = downloads::verify_completed_file(&job.destination, job.destination_identity, &job.id, &job.extension, job.bytes)?;
        Ok(valid && crate::direct_subtitles::read(&job.destination, job.destination_identity, &job.id, &job.extension).is_ok())
    }
}

impl DownloadManager {
    /// Only one app instance can own a queue; interrupted jobs are recovered before scheduling.
    pub fn start(app: &Arc<App>, max_active: usize) -> Result<Arc<Self>> {
        if !(1..=4).contains(&max_active) { bail!("Download concurrency must be between 1 and 4"); }
        tokio::runtime::Handle::try_current().context("Download queue requires an async runtime")?;
        let lease = QueueLease::acquire(&app.data)?;
        app.store.recover_download_queue(&lease)?;
        let (shutdown, _) = watch::channel(false);
        let (events, _) = broadcast::channel(128);
        let (stopped, _) = watch::channel(false);
        let manager = Arc::new(Self { app: Arc::downgrade(app), lease, max_active, controls: Mutex::new(HashMap::new()), wake: Notify::new(), stopped, shutdown, runner: std::sync::Mutex::new(None), events });
        let task = manager.clone();
        let runner = tokio::spawn(async move { task.run().await; });
        *manager.runner.lock().map_err(|_| anyhow::anyhow!("Download queue lock failed"))? = Some(runner);
        Ok(manager)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> { self.events.subscribe() }
    fn emit(&self, job: DownloadJob) { let _ = self.events.send(DownloadEvent::JobChanged { job }); }

    pub fn enqueue(&self, profile_id: &str, request: DownloadRequest) -> Result<DownloadJob> {
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let app = self.app.upgrade().context("The application is closing")?;
        let job = app.store.enqueue_download(profile_id, request)?;
        self.emit(job.clone()); self.wake.notify_one(); Ok(job)
    }

    /// Enqueue several episodes at once. Episodes already queued, running or finished for this
    /// profile are reported as `already_queued`, not added again; one failure does not stop the
    /// rest, so a partial batch is always explained.
    pub fn enqueue_many(&self, profile_id: &str, requests: Vec<DownloadRequest>) -> Result<BatchSummary> {
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let app = self.app.upgrade().context("The application is closing")?;
        let mut summary = BatchSummary::default();
        for request in requests {
            let name = request.episode.name.clone();
            let known: HashSet<String> = app.store.download_jobs(Some(profile_id))?.into_iter().map(|job| job.id).collect();
            match self.enqueue(profile_id, request) {
                Ok(job) if known.contains(&job.id) => summary.already_queued += 1,
                Ok(_) => summary.added += 1,
                Err(error) => summary.failed.push(format!("{name}: {error}")),
            }
        }
        Ok(summary)
    }

    pub async fn control(&self, profile_id: &str, id: &str, action: DownloadAction) -> Result<DownloadJob> {
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let app = self.app.upgrade().context("The application is closing")?;
        let controls = self.controls.lock().await;
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        if matches!(action, DownloadAction::Resume | DownloadAction::Retry) && controls.contains_key(id) {
            bail!("The download is still stopping; try again shortly");
        }
        let job = app.store.control_download(id, profile_id, action)?;
        if matches!(action, DownloadAction::Pause | DownloadAction::Cancel) {
            if let Some(stop) = controls.get(id) { let _ = stop.send(true); }
        }
        drop(controls);
        self.emit(job.clone()); self.wake.notify_one(); Ok(job)
    }

    /// Repair by queuing a new copy. The old folder and any user-edited files stay untouched.
    pub async fn repair(&self, profile_id: &str, id: &str) -> Result<DownloadJob> {
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let app = self.app.upgrade().context("The application is closing")?;
        let controls = self.controls.lock().await;
        let job = app.store.get_download_job(id)?.filter(|job| job.profile_id == profile_id).context("Download job not found")?;
        if job.state != DownloadState::Completed { bail!("Only finished downloads can be repaired"); }
        if controls.contains_key(id) { bail!("The download is still stopping; try again shortly"); }
        let checked = job.clone();
        if tokio::task::spawn_blocking(move || download_is_healthy(&checked)).await.context("Could not check the download")?? {
            bail!("The downloaded files are already present and unchanged");
        }
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let repair = app.store.enqueue_download_repair(id, profile_id)?;
        drop(controls);
        self.emit(repair.clone()); self.wake.notify_one();
        Ok(repair)
    }

    /// Remove a job and its temporary files, optionally deleting the finished file it published.
    /// Holds the controls lock throughout, so the runner cannot claim the job while it is removed.
    pub async fn remove(&self, profile_id: &str, id: &str, delete_file: bool) -> Result<crate::downloads::RemovalReport> {
        if *self.shutdown.borrow() { bail!("The download queue is stopping"); }
        let app = self.app.upgrade().context("The application is closing")?;
        let controls = self.controls.lock().await;
        let job = app.store.get_download_job(id)?.filter(|job| job.profile_id == profile_id).context("Download job not found")?;
        if job.state.active() { bail!("Pause or cancel the download before removing it"); }
        if controls.contains_key(id) { bail!("The download is still stopping; try again shortly"); }
        let (destination, identity, extension, job_id) = (job.destination.clone(), job.destination_identity, job.extension.clone(), job.id.clone());
        let delete = delete_file && job.state == DownloadState::Completed;
        let mut report = tokio::task::spawn_blocking(move || if extension == downloads::HLS_EXTENSION {
            hls_download::remove_folder(&destination, identity, &job_id, delete)
        } else { downloads::remove_job_assets(&destination, identity, &job_id, &extension, delete) })
            .await.context("Download cleanup was interrupted")?
            .unwrap_or_else(|error| {
                // A missing or replaced folder cannot be cleaned safely; the record can still go.
                crate::downloads::RemovalReport { file_deleted: false, temp_cleaned: false, notes: vec![format!("Temporary files could not be cleaned: {error}")] }
            });
        if delete_file && job.state != DownloadState::Completed { report.notes.push("Only finished downloads have a file to delete.".into()); }
        app.store.delete_download_job(id, profile_id)?;
        drop(controls);
        self.wake.notify_one();
        Ok(report)
    }

    pub fn wake(&self) { self.wake.notify_one(); }

    /// Finish outstanding cleanup before releasing the queue lease. Interrupted active jobs
    /// remain queued for the next start; explicit pause/cancel remains unchanged.
    pub async fn shutdown(&self) {
        let mut stopped = self.stopped.subscribe();
        self.shutdown.send_replace(true);
        for stop in self.controls.lock().await.values() { let _ = stop.send(true); }
        self.wake.notify_one();
        let runner = self.runner.lock().ok().and_then(|mut guard| guard.take());
        if let Some(runner) = runner { let _ = runner.await; }
        else { while !*stopped.borrow() { if stopped.changed().await.is_err() { break; } } }
    }

    async fn run(self: Arc<Self>) {
        loop {
            if *self.shutdown.borrow() { break; }
            let Some(app) = self.app.upgrade() else { break };
            {
                let mut controls = self.controls.lock().await;
                while controls.len() < self.max_active && !*self.shutdown.borrow() {
                    match app.store.claim_next_download(&self.lease, self.max_active) {
                        Ok(Some(job)) => {
                            let (stop, receiver) = watch::channel(false);
                            controls.insert(job.id.clone(), stop.clone());
                            self.emit(job.clone());
                            let manager = self.clone(); let app = app.clone();
                            tokio::spawn(async move {
                                run_job(&app, &manager, job.clone(), receiver, stop).await;
                                manager.controls.lock().await.remove(&job.id);
                                manager.wake.notify_one();
                            });
                        }
                        Ok(None) => break,
                        Err(_) => { let _ = self.events.send(DownloadEvent::QueueError { message: "Could not read the download queue".into() }); break; }
                    }
                }
            }
            drop(app);
            let mut shutdown = self.shutdown.subscribe();
            if *shutdown.borrow() { break; }
            tokio::select! { _ = self.wake.notified() => {}, _ = shutdown.changed() => {} }
        }
        loop {
            let notified = self.wake.notified();
            if self.controls.lock().await.is_empty() { break; }
            notified.await;
        }
        self.stopped.send_replace(true);
    }
}

/// Turn a storage failure into advice the user can act on. Only these fixed strings are saved:
/// raw OS errors can include paths and provider errors can include signed URLs.
pub(crate) fn disk_error_message(error: &anyhow::Error) -> Option<&'static str> {
    let code = error.chain().find_map(|cause| cause.downcast_ref::<std::io::Error>()?.raw_os_error())?;
    match code {
        libc::ENOSPC => Some("The destination is full. Free some space, then retry."),
        libc::EDQUOT => Some("Your storage quota for the destination is used up. Free some space, then retry."),
        libc::EACCES | libc::EPERM => Some("The app is not allowed to write to the destination folder. Choose another folder and retry."),
        libc::EROFS => Some("The destination folder is read-only. Choose another folder and retry."),
        libc::ENAMETOOLONG => Some("The destination path is too long. Choose a folder with a shorter path and retry."),
        libc::EFBIG => Some("The destination file system cannot store a file this large. Choose another folder and retry."),
        libc::EIO => Some("The destination reported a read/write error. Check the disk, then retry."),
        _ => None,
    }
}

fn eligible_direct(stream: &Stream) -> bool {
    let kind = stream.kind.as_deref().unwrap_or("").to_ascii_lowercase();
    let ty = stream.stream_type.to_ascii_lowercase();
    let unsupported = ["hls", "m3u8", "dash", "torrent", "magnet", "playlist", "drm", "live"];
    if unsupported.iter().any(|item| kind.contains(item) || ty.contains(item)) || stream.drm.is_some() || streams::is_playlist(stream) || !stream.audio_tracks.is_empty() { return false; }
    let Ok(url) = crate::repository::network_url(&stream.url) else { return false };
    let path = url.path().to_ascii_lowercase();
    !path.ends_with(".m3u8") && !path.ends_with(".mpd")
}

fn eligible_hls(stream: &Stream) -> bool {
    let kind = stream.kind.as_deref().unwrap_or("").to_ascii_lowercase();
    let ty = stream.stream_type.to_ascii_lowercase();
    if ["dash", "torrent", "magnet", "drm", "live"].iter().any(|item| kind.contains(item) || ty.contains(item))
        || stream.drm.is_some() || streams::is_playlist(stream) || !stream.audio_tracks.is_empty() { return false; }
    let Ok(url) = crate::repository::network_url(&stream.url) else { return false };
    kind.contains("hls") || kind.contains("m3u8") || ty.contains("hls") || ty.contains("m3u8") || url.path().to_ascii_lowercase().ends_with(".m3u8")
}

fn choose_stream(app: &App, job: &DownloadJob, links: &ResolvedLinks) -> Result<Stream> {
    let profile = app.source_profile(&job.profile_id);
    let ranked = streams::rank_streams(&links.streams, &profile, &HashSet::new());
    for ranked in ranked.into_iter().filter(|entry| entry.usable) {
        let stream = &links.streams[ranked.index];
        if !(if job.extension == downloads::HLS_EXTENSION { eligible_hls(stream) } else { eligible_direct(stream) }) { continue; }
        if job.policy.source_name.as_deref().is_some_and(|wanted| !streams::source_name(stream).eq_ignore_ascii_case(wanted)) { continue; }
        if job.policy.stream_name.as_deref().is_some_and(|wanted| stream.name != wanted) { continue; }
        if job.policy.quality.is_some_and(|wanted| stream.quality != wanted) { continue; }
        if stream.resource_handle.is_none() && (!stream.headers.is_empty() || !stream.referer.is_empty()) { continue; }
        return Ok(stream.clone());
    }
    if job.extension == downloads::HLS_EXTENSION { bail!("No supported HLS source matches this download") }
    bail!("No supported direct source matches this download")
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    loop { if *stop.borrow() { return; } if stop.changed().await.is_err() { std::future::pending::<()>().await; } }
}

async fn run_job(app: &Arc<App>, manager: &Arc<DownloadManager>, job: DownloadJob, mut stop: watch::Receiver<bool>, stop_tx: watch::Sender<bool>) {
    let mut current = job;
    let mut session_tokens: Vec<String> = Vec::new();
    let result: Result<Option<(std::path::PathBuf, u64, Option<u64>)>> = async {
        if !app.store.provider_is_enabled(&current.provider_id)? { bail!("The source is disabled"); }
        let payload = app.store.get_episode_payload(&current.episode_id)?.context("The episode is no longer available")?;
        let request_id = format!("download-{}", uuid::Uuid::new_v4());
        let response = tokio::select! {
            _ = stopped(&mut stop) => { let _ = app.cancel_request(&request_id).await; return Ok(None); },
            response = tokio::time::timeout(Duration::from_secs(30), app.call_cancellable(Some(&request_id), "loadLinks", serde_json::json!({"providerId":current.provider_id,"data":payload})) ) => {
                match response { Ok(result) => result.map_err(|_| anyhow::anyhow!("Could not resolve a download source"))?, Err(_) => { let _ = app.cancel_request(&request_id).await; bail!("The source took too long to respond"); } }
            },
        };
        let links: ResolvedLinks = serde_json::from_value(response).map_err(|_| anyhow::anyhow!("The source returned invalid links"))?;
        let stream = choose_stream(app, &current, &links)?;
        let resource_identity = if current.extension == downloads::HLS_EXTENSION {
            crate::repository::network_url(&stream.url)?.path().to_owned()
        } else { stream.url.clone() };
        let source_identity = format!("{}\n{}\n{}\n{}\n{}\n{}", current.provider_id, current.episode_id, streams::source_name(&stream), stream.name, stream.quality, resource_identity);
        let url = if let Some(handle) = &stream.resource_handle {
            // Await session creation even after a stop request so a token can always be closed.
            let installed = app.store.installed()?;
            let session = app.worker.create_media_session(&installed, handle).await.map_err(|_| anyhow::anyhow!("Could not start the provider download session"))?;
            session_tokens.push(session.session_token);
            session.transport_url
        } else { stream.url.clone() };
        let video_is_proxied = !session_tokens.is_empty();
        let mut subtitles = Vec::new();
        let mut skipped_subtitles = 0usize;
        {
            for subtitle in &links.subtitles {
                if subtitles.len() >= 16 || crate::subtitles::is_archive_subtitle(subtitle) { skipped_subtitles += 1; continue; }
                if *stop.borrow() { return Ok(None); }
                let (url, no_proxy) = if let Some(handle) = &subtitle.resource_handle {
                    let installed = app.store.installed()?;
                    let session = app.worker.create_media_session(&installed, handle).await.map_err(|_| anyhow::anyhow!("Could not start a subtitle download session"))?;
                    session_tokens.push(session.session_token);
                    (session.transport_url, true)
                } else if subtitle.headers.is_empty() {
                    (subtitle.url.clone(), false)
                } else { skipped_subtitles += 1; continue };
                subtitles.push(ExternalSubtitle { language: subtitle.lang_tag.clone().unwrap_or_else(|| subtitle.lang.clone()), name: subtitle.lang.clone(), url, no_proxy });
            }
        }
        if *stop.borrow() { return Ok(None); }
        current = app.store.transition_download(&current.id, &current.profile_id, current.revision, DownloadState::Downloading, None, None, current.bytes, current.total)?;
        manager.emit(current.clone());
        let mut last_progress = TransferProgress { bytes: current.bytes, total: current.total };
        let mut last_saved = Instant::now();
        let mut progress_failed = false;
        let destination = current.destination.clone();
        let destination_identity = current.destination_identity;
        let id = current.id.clone();
        let extension = current.extension.clone();
        let preferred = app.source_profile(&current.profile_id).preferred_language.into_iter().collect::<Vec<_>>();
        let target_height = current.policy.quality.and_then(|quality| u32::try_from(quality).ok()).filter(|quality| *quality > 0 && *quality != 400);
        let mut record_progress = |progress: TransferProgress| {
                last_progress = progress;
                if last_saved.elapsed() >= Duration::from_millis(300) {
                    match app.store.update_download_progress(&current.id, &current.profile_id, current.revision, last_progress.bytes, last_progress.total) {
                        Ok(updated) => { current = updated; manager.emit(current.clone()); last_saved = Instant::now(); },
                        Err(_) => { progress_failed = true; let _ = stop_tx.send(true); }
                    }
                }
            };
        let result = if extension == downloads::HLS_EXTENSION {
            let policy = crate::hls::SelectionPolicy { target_height, audio_languages: preferred.clone(), subtitle_languages: preferred };
            let request = HlsRequest { root: &destination, identity: destination_identity, job_id: &id, source_identity: &source_identity, url: &url, no_proxy: video_is_proxied, policy, subtitles: subtitles.clone(), skipped_subtitles };
            hls_download::download_hls(request, stop.clone(), |progress| record_progress(TransferProgress { bytes: progress.bytes, total: progress.total })).await.map(|result| match result {
                HlsResult::Completed { path, bytes } => TransferResult::Completed { path, bytes },
                HlsResult::Paused { bytes } => TransferResult::Paused { bytes },
            })
        } else {
            downloads::transfer_with_retry(&destination, destination_identity, &id, &extension, &source_identity, &url,
                video_is_proxied, stop.clone(), 3, &mut record_progress).await
        };
        current.bytes = last_progress.bytes;
        current.total = last_progress.total;
        if progress_failed { bail!("Download state changed or progress could not be saved"); }
        match result? {
            TransferResult::Completed { path, bytes } => {
                if extension != downloads::HLS_EXTENSION && !crate::direct_subtitles::save(&destination, destination_identity, &id, &extension, &subtitles, skipped_subtitles, &stop).await? {
                    current.bytes = bytes; current.total = last_progress.total; return Ok(None);
                }
                Ok(Some((path, bytes, if extension == downloads::HLS_EXTENSION { Some(bytes) } else { last_progress.total })))
            },
            TransferResult::Paused { bytes } => { current.bytes = bytes; current.total = last_progress.total; Ok(None) },
        }
    }.await;
    if !session_tokens.is_empty() {
        if let Ok(installed) = app.store.installed() {
            for token in &session_tokens { let _ = app.worker.close_media_session(&installed, token).await; }
        }
    }
    let transition = match result {
        Ok(Some((path, bytes, total))) => app.store.transition_download(&current.id, &current.profile_id, current.revision, DownloadState::Completed, None, Some(&path), bytes, total),
        Ok(None) => app.store.transition_download(&current.id, &current.profile_id, current.revision, DownloadState::Queued, None, None, current.bytes, current.total),
        Err(error) => {
            // Provider and OS errors can contain signed URLs; only known generic messages are persisted.
            app.diagnostics.record("download", &format!("{error:#}"));
            let message = if let Some(failure) = error.downcast_ref::<HlsFailure>() { failure.to_string() }
                else if error.to_string().contains("disabled") { "The source is disabled".into() }
                else if error.to_string().contains("supported HLS") { "No supported HLS source matches this download".into() }
                else if error.to_string().contains("supported direct") { "No supported direct source matches this download".into() }
                else if let Some(disk) = disk_error_message(&error) { disk.into() }
                else { "Download failed. Check the source, connection and destination, then retry.".into() };
            app.store.transition_download(&current.id, &current.profile_id, current.revision, DownloadState::Failed, Some(&message), None, current.bytes, current.total)
        }
    };
    match transition {
        Ok(updated) => manager.emit(updated),
        Err(_) if *stop.borrow() => {
            // The user's durable pause/cancel wins; update only its byte count, never its state.
            if let Ok(Some(stopped_job)) = app.store.get_download_job(&current.id) {
                if let Ok(updated) = app.store.update_stopped_download_progress(&current.id, &current.profile_id, stopped_job.revision, current.bytes, current.total) { manager.emit(updated); }
            }
        },
        Err(_) => { let _ = manager.events.send(DownloadEvent::QueueError { message: "Could not save a download state change".into() }); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_failures_get_specific_safe_messages() {
        let wrap = |code: i32| anyhow::Error::from(std::io::Error::from_raw_os_error(code)).context("Could not write https://signed.example/v?token=secret");
        assert!(disk_error_message(&wrap(libc::ENOSPC)).unwrap().contains("full"));
        assert!(disk_error_message(&wrap(libc::EROFS)).unwrap().contains("read-only"));
        assert!(disk_error_message(&wrap(libc::EACCES)).unwrap().contains("not allowed"));
        assert!(disk_error_message(&wrap(libc::EDQUOT)).unwrap().contains("quota"));
        for code in [libc::ENOSPC, libc::EROFS, libc::EACCES, libc::EDQUOT, libc::EIO, libc::EFBIG, libc::ENAMETOOLONG] {
            let message = disk_error_message(&wrap(code)).unwrap();
            assert!(!message.contains("token") && !message.contains("signed.example"), "{message}");
        }
        assert_eq!(disk_error_message(&wrap(libc::ECONNRESET)), None, "network errors keep the generic message");
        assert_eq!(disk_error_message(&anyhow::anyhow!("The source took too long to respond")), None);
    }
    #[test]
    fn excludes_playlists_drm_separate_audio_and_adaptive_formats() {
        let direct = Stream { name: "File".into(), url: "https://example.test/file.mp4".into(), kind: Some("direct".into()), ..Default::default() };
        assert!(eligible_direct(&direct));
        for kind in ["hls", "dash", "torrent", "magnet", "playlist", "drm"] {
            assert!(!eligible_direct(&Stream { kind: Some(kind.into()), ..direct.clone() }));
        }
        assert!(!eligible_direct(&Stream { url: "https://example.test/master.m3u8?token=x".into(), ..direct.clone() }));
        assert!(!eligible_direct(&Stream { audio_tracks: vec![crate::models::AudioTrack { url: "https://example.test/audio".into(), headers: Default::default(), resource_handle: None, extra: Default::default() }], ..direct.clone() }));
        assert!(!eligible_direct(&Stream { url: "file:///etc/passwd".into(), ..direct }));
    }

    #[test]
    fn hls_selection_rejects_unsupported_and_unsafe_sources() {
        let hls = Stream { name: "HLS".into(), url: "https://example.test/master.m3u8?token=x".into(), ..Default::default() };
        assert!(eligible_hls(&hls));
        assert!(!eligible_direct(&hls));
        assert!(eligible_hls(&Stream { url: "https://example.test/stream".into(), stream_type: "M3U8".into(), ..hls.clone() }));
        assert!(!eligible_hls(&Stream { url: "file:///etc/passwd".into(), ..hls.clone() }));
        assert!(!eligible_hls(&Stream { kind: Some("live".into()), ..hls.clone() }));
        assert!(!eligible_hls(&Stream { stream_type: "DASH".into(), ..hls.clone() }));
        assert!(!eligible_hls(&Stream { audio_tracks: vec![crate::models::AudioTrack { url: "https://example.test/audio".into(), headers: Default::default(), resource_handle: None, extra: Default::default() }], ..hls }));
    }
}
