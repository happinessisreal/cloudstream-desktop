mod support;
use cloudstream_core::{direct_subtitles, downloads::{self, DestinationIdentity, TransferResult}, hls_download::ExternalSubtitle};
use std::path::Path;
use tokio::sync::watch;

async fn download(root: &Path, server: &support::Fixture) -> (String, DestinationIdentity) {
    let id = uuid::Uuid::new_v4().to_string();
    let identity = DestinationIdentity::from_path(root).unwrap();
    let (_tx, stop) = watch::channel(false);
    assert!(matches!(downloads::transfer_direct(root, identity, &id, "mp4", "fixture", &server.url("/v.wav"), false, stop, |_| {}).await.unwrap(), TransferResult::Completed { .. }));
    (id, identity)
}
fn subtitles(server: &support::Fixture) -> Vec<ExternalSubtitle> {
    vec![ExternalSubtitle { language: "en".into(), name: "English".into(), url: server.url("/sub.srt"), no_proxy: true }]
}

#[tokio::test]
async fn interrupted_sidecars_resume_and_missing_tracks_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0)), ("/sub.srt", b"1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_vec())]).await;
    let (id, identity) = download(dir.path(), &server).await;
    let (tx, stop) = watch::channel(true);
    assert!(!direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 1, &stop).await.unwrap());
    tx.send_replace(false);
    assert!(direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 1, &stop).await.unwrap());
    let (tracks, missing) = direct_subtitles::read(dir.path(), identity, &id, "mp4").unwrap().unwrap();
    assert_eq!(tracks.len(), 1); assert_eq!(missing, 1);
    assert!(direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 1, &stop).await.unwrap());
    let record = std::fs::read_to_string(dir.path().join(format!(".cloudstream-{id}/direct-subtitles.json"))).unwrap();
    assert!(!record.contains("127.0.0.1"));
}

#[tokio::test]
async fn edited_subtitle_and_unrelated_files_survive_confirmed_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0)), ("/sub.srt", b"1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_vec())]).await;
    let (id, identity) = download(dir.path(), &server).await;
    let (_tx, stop) = watch::channel(false);
    assert!(direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 0, &stop).await.unwrap());
    let (tracks, _) = direct_subtitles::read(dir.path(), identity, &id, "mp4").unwrap().unwrap();
    std::fs::write(&tracks[0].url, b"my edited subtitle").unwrap();
    assert!(direct_subtitles::read(dir.path(), identity, &id, "mp4").is_err());
    let report = downloads::remove_job_assets(dir.path(), identity, &id, "mp4", true).unwrap();
    assert!(report.file_deleted && report.temp_cleaned);
    assert!(report.notes.iter().any(|note| note.contains("changed")));
    assert_eq!(std::fs::read(&tracks[0].url).unwrap(), b"my edited subtitle");
}

#[tokio::test]
async fn a_preexisting_subtitle_folder_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0)), ("/sub.srt", b"1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_vec())]).await;
    let (id, identity) = download(dir.path(), &server).await;
    let valuable = outside.path().join("keep"); std::fs::write(&valuable, b"keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(format!("cloudstream-{id}.subtitles"))).unwrap();
    let (_tx, stop) = watch::channel(false);
    assert!(direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 0, &stop).await.is_err());
    assert_eq!(std::fs::read(valuable).unwrap(), b"keep");
}

#[tokio::test]
async fn removing_the_record_but_keeping_the_video_also_keeps_subtitles() {
    let dir = tempfile::tempdir().unwrap();
    let server = support::serve(vec![("/v.wav", support::wav(2.0)), ("/sub.srt", b"1\n00:00:00,000 --> 00:00:01,000\nHello\n".to_vec())]).await;
    let (id, identity) = download(dir.path(), &server).await;
    let (_tx, stop) = watch::channel(false);
    assert!(direct_subtitles::save(dir.path(), identity, &id, "mp4", &subtitles(&server), 0, &stop).await.unwrap());
    let (tracks, _) = direct_subtitles::read(dir.path(), identity, &id, "mp4").unwrap().unwrap();
    let report = downloads::remove_job_assets(dir.path(), identity, &id, "mp4", false).unwrap();
    assert!(!report.file_deleted && report.temp_cleaned);
    assert!(std::path::Path::new(&tracks[0].url).exists());
    assert!(dir.path().join(format!("cloudstream-{id}.mp4")).exists());
}
