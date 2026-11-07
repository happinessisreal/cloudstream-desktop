use cloudstream_core::{models::*, store::Store, subscriptions::{self, SubscriptionSnapshot}};
use std::collections::BTreeMap;

fn media() -> MediaItem { MediaItem { provider_id: "provider".into(), name: "Show".into(), url: "https://example.test/show".into(), ..serde_json::from_value(serde_json::json!({"providerId":"provider","name":"Show","url":"https://example.test/show"})).unwrap() } }
fn episode(n: i32) -> Episode { Episode { name: format!("Episode {n}"), episode: Some(n), season: Some(1), data: format!("url-{n}"), ..Default::default() } }
fn details(episodes: Vec<Episode>) -> MediaDetails { MediaDetails { item: media(), plot: None, tags: vec![], episodes, duration: None, unique_url: None, background_poster_url: None, logo_url: None, content_rating: None, coming_soon: None, sync_data: BTreeMap::new(), trailers: vec![], actors: vec![], recommendations: vec![], dub_groups: BTreeMap::new(), seasons: vec![], extra: BTreeMap::new() } }
fn generation(item: &LibraryItem) -> &str { item.extra["subscriptionGeneration"].as_str().unwrap() }

#[test]
fn snapshots_survive_restart_ignore_reordering_and_reject_stale_subscriptions() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.db");
    let store = Store::open(&db).unwrap();
    store.update_settings(serde_json::json!({"subscriptionNotifications":true}), Some("global"), None).unwrap();
    let entry = store.set_library_subscribed(media(), true, Some("default")).unwrap();
    let id = entry.id.clone();
    let token = generation(&entry).to_owned();
    let first = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2)])).unwrap();
    let baseline = store.apply_subscription_snapshot("default", &id, &token, first.clone(), 1_000, true).unwrap().unwrap();
    assert!(baseline.first_check && baseline.new_episodes.is_empty());
    drop(store);
    let store = Store::open(&db).unwrap();
    let reversed = subscriptions::episode_snapshot(&details(vec![episode(2), episode(1)])).unwrap();
    assert_eq!(first, reversed);
    assert!(store.apply_subscription_snapshot("default", &id, &token, reversed, 2_000, true).unwrap().unwrap().new_episodes.is_empty());
    assert!(subscriptions::episode_snapshot(&details(vec![])).is_err());
    let new = subscriptions::episode_snapshot(&details(vec![episode(3), episode(2), episode(1)])).unwrap();
    let changes = store.apply_subscription_snapshot("default", &id, &token, new.clone(), 3_000, true).unwrap().unwrap();
    assert_eq!(changes.new_episodes["none"], vec!["season:1:episode:3"]);
    assert!(store.apply_subscription_snapshot("default", &id, &token, first, 4_000, true).unwrap().unwrap().new_episodes.is_empty());
    assert!(store.apply_subscription_snapshot("default", &id, &token, new, 5_000, true).unwrap().unwrap().new_episodes.is_empty());
    let key = format!("default::{id}");
    let snapshot: SubscriptionSnapshot = store.get("subscription_snapshot", &key).unwrap().unwrap();
    assert_eq!(snapshot.pending["none"].len(), 1);
    let unread = store.subscription_unread("default", &id, &token).unwrap();
    assert_eq!(unread["none"].len(), 1);
    let update = subscriptions::SubscriptionUpdate { media_id: id.clone(), title: "Show".into(), episode_count: 1, generation: token.clone(), episodes: unread };
    let four = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2), episode(3), episode(4)])).unwrap();
    store.apply_subscription_snapshot("default", &id, &token, four, 5_500, false).unwrap();
    store.acknowledge_subscription_unread("default", &update).unwrap();
    assert_eq!(store.subscription_unread("default", &id, &token).unwrap()["none"].iter().cloned().collect::<Vec<_>>(), vec!["season:1:episode:4"]);
    let still_pending: SubscriptionSnapshot = store.get("subscription_snapshot", &key).unwrap().unwrap();
    assert_eq!(still_pending.pending["none"].len(), 1);
    store.set_library_subscribed(media(), false, Some("default")).unwrap();
    assert!(store.get::<SubscriptionSnapshot>("subscription_snapshot", &key).unwrap().is_none());
    let renewed = store.set_library_subscribed(media(), true, Some("default")).unwrap();
    assert_ne!(generation(&renewed), token);
    let fresh = subscriptions::episode_snapshot(&details(vec![episode(1)])).unwrap();
    assert!(store.apply_subscription_snapshot("default", &id, &token, fresh.clone(), 6_000, true).unwrap().is_none());
    assert!(store.apply_subscription_snapshot("default", &id, generation(&renewed), fresh, 7_000, true).unwrap().unwrap().first_check);
}

