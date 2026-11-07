//! MPRIS over a real private D-Bus session bus (T27): a separate client connection calls the
//! methods and watches the property signals, and `busctl` (a stand-in for playerctl) does too.
use cloudstream_core::{models::PlayerState, mpris::*};
use futures_lite::StreamExt;
use std::{process::{Child, Command, Stdio}, time::Duration};
use tokio::sync::mpsc;
use zbus::zvariant::{ObjectPath, OwnedValue};

struct Bus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Bus {
    fn start() -> Option<Self> {
        if Command::new("dbus-daemon").arg("--version").stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false) == false {
            return None;
        }
        let dir = tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap();
        let socket = dir.path().join("bus");
        let config = dir.path().join("bus.conf");
        std::fs::write(&config, format!(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n<busconfig><type>session</type><listen>unix:path={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>",
            socket.display()
        )).unwrap();
        let child = Command::new("dbus-daemon").args(["--nofork", "--config-file"]).arg(&config).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
        for _ in 0..100 {
            if socket.exists() {
                return Some(Self { child, address: format!("unix:path={}", socket.display()), _dir: dir });
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        None
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn client(address: &str) -> zbus::Connection {
    zbus::connection::Builder::address(address).unwrap().build().await.unwrap()
}

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

async fn call(conn: &zbus::Connection, interface: &str, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) {
    conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some(interface), method, body).await.unwrap();
}

async fn prop<T: TryFrom<OwnedValue>>(conn: &zbus::Connection, interface: &str, name: &str) -> T
where
    T::Error: Into<zbus::zvariant::Error>,
{
    let proxy = zbus::fdo::PropertiesProxy::builder(conn).destination(BUS_NAME).unwrap().path(OBJECT_PATH).unwrap().build().await.unwrap();
    let value = proxy.get(zbus::names::InterfaceName::try_from(interface).unwrap(), name).await.unwrap();
    T::try_from(value).map_err(Into::into).unwrap()
}

fn playing_state() -> PlayerState {
    PlayerState {
        session_id: Some("abc-123-def".into()),
        active: true,
        playing: true,
        paused: false,
        position: 12.5,
        duration: 600.0,
        volume: 50.0,
        speed: 1.5,
        item_name: Some("Big Buck Bunny".into()),
        episode_name: Some("Chapter 1".into()),
        has_next: true,
        has_prev: false,
        ..Default::default()
    }
}

#[test]
fn player_state_maps_to_mpris_state_without_inventing_values() {
    let state = MprisState::from_player(&playing_state());
    assert_eq!((state.status, state.title.as_deref(), state.album.as_deref()), (PlaybackStatus::Playing, Some("Chapter 1"), Some("Big Buck Bunny")));
    assert_eq!((state.length_us, state.position_us, state.volume, state.rate), (Some(600_000_000), 12_500_000, 0.5, 1.5));
    assert!(state.can_go_next && !state.can_go_previous);
    assert_eq!(state.track_id.as_deref(), Some("/dev/cloudstream/desktop/track/abc123def"), "only valid object path characters survive");

    let mut paused = playing_state();
    paused.paused = true;
    paused.duration = 0.0;
    paused.episode_name = None;
    let paused = MprisState::from_player(&paused);
    assert_eq!((paused.status, paused.length_us, paused.title.as_deref(), paused.album), (PlaybackStatus::Paused, None, Some("Big Buck Bunny"), None), "unknown duration is not reported as 0");

    let stopped = MprisState::from_player(&PlayerState::default());
    assert_eq!(stopped.status, PlaybackStatus::Stopped);
    assert_eq!((stopped.title, stopped.track_id), (None, None));
}

#[tokio::test]
async fn the_service_is_a_correct_mpris_player_on_a_real_bus() {
    let Some(bus) = Bus::start() else {
        eprintln!("SKIPPED: dbus-daemon is not available");
        return;
    };
    let (tx, mut requests) = mpsc::unbounded_channel();
    let service = MprisService::start_on(&bus.address, tx).await.unwrap();
    let conn = client(&bus.address).await;

    // Identity and capabilities before anything plays.
    assert_eq!(prop::<String>(&conn, "org.mpris.MediaPlayer2", "Identity").await, "Cloudstream Desktop");
    assert!(!prop::<bool>(&conn, "org.mpris.MediaPlayer2", "CanQuit").await, "remote quit is not offered");
    assert!(prop::<bool>(&conn, "org.mpris.MediaPlayer2", "CanRaise").await);
    assert_eq!(prop::<String>(&conn, PLAYER, "PlaybackStatus").await, "Stopped");
    assert!(!prop::<bool>(&conn, PLAYER, "CanSeek").await);

    // Controls that make no sense while stopped are ignored; Play/Pause reach the app.
    call(&conn, PLAYER, "Seek", &(5_000_000i64,)).await;
    call(&conn, PLAYER, "Next", &()).await;
    call(&conn, PLAYER, "PlayPause", &()).await;
    assert_eq!(requests.recv().await, Some(MprisRequest::PlayPause), "Seek and Next while stopped were dropped");

    // Subscribe to property changes, then push playing state.
    let proxy = zbus::fdo::PropertiesProxy::builder(&conn).destination(BUS_NAME).unwrap().path(OBJECT_PATH).unwrap().build().await.unwrap();
    let mut changes = proxy.receive_properties_changed().await.unwrap();
    service.update(MprisState::from_player(&playing_state())).await.unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !(seen.contains("PlaybackStatus") && seen.contains("Metadata") && seen.contains("Volume")) {
        let signal = tokio::time::timeout_at(deadline, changes.next()).await.expect("no PropertiesChanged signal").unwrap();
        let args = signal.args().unwrap();
        if args.interface_name().as_str() == PLAYER {
            seen.extend(args.changed_properties().keys().map(|k| k.to_string()));
        }
    }
    assert_eq!(prop::<String>(&conn, PLAYER, "PlaybackStatus").await, "Playing");
    assert_eq!(prop::<f64>(&conn, PLAYER, "Volume").await, 0.5);
    assert!(prop::<bool>(&conn, PLAYER, "CanSeek").await && prop::<bool>(&conn, PLAYER, "CanGoNext").await && !prop::<bool>(&conn, PLAYER, "CanGoPrevious").await);
    assert_eq!(prop::<i64>(&conn, PLAYER, "Position").await, 12_500_000);
    let metadata: std::collections::HashMap<String, OwnedValue> = prop(&conn, PLAYER, "Metadata").await;
    assert_eq!(String::try_from(metadata["xesam:title"].try_clone().unwrap()).unwrap(), "Chapter 1");
    assert_eq!(String::try_from(metadata["xesam:album"].try_clone().unwrap()).unwrap(), "Big Buck Bunny");
    assert_eq!(i64::try_from(metadata["mpris:length"].try_clone().unwrap()).unwrap(), 600_000_000);
    let track = zbus::zvariant::OwnedObjectPath::try_from(metadata["mpris:trackid"].try_clone().unwrap()).unwrap();
    assert_eq!(track.as_str(), "/dev/cloudstream/desktop/track/abc123def");

    // While playing, every control is delivered exactly as asked.
    call(&conn, PLAYER, "Next", &()).await;
    call(&conn, PLAYER, "Previous", &()).await;
    call(&conn, PLAYER, "Pause", &()).await;
    call(&conn, PLAYER, "Play", &()).await;
    call(&conn, PLAYER, "Stop", &()).await;
    call(&conn, PLAYER, "Seek", &(-7_000_000i64,)).await;
    call(&conn, PLAYER, "SetPosition", &(ObjectPath::try_from(track.as_str()).unwrap(), 90_000_000i64)).await;
    call(&conn, "org.mpris.MediaPlayer2", "Raise", &()).await;
    call(&conn, "org.mpris.MediaPlayer2", "Quit", &()).await;
    call(&conn, PLAYER, "OpenUri", &("cloudstreamsearch://hello".to_string(),)).await;
    call(&conn, PLAYER, "OpenUri", &("x".repeat(10_000),)).await;
    let mut got = Vec::new();
    while let Ok(Some(request)) = tokio::time::timeout(Duration::from_millis(300), requests.recv()).await {
        got.push(request);
    }
    assert_eq!(got, vec![
        MprisRequest::Next,
        // Previous was refused: the queue has no previous item.
        MprisRequest::Pause,
        MprisRequest::Play,
        MprisRequest::Stop,
        MprisRequest::Seek(-7_000_000),
        MprisRequest::SetPosition { track_id: "/dev/cloudstream/desktop/track/abc123def".into(), position_us: 90_000_000 },
        MprisRequest::Raise,
        MprisRequest::OpenUri("cloudstreamsearch://hello".into()),
    ], "Quit does nothing and an oversized OpenUri is dropped");

    // Writable properties turn into requests (clamped), not into direct state changes.
    let props = zbus::fdo::PropertiesProxy::builder(&conn).destination(BUS_NAME).unwrap().path(OBJECT_PATH).unwrap().build().await.unwrap();
    props.set(zbus::names::InterfaceName::try_from(PLAYER).unwrap(), "Volume", zbus::zvariant::Value::from(3.0f64)).await.unwrap();
    props.set(zbus::names::InterfaceName::try_from(PLAYER).unwrap(), "Rate", zbus::zvariant::Value::from(2.0f64)).await.unwrap();
    assert_eq!(requests.recv().await, Some(MprisRequest::SetVolume(1.0)));
    assert_eq!(requests.recv().await, Some(MprisRequest::SetRate(2.0)));
    assert_eq!(prop::<f64>(&conn, PLAYER, "Volume").await, 0.5, "the shown volume only changes when the player reports it");

    // Back to stopped: status, controls and metadata follow.
    service.update(MprisState::from_player(&PlayerState::default())).await.unwrap();
    assert_eq!(prop::<String>(&conn, PLAYER, "PlaybackStatus").await, "Stopped");
    assert!(!prop::<bool>(&conn, PLAYER, "CanPause").await);
    let stopped: std::collections::HashMap<String, OwnedValue> = prop(&conn, PLAYER, "Metadata").await;
    assert!(!stopped.contains_key("xesam:title"));
}

// busctl is a blocking child process: the service needs its own worker thread to answer it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busctl_sees_and_controls_the_player_like_playerctl_would() {
    let Some(bus) = Bus::start() else {
        eprintln!("SKIPPED: dbus-daemon is not available");
        return;
    };
    if Command::new("busctl").arg("--version").stdout(Stdio::null()).status().map(|s| s.success()).unwrap_or(false) == false {
        eprintln!("SKIPPED: busctl is not available");
        return;
    }
    let (tx, mut requests) = mpsc::unbounded_channel();
    let service = MprisService::start_on(&bus.address, tx).await.unwrap();
    service.update(MprisState::from_player(&playing_state())).await.unwrap();
    let busctl = |args: &[&str]| {
        let output = Command::new("busctl").arg(format!("--address={}", bus.address)).args(args).output().unwrap();
        assert!(output.status.success(), "busctl {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(busctl(&["list", "--no-pager"]).contains(BUS_NAME));
    assert_eq!(busctl(&["get-property", BUS_NAME, OBJECT_PATH, PLAYER, "PlaybackStatus"]).trim(), "s \"Playing\"");
    // The standard interface description is well formed and lists the controls.
    let introspection = busctl(&["introspect", BUS_NAME, OBJECT_PATH, PLAYER, "--no-pager"]);
    for member in ["PlayPause", "Seek", "SetPosition", "OpenUri", "Metadata", "CanGoNext", "Seeked"] {
        assert!(introspection.contains(member), "{member} missing from:\n{introspection}");
    }
    busctl(&["call", BUS_NAME, OBJECT_PATH, PLAYER, "PlayPause"]);
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), requests.recv()).await.unwrap(), Some(MprisRequest::PlayPause));
}

#[tokio::test]
async fn a_second_player_cannot_take_over_the_name() {
    let Some(bus) = Bus::start() else {
        eprintln!("SKIPPED: dbus-daemon is not available");
        return;
    };
    let (tx, _rx) = mpsc::unbounded_channel();
    let _first = MprisService::start_on(&bus.address, tx.clone()).await.unwrap();
    let error = MprisService::start_on(&bus.address, tx).await.err().expect("a second owner must be refused").to_string();
    assert!(error.contains("Could not register"), "{error}");
}
