//! Notification click-through (T13/T27): scripted notifiers prove the argument handling and
//! timing; a real `notify-send` against a fake notification server on a private D-Bus proves the
//! actual click path.
use cloudstream_core::notify::*;
use std::{collections::HashMap, os::unix::fs::PermissionsExt, path::Path, sync::{Arc, Mutex}, time::Duration};
use tokio::sync::{mpsc, oneshot};
use zbus::{interface, object_server::SignalEmitter, zvariant::OwnedValue};

/// These tests write and then execute fresh scripts. Doing that in parallel with another test's
/// fork can fail with "Text file busy", so they run one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

fn note(title: &str) -> Notification {
    Notification { title: title.into(), body: "2 new episodes are available".into(), icon: "dev.cloudstream.desktop".into(), action: Some(("open".into(), "Open".into())) }
}

fn fast() -> Timing {
    Timing { settle: Duration::from_millis(400), click_window: Duration::from_secs(5) }
}

async fn clicked(rx: &mut mpsc::UnboundedReceiver<String>) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(4), rx.recv()).await.ok().flatten()
}

#[tokio::test]
async fn a_click_reports_the_chosen_action_and_text_is_never_interpreted() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let argv = dir.path().join("argv");
    // Records its arguments one per line, waits, then prints the action as a click would.
    let notifier = script(dir.path(), "notify-send", &format!("for a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\nsleep 0.8\necho open", argv.display()));
    let marker = dir.path().join("pwned");
    let title = format!("--evil; touch {} $(touch {}) `id`", marker.display(), marker.display());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let started = std::time::Instant::now();
    send(&notifier, &note(&title), fast(), move |action| tx.send(action).unwrap()).await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(1500), "send returns once the notification is accepted, not when it is closed");
    assert_eq!(clicked(&mut rx).await.as_deref(), Some("open"));

    let received = std::fs::read_to_string(&argv).unwrap();
    let lines: Vec<&str> = received.lines().collect();
    assert_eq!(lines[0], "--app-name=Cloudstream Desktop");
    assert!(lines.contains(&"--action=open=Open"));
    let dashes = lines.iter().position(|l| *l == "--").unwrap();
    assert_eq!(lines[dashes + 1], title.replace(|c: char| c.is_control(), ""), "after -- the title is plain text, even though it starts with dashes");
    assert!(!marker.exists(), "nothing in a title is ever executed");
}

#[tokio::test]
async fn other_output_a_missing_click_and_failures_do_not_trigger_the_action() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    // The server printed a different action name: ignored.
    let other = script(dir.path(), "other", "sleep 0.6\necho dismissed");
    let (tx, mut rx) = mpsc::unbounded_channel();
    send(&other, &note("T"), fast(), move |a| tx.send(a).unwrap()).await.unwrap();
    assert_eq!(clicked(&mut rx).await, None);

    // Closed without a click: the notifier exits quietly.
    let quiet = script(dir.path(), "quiet", "sleep 0.6");
    let (tx, mut rx) = mpsc::unbounded_channel();
    send(&quiet, &note("T"), fast(), move |a| tx.send(a).unwrap()).await.unwrap();
    assert_eq!(clicked(&mut rx).await, None);

    // A notifier that fails straight away is reported, so the alert is kept for another try.
    let broken = script(dir.path(), "broken", "exit 1");
    let error = send(&broken, &note("T"), fast(), |_| {}).await.unwrap_err().to_string();
    assert!(error.contains("unavailable"), "{error}");
    let missing = send(&dir.path().join("absent").display().to_string(), &note("T"), fast(), |_| {}).await.unwrap_err().to_string();
    assert!(missing.contains("unavailable"), "{missing}");
    assert!(send("notify-send", &Notification { title: "   ".into(), ..note("x") }, fast(), |_| {}).await.is_err(), "a notification needs a title");
    let bad = Notification { action: Some(("op en; rm".into(), "x".into())), ..note("T") };
    assert!(send(&other, &bad, fast(), |_| {}).await.unwrap_err().to_string().contains("Invalid notification action"));
}

#[tokio::test]
async fn a_notification_nobody_answers_is_ended_after_the_window() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let waiting = script(dir.path(), "waiting", &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()));
    let timing = Timing { settle: Duration::from_millis(300), click_window: Duration::from_millis(600) };
    send(&waiting, &note("T"), timing, |_| panic!("no click happened")).await.unwrap();
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the notifier must not be left running forever");
}