#[test]
fn episode_keys_are_grouped_and_independent_of_order_and_ephemeral_tokens() {
    let mut d = details(vec![]);
    let mut changed = episode(1); changed.data = "expiring-token".into();
    d.dub_groups.insert("Subbed".into(), vec![episode(1), episode(2)]);
    d.dub_groups.insert("Dubbed".into(), vec![changed]);
    let result = subscriptions::episode_snapshot(&d).unwrap();
    assert_eq!(result["subbed"].len(), 2);
    assert_eq!(result["dubbed"].len(), 1);
    let mut reverse = d.clone(); reverse.dub_groups.get_mut("Subbed").unwrap().reverse(); reverse.dub_groups.get_mut("Dubbed").unwrap()[0].data = "new-token".into();
    assert_eq!(result, subscriptions::episode_snapshot(&reverse).unwrap());
    let unnumbered = |url: &str| Episode { name: "Episode".into(), data: url.into(), ..Default::default() };
    let original = subscriptions::episode_snapshot(&details(vec![unnumbered("https://example.test/watch?id=1&token=old"), unnumbered("https://example.test/watch?id=2&token=old")])).unwrap();
    let changed = subscriptions::episode_snapshot(&details(vec![unnumbered("https://example.test/watch?token=new&id=2"), unnumbered("https://example.test/watch?token=new&id=1")])).unwrap();
    assert_eq!(original, changed);
    assert_eq!(original["none"].len(), 2);
}

#[tokio::test]
async fn real_worker_subscription_check_baselines_skips_and_preserves_snapshot_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = cloudstream_core::App::open(dir.path().join("data"), resources).unwrap();
    app.install_sample().await.unwrap();
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Blender Open Movies").unwrap();
    let mut item = media();
    item.provider_id = provider.id;
    item.url = "https://download.blender.org/peach/trailer/trailer_480p.mov".into();
    let entry = app.set_library_subscribed(item.clone(), true, Some("default")).unwrap();
    let first = app.check_subscriptions("default", true).await.unwrap();
    assert_eq!(first.checked, 1);
    assert_eq!(first.failed, 0);
    assert!(first.updates.is_empty());
    let skipped = app.check_subscriptions("default", false).await.unwrap();
    assert_eq!(skipped.skipped, 1);
    let repeated = app.check_subscriptions("default", true).await.unwrap();
    assert_eq!(repeated.checked, 1);
    assert!(repeated.updates.is_empty());
    let key = format!("default::{}", entry.id);
    let extra = subscriptions::episode_snapshot(&details(vec![episode(42)])).unwrap();
    app.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), extra, 9_000, false).unwrap();
    let saved: SubscriptionSnapshot = app.store.get("subscription_snapshot", &key).unwrap().unwrap();
    app.worker.stop().await;
    app.store.remove("plugin", &app.store.installed().unwrap()[0].id).unwrap();
    let failed = app.check_subscriptions("default", true).await.unwrap();
    assert_eq!(failed.failed, 1);
    assert_eq!(failed.updates.len(), 1);
    assert_eq!(failed.updates[0].episode_count, 1);
    let after: SubscriptionSnapshot = app.store.get("subscription_snapshot", &key).unwrap().unwrap();
    assert_eq!(saved, after);
}

#[test]
fn profile_deletion_and_library_removal_cancel_old_subscription_observations() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("profiles.db")).unwrap();
    let profile = store.create_profile(CreateProfileInput { name: "Second".into(), avatar: None, pin: None }).unwrap();
    let first = store.set_library_subscribed(media(), true, Some("default")).unwrap();
    let second = store.set_library_subscribed(media(), true, Some(&profile.id)).unwrap();
    let observed = subscriptions::episode_snapshot(&details(vec![episode(1)])).unwrap();
    store.apply_subscription_snapshot("default", &first.id, generation(&first), observed.clone(), 100, true).unwrap();
    store.apply_subscription_snapshot(&profile.id, &second.id, generation(&second), observed.clone(), 100, true).unwrap();
    store.delete_profile(&profile.id).unwrap();
    assert!(store.set_library_subscribed(media(), true, Some(&profile.id)).is_err());
    assert!(store.get::<SubscriptionSnapshot>("subscription_snapshot", &format!("{}::{}", profile.id, second.id)).unwrap().is_none());
    assert!(store.apply_subscription_snapshot(&profile.id, &second.id, generation(&second), observed.clone(), 200, true).unwrap().is_none());
    assert!(store.get::<SubscriptionSnapshot>("subscription_snapshot", &format!("default::{}", first.id)).unwrap().is_some());
    store.remove_library_item(&first.id, Some("default")).unwrap();
    assert!(store.get::<SubscriptionSnapshot>("subscription_snapshot", &format!("default::{}", first.id)).unwrap().is_none());
    assert!(store.apply_subscription_snapshot("default", &first.id, generation(&first), observed, 200, true).unwrap().is_none());
}

