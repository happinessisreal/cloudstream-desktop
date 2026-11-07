//! Single-instance routing (T27): one primary, later launches forward their arguments and exit.
use cloudstream_core::instance::*;
use std::{io::{Read, Write}, os::unix::{fs::PermissionsExt, net::UnixStream}, sync::{mpsc, Arc, Mutex}, time::Duration};

fn short_dir() -> tempfile::TempDir {
    // Unix socket paths are limited to ~108 bytes, so keep the folder name short.
    tempfile::Builder::new().prefix("cs").tempdir_in("/tmp").unwrap()
}

fn primary(dir: &std::path::Path) -> Primary {
    match acquire(dir).unwrap() {
        Role::Primary(primary) => primary,
        Role::Secondary => panic!("expected to be primary"),
    }
}

#[test]
fn the_first_launch_is_primary_and_later_launches_forward_their_arguments() {
    let dir = short_dir();
    let (tx, rx) = mpsc::channel();
    let first = primary(dir.path());
    assert!(first.socket_path().starts_with(dir.path()));
    let _server = first.serve(move |args| tx.send(args).unwrap());

    // A second and third launch find the lock taken and become secondaries.
    for n in 0..3 {
        assert!(matches!(acquire(dir.path()).unwrap(), Role::Secondary), "launch {n}");
        let args = vec![format!("cloudstreamsearch://query{n}"), "/home/me/Movies/a b.mkv".into()];
        forward(dir.path(), &args).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), args);
    }
}

#[test]
fn a_stale_socket_from_a_crashed_instance_does_not_block_a_new_primary() {
    let dir = short_dir();
    {
        let crashed = primary(dir.path());
        // Simulate a crash: leak the socket file by dropping the lock but keeping the file.
        let path = crashed.socket_path().to_path_buf();
        drop(crashed);
        std::os::unix::net::UnixListener::bind(&path).unwrap();
    }
    let (tx, rx) = mpsc::channel();
    let _server = primary(dir.path()).serve(move |args| tx.send(args).unwrap());
    forward(dir.path(), &["cloudstreamsearch://after".into()]).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), vec!["cloudstreamsearch://after".to_string()]);
}

#[test]
fn something_that_is_not_a_socket_is_never_deleted() {
    let dir = short_dir();
    std::fs::write(dir.path().join("cloudstream-desktop.sock"), b"important").unwrap();
    assert!(acquire(dir.path()).err().unwrap().to_string().contains("not a socket"));
    assert_eq!(std::fs::read(dir.path().join("cloudstream-desktop.sock")).unwrap(), b"important");
}

#[test]
fn the_runtime_directory_is_private_and_unsafe_ones_are_refused() {
    let dir = short_dir();
    let loose = dir.path().join("loose");
    std::fs::create_dir(&loose).unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o777)).unwrap();
    ensure_private_directory(&loose).unwrap();
    assert_eq!(std::fs::metadata(&loose).unwrap().permissions().mode() & 0o777, 0o700, "group/other access is removed");
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
    assert!(ensure_private_directory(&dir.path().join("link")).unwrap_err().to_string().contains("plain folder"));
    let file = dir.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(ensure_private_directory(&file).is_err());
}

fn raw(dir: &std::path::Path, bytes: &[u8]) -> String {
    let mut stream = UnixStream::connect(dir.join("cloudstream-desktop.sock")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    // An oversized request is cut off by the server mid-send; that is the behaviour under test.
    let _ = stream.write_all(bytes);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn malformed_oversized_and_flooding_requests_are_refused_and_the_server_survives() {
    let dir = short_dir();
    let received = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let sink = received.clone();
    let _server = primary(dir.path()).serve(move |args| {
        if args.first().map(String::as_str) == Some("panic") {
            panic!("handler failure");
        }
        sink.lock().unwrap().push(args);
    });

    assert!(raw(dir.path(), b"not json\n").contains("\"ok\":false"));
    assert!(raw(dir.path(), b"{\"version\":2,\"arguments\":[]}\n").contains("Unsupported request version"));
    assert!(raw(dir.path(), b"{\"version\":1,\"arguments\":[]}").contains("incomplete"), "no newline");
    let too_many = serde_json::json!({ "version": 1, "arguments": vec!["x"; 33] }).to_string() + "\n";
    assert!(raw(dir.path(), too_many.as_bytes()).contains("Too many arguments"));
    let too_long = serde_json::json!({ "version": 1, "arguments": ["a".repeat(9000)] }).to_string() + "\n";
    assert!(raw(dir.path(), too_long.as_bytes()).contains("too long"));
    let huge = vec![b'a'; 300 * 1024];
    assert!(!raw(dir.path(), &huge).contains("\"ok\":true"), "an oversized request is never accepted");
    assert!(received.lock().unwrap().is_empty(), "nothing malformed reached the handler");

    // A handler panic is contained; the next launch still works.
    let _ = raw(dir.path(), b"{\"version\":1,\"arguments\":[\"panic\"]}\n");
    forward(dir.path(), &["cloudstreamsearch://still-alive".into()]).unwrap();
    assert_eq!(received.lock().unwrap().last().unwrap(), &vec!["cloudstreamsearch://still-alive".to_string()]);
}

#[test]
fn forwarding_with_no_primary_fails_clearly_instead_of_hanging() {
    let dir = short_dir();
    let started = std::time::Instant::now();
    let error = forward(dir.path(), &["x".into()]).unwrap_err().to_string();
    assert!(error.contains("did not answer"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(6));
}

#[test]
fn relative_file_arguments_are_made_absolute_but_links_and_flags_are_left_alone() {
    let cwd = std::path::Path::new("/home/me/Videos");
    let out = absolutize_arguments(
        &["movie.mkv".into(), "sub/dir/a.mp4".into(), "/abs/path.mkv".into(), "--flag".into(), "https://x.test/a".into(), "cloudstreamsearch://q".into(), "csshare:abc?def".into(), "".into()],
        cwd,
    );
    assert_eq!(out, ["/home/me/Videos/movie.mkv", "/home/me/Videos/sub/dir/a.mp4", "/abs/path.mkv", "--flag", "https://x.test/a", "cloudstreamsearch://q", "csshare:abc?def", ""]);
}