// ---- A real notify-send talking to a fake notification server ---------------------------------

struct Recorded {
    summary: String,
    body: String,
    actions: Vec<String>,
}

struct FakeServer {
    seen: Arc<Mutex<Vec<Recorded>>>,
    ready: Option<oneshot::Sender<()>>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl FakeServer {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into()]
    }
    fn get_server_information(&self) -> (String, String, String, String) {
        ("fake".into(), "test".into(), "1".into(), "1.2".into())
    }
    #[allow(clippy::too_many_arguments)]
    fn notify(&mut self, _app: String, _replaces: u32, _icon: String, summary: String, body: String, actions: Vec<String>, _hints: HashMap<String, OwnedValue>, _timeout: i32) -> u32 {
        let id = 42;
        self.seen.lock().unwrap().push(Recorded { summary, body, actions });
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(());
        }
        id
    }
    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;
}

#[tokio::test]
async fn a_real_notify_send_reports_a_click_from_a_notification_server() {
    let _serial = SERIAL.lock().await;
    let have_tools = |tool: &str| std::process::Command::new(tool).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    if !have_tools("notify-send") || !have_tools("dbus-daemon") {
        eprintln!("SKIPPED: notify-send or dbus-daemon is missing");
        return;
    }
    let dir = tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("bus");
    let config = dir.path().join("bus.conf");
    std::fs::write(&config, format!("<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\" \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\"><busconfig><type>session</type><listen>unix:path={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>", socket.display())).unwrap();
    let mut daemon = std::process::Command::new("dbus-daemon").args(["--nofork", "--config-file"]).arg(&config).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
    for _ in 0..100 {
        if socket.exists() { break; }
        std::thread::sleep(Duration::from_millis(30));
    }
    let address = format!("unix:path={}", socket.display());

    let seen = Arc::new(Mutex::new(Vec::new()));
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = zbus::connection::Builder::address(address.as_str()).unwrap()
        .name("org.freedesktop.Notifications").unwrap()
        .serve_at("/org/freedesktop/Notifications", FakeServer { seen: seen.clone(), ready: Some(ready_tx) }).unwrap()
        .build().await.unwrap();

    // The real notify-send, pointed at the private bus by a one-line wrapper (no global env change).
    let wrapper = script(dir.path(), "notify-send", &format!("export DBUS_SESSION_BUS_ADDRESS='{address}'\nexec notify-send \"$@\""));
    // The wrapper must call the real binary, not itself.
    let real = which_notify_send();
    std::fs::write(&wrapper, format!("#!/bin/sh\nexport DBUS_SESSION_BUS_ADDRESS='{address}'\nexec {real} \"$@\"\n")).unwrap();

    let (tx, mut rx) = mpsc::unbounded_channel();
    let timing = Timing { settle: Duration::from_millis(800), click_window: Duration::from_secs(10) };
    send(&wrapper, &note("Fixture show"), timing, move |action| tx.send(action).unwrap()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), ready_rx).await.expect("the server never received the notification").unwrap();
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].summary.as_str(), seen[0].body.as_str()), ("Fixture show", "2 new episodes are available"));
        assert_eq!(seen[0].actions, vec!["open".to_string(), "Open".to_string()], "the server was offered the Open action");
    }
    // The user clicks "Open" a moment later, once notify-send knows the notification's id.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let iface = server.object_server().interface::<_, FakeServer>("/org/freedesktop/Notifications").await.unwrap();
    // Like a real server: report the action, then close the notification (reason 3: closed by a
    // call to CloseNotification; real servers use 2 for "dismissed" — either ends notify-send).
    FakeServer::action_invoked(iface.signal_emitter(), 42, "open").await.unwrap();
    FakeServer::notification_closed(iface.signal_emitter(), 42, 2).await.unwrap();
    assert_eq!(clicked(&mut rx).await.as_deref(), Some("open"));
    let _ = daemon.kill();
    let _ = daemon.wait();
}

fn which_notify_send() -> String {
    String::from_utf8(std::process::Command::new("sh").args(["-c", "command -v notify-send"]).output().unwrap().stdout).unwrap().trim().to_string()
}