#[tokio::test]
async fn notification_sink_is_opt_in_deduplicated_and_retries_failures() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let data = dir.path().join("data");
    let app = cloudstream_core::App::open(data.clone(), resources.clone()).unwrap();
    let entry = app.store.set_library_subscribed(media(), true, Some("default")).unwrap();
    let baseline = subscriptions::episode_snapshot(&details(vec![episode(1)])).unwrap();
    let two = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2)])).unwrap();
    app.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), baseline, 1, true).unwrap();
    app.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), two, 2, true).unwrap();
    let sent = Arc::new(AtomicUsize::new(0));
    let count = sent.clone();
    let report = app.deliver_subscription_alerts("default", move |_| { count.fetch_add(1, Ordering::SeqCst); async { Ok(()) } }).await.unwrap();
    assert_eq!(report.delivered, 0);
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    app.store.update_settings(serde_json::json!({"subscriptionNotifications":true}), Some("global"), None).unwrap();
    let three = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2), episode(3)])).unwrap();
    app.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), three, 3, true).unwrap();
    let failed = app.deliver_subscription_alerts("default", |_| async { anyhow::bail!("unavailable") }).await.unwrap();
    assert_eq!(failed.failed, 1);
    let count = sent.clone();
    let result = app.deliver_subscription_alerts("default", move |alert| { assert_eq!(alert.episode_count(), 1); count.fetch_add(1, Ordering::SeqCst); async { Ok(()) } }).await.unwrap();
    assert_eq!(result.delivered, 1);
    drop(app);
    let reopened = cloudstream_core::App::open(data, resources).unwrap();
    let count = sent.clone();
    let result = reopened.deliver_subscription_alerts("default", move |_| { count.fetch_add(1, Ordering::SeqCst); async { Ok(()) } }).await.unwrap();
    assert_eq!(result.delivered, 0);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
    let four = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2), episode(3), episode(4)])).unwrap();
    reopened.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), four, 4, true).unwrap();
    reopened.store.update_settings(serde_json::json!({"subscriptionNotifications":false}), Some("global"), None).unwrap();
    let five = subscriptions::episode_snapshot(&details(vec![episode(1), episode(2), episode(3), episode(4), episode(5)])).unwrap();
    // Simulate a provider request that began while opted in but finished after opt-out.
    reopened.store.apply_subscription_snapshot("default", &entry.id, generation(&entry), five, 5, true).unwrap();
    reopened.store.update_settings(serde_json::json!({"subscriptionNotifications":true}), Some("global"), None).unwrap();
    let count = sent.clone();
    assert_eq!(reopened.deliver_subscription_alerts("default", move |_| { count.fetch_add(1, Ordering::SeqCst); async { Ok(()) } }).await.unwrap().delivered, 0);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unsubscribing_cancels_inflight_provider_check() {
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let src = dir.path().join("SlowPlugin.java");
    std::fs::write(&src, r#"
import com.lagradost.cloudstream3.*;
import com.lagradost.cloudstream3.plugins.*;
import kotlin.coroutines.Continuation;
@CloudstreamPlugin public class SlowPlugin extends BasePlugin {
  public void load() { registerMainAPI(new Slow()); }
  public static class Slow extends MainAPI {
    public String getName() { return "Slow subscription fixture"; }
    public String getMainUrl() { return "https://example.test"; }
    public Object load(String path, Continuation<? super LoadResponse> continuation) {
      try { java.nio.file.Files.writeString(java.nio.file.Path.of(path), "started"); Thread.sleep(10000); }
      catch (Exception ignored) { }
      return null;
    }
  }
}
"#).unwrap();
    let classes = dir.path().join("classes"); std::fs::create_dir(&classes).unwrap();
    let javac = std::process::Command::new("javac").args(["--release", "21"]).arg("-cp").arg(resources.join("worker/worker.jar")).arg("-d").arg(&classes).arg(&src).output().unwrap();
    assert!(javac.status.success(), "{}", String::from_utf8_lossy(&javac.stderr));
    let jar = dir.path().join("slow.jar");
    assert!(std::process::Command::new("jar").arg("cf").arg(&jar).arg("-C").arg(&classes).arg(".").status().unwrap().success());
    let app = cloudstream_core::App::open(dir.path().join("data"), resources).unwrap();
    app.install_local(&jar).await.unwrap();
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Slow subscription fixture").unwrap();
    let marker = dir.path().join("started");
    let mut item = media(); item.provider_id = provider.id; item.url = marker.to_string_lossy().into_owned();
    app.set_library_subscribed(item.clone(), true, Some("default")).unwrap();
    let checking = app.clone();
    let job = tokio::spawn(async move { checking.check_subscriptions("default", true).await });
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while !marker.exists() { tokio::time::sleep(std::time::Duration::from_millis(30)).await; }
    }).await.expect("provider should start");
    app.set_library_subscribed(item, false, Some("default")).unwrap();
    let report = tokio::time::timeout(std::time::Duration::from_secs(3), job).await.expect("unsubscribe should cancel the check").unwrap().unwrap();
    assert_eq!(report.skipped, 1);
    assert_eq!(report.checked, 0);
    app.worker.stop().await;
}

#[test]
fn scheduled_checks_use_six_hour_clock_and_handle_clock_changes() {
    let snapshot = SubscriptionSnapshot { generation: "g".into(), observed: Default::default(), pending: Default::default(), unread: Default::default(), checked_at_ms: 1_000 };
    assert!(!subscriptions::check_is_due(Some(&snapshot), "g", 1_000 + subscriptions::CHECK_INTERVAL_MS - 1));
    assert!(subscriptions::check_is_due(Some(&snapshot), "g", 1_000 + subscriptions::CHECK_INTERVAL_MS));
    assert!(subscriptions::check_is_due(Some(&snapshot), "new-generation", 1_001));
    assert!(subscriptions::check_is_due(Some(&snapshot), "g", 999));
    assert!(subscriptions::check_is_due(None, "g", 999));
}

#[tokio::test]
async fn automatic_checks_reuse_only_fresh_profile_scoped_details_and_manual_checks_bypass_cache() {
    let dir = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources");
    let app = cloudstream_core::App::open(dir.path().join("data"), resources).unwrap();
    let plugin = app.install_sample().await.unwrap();
    let provider = app.providers().await.unwrap().into_iter().find(|p| p.name == "Blender Open Movies").unwrap();
    let mut item = media();
    item.provider_id = provider.id;
    item.url = "https://download.blender.org/peach/trailer/trailer_480p.mov".into();
    app.load_details("default", &item.provider_id, &item.url, None).await.unwrap();
    let default_entry = app.set_library_subscribed(item.clone(), true, Some("default")).unwrap();
    let second = app.create_profile(CreateProfileInput { name: "Other".into(), avatar: None, pin: None }).unwrap();
    app.set_library_subscribed(item, true, Some(&second.id)).unwrap();
    // Remove the backing worker source directly to distinguish cache hits from provider calls.
    app.worker.stop().await;
    app.store.remove("plugin", &plugin.id).unwrap();
    let other = app.check_subscriptions(&second.id, false).await.unwrap();
    assert_eq!(other.failed, 1, "another profile must not use the cached details");
    let cached = app.check_subscriptions("default", false).await.unwrap();
    assert_eq!(cached.checked, 1, "automatic check should use recently viewed details");
    let manual = app.check_subscriptions("default", true).await.unwrap();
    assert_eq!(manual.failed, 1, "manual refresh should always contact the provider");
    let key = format!("default::{}", default_entry.id);
    let mut snapshot: SubscriptionSnapshot = app.store.get("subscription_snapshot", &key).unwrap().unwrap();
    snapshot.checked_at_ms = 0;
    app.store.put("subscription_snapshot", &key, &snapshot).unwrap();
    app.invalidate_home_cache(None);
    let invalidated = app.check_subscriptions("default", false).await.unwrap();
    assert_eq!(invalidated.failed, 1, "source invalidation must clear cached details");
    app.worker.stop().await;
}

#[test]
fn disabled_sources_and_their_clones_cannot_commit_stale_observations() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("app.db")).unwrap();
    let mut item = media(); item.provider_id = "clone:one".into();
    let clone = ProviderCloneConfig { id: item.provider_id.clone(), parent_provider_id: "provider".into(), name: "Clone".into(), custom_url: "https://example.test".into(), credentials_handle: None, enabled: true, extra: Default::default() };
    store.save_provider_clone(&clone).unwrap();
    let entry = store.set_library_subscribed(item, true, Some("default")).unwrap();
    let observed = subscriptions::episode_snapshot(&details(vec![episode(1)])).unwrap();
    assert!(store.subscription_is_current("default", &entry.id, generation(&entry)).unwrap());
    store.set_provider_disabled("provider", true).unwrap();
    assert!(!store.subscription_is_current("default", &entry.id, generation(&entry)).unwrap());
    assert!(store.apply_subscription_snapshot("default", &entry.id, generation(&entry), observed.clone(), 1, false).unwrap().is_none());
    store.set_provider_disabled("provider", false).unwrap();
    assert!(store.apply_subscription_snapshot("default", &entry.id, generation(&entry), observed, 2, false).unwrap().unwrap().first_check);
}
