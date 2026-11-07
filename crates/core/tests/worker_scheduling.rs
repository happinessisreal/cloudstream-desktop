use cloudstream_core::{models::*, worker::*};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
};
use tempfile::tempdir;
use tokio::sync::mpsc;

fn create_fake_worker_script(script_body: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempdir().expect("tempdir");
    let script_path = dir.path().join("fake_worker.py");
    let mut file = File::create(&script_path).expect("create fake worker script");
    file.write_all(script_body.as_bytes()).expect("write script");
    let mut perms = fs::metadata(&script_path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("set permissions");

    let fake_jar = dir.path().join("worker.jar");
    File::create(&fake_jar).expect("create fake jar");

    let python = PathBuf::from("python3");
    (dir, python, fake_jar)
}

#[tokio::test]
async fn out_of_order_responses_demultiplex_correctly() {
    let script = r#"#!/usr/bin/env python3
import sys, json, time, threading

def handle(line):
    try:
        req = json.loads(line.strip())
    except:
        return
    req_id = req.get("id")
    method = req.get("method")
    params = req.get("params", {})
    if method == "health":
        resp = {"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 2, "runtime": "jvm", "capabilities": []}}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()
        return

    delay = params.get("delay", 0)
    time.sleep(delay)
    resp = {"jsonrpc": "2.0", "id": req_id, "result": {"echo": params.get("val")}}
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    if not line: break
    t = threading.Thread(target=handle, args=(line,))
    t.start()
"#;

    let (_dir, _python, jar) = create_fake_worker_script(script);
    // Point java to a shell script wrapper that launches python with fake_worker.py
    let wrapper_path = _dir.path().join("launcher.sh");
    let script_path = _dir.path().join("fake_worker.py");
    fs::write(
        &wrapper_path,
        format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script_path.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).unwrap();

    let worker = Worker::new(WorkerConfig {
        java: wrapper_path,
        jar,
        data_dir: None,
    });

    let installed: Vec<InstalledPlugin> = vec![];

    // Fire slow request (takes 0.15s) and fast request (takes 0.01s) concurrently
    let slow = worker.request(&installed, "test", json!({"delay": 0.15, "val": "slow"}));
    let fast = worker.request(&installed, "test", json!({"delay": 0.01, "val": "fast"}));

    let (res_slow, res_fast) = tokio::join!(slow, fast);

    assert_eq!(res_slow.unwrap()["echo"], "slow");
    assert_eq!(res_fast.unwrap()["echo"], "fast");
}

#[tokio::test]
async fn hung_request_cancellation_leaves_other_requests_intact() {
    let script = r#"#!/usr/bin/env python3
import sys, json, time, threading

active_jobs = {}

def handle(line):
    req = json.loads(line.strip())
    req_id = req.get("id")
    method = req.get("method")
    params = req.get("params", {})
    if method == "health":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 2, "capabilities": []}}) + "\n")
        sys.stdout.flush()
        return
    if method == "cancel":
        target = params.get("id")
        if target in active_jobs:
            active_jobs[target]["cancelled"] = True
        return

    active_jobs[req_id] = {"cancelled": False}
    delay = params.get("delay", 0)
    for _ in range(int(delay * 20)):
        if active_jobs[req_id]["cancelled"]:
            sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32800, "message": "Request cancelled"}}) + "\n")
            sys.stdout.flush()
            return
        time.sleep(0.05)

    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"status": "ok", "tag": params.get("tag")}}) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    if not line: break
    threading.Thread(target=handle, args=(line,)).start()
"#;

    let (_dir, _, jar) = create_fake_worker_script(script);
    let wrapper_path = _dir.path().join("launcher.sh");
    let script_path = _dir.path().join("fake_worker.py");
    fs::write(
        &wrapper_path,
        format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script_path.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).unwrap();

    let worker = Arc::new(Worker::new(WorkerConfig {
        java: wrapper_path,
        jar,
        data_dir: None,
    }));

    let installed: Vec<InstalledPlugin> = vec![];

    // Request 1 hangs for 10 seconds
    let id_hung = worker.next_id();
    let worker_hung = Arc::clone(&worker);
    let installed_hung = installed.clone();
    let hung_handle = tokio::spawn(async move {
        worker_hung
            .request_with_id(
                &installed_hung,
                id_hung,
                "search",
                json!({"delay": 10.0, "tag": "hung"}),
                None,
            )
            .await
    });

    // Request 2 runs quickly (0.05s)
    let id_healthy = worker.next_id();
    let res_healthy = worker
        .request_with_id(
            &installed,
            id_healthy,
            "search",
            json!({"delay": 0.05, "tag": "healthy"}),
            None,
        )
        .await
        .expect("Healthy request must succeed while hung request is running");

    assert_eq!(res_healthy["tag"], "healthy");

    // Cancel hung request
    worker.cancel(id_hung).await;

    // Await hung request
    let hung_res = hung_handle.await.unwrap();
    // It should have returned an error (Request cancelled or error)
    assert!(
        hung_res.is_err(),
        "Hung request should have failed after cancellation"
    );

    // Another subsequent request still succeeds normally
    let res_after = worker
        .request(&installed, "search", json!({"delay": 0.01, "tag": "after"}))
        .await
        .expect("Subsequent request must succeed");
    assert_eq!(res_after["tag"], "after");
}

#[tokio::test]
async fn worker_crash_fails_in_flight_calls_once_and_recovers_on_next_call() {
    let script = r#"#!/usr/bin/env python3
import sys, json, os

for line in sys.stdin:
    if not line: break
    req = json.loads(line.strip())
    req_id = req.get("id")
    method = req.get("method")
    if method == "health":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 2, "capabilities": []}}) + "\n")
        sys.stdout.flush()
        continue
    if method == "crash":
        # Simulate abrupt process crash / death
        os._exit(1)
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"ok": True}}) + "\n")
    sys.stdout.flush()
"#;

    let (_dir, _, jar) = create_fake_worker_script(script);
    let wrapper_path = _dir.path().join("launcher.sh");
    let script_path = _dir.path().join("fake_worker.py");
    fs::write(
        &wrapper_path,
        format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script_path.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).unwrap();

    let worker = Worker::new(WorkerConfig {
        java: wrapper_path,
        jar,
        data_dir: None,
    });

    let installed: Vec<InstalledPlugin> = vec![];

    // First call succeeds
    let res1 = worker.request(&installed, "ping", json!({})).await;
    assert!(res1.is_ok());

    // Second call triggers crash
    let res_crash = worker.request(&installed, "crash", json!({})).await;
    assert!(
        res_crash.is_err(),
        "Crashed worker call must fail"
    );
    let err_str = res_crash.err().unwrap().to_string();
    assert!(
        err_str.contains("Worker disconnected"),
        "Expected Worker disconnected, got: {err_str}"
    );

    // Third call should restart the worker and succeed
    let res_recovered = worker.request(&installed, "ping", json!({})).await;
    assert!(
        res_recovered.is_ok(),
        "Worker should recover and succeed on subsequent call: {:?}",
        res_recovered.err()
    );
}

#[tokio::test]
async fn malformed_stdout_is_ignored_without_corrupting_results() {
    let script = r#"#!/usr/bin/env python3
import sys, json

for line in sys.stdin:
    if not line: break
    req = json.loads(line.strip())
    req_id = req.get("id")
    method = req.get("method")
    if method == "health":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 2, "capabilities": []}}) + "\n")
        sys.stdout.flush()
        continue

    # Emit garbage and malformed stdout before the valid response
    sys.stdout.write("LOGGER: plugin debug line not json\n")
    sys.stdout.write("{malformed json line\n")
    sys.stdout.write("\n")
    sys.stdout.flush()

    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"valid": True}}) + "\n")
    sys.stdout.flush()
"#;

    let (_dir, _, jar) = create_fake_worker_script(script);
    let wrapper_path = _dir.path().join("launcher.sh");
    let script_path = _dir.path().join("fake_worker.py");
    fs::write(
        &wrapper_path,
        format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script_path.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).unwrap();

    let worker = Worker::new(WorkerConfig {
        java: wrapper_path,
        jar,
        data_dir: None,
    });

    let installed: Vec<InstalledPlugin> = vec![];
    let res = worker.request(&installed, "test", json!({})).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap()["valid"], true);
}

#[tokio::test]
async fn progress_events_are_streamed_before_final_response() {
    let script = r#"#!/usr/bin/env python3
import sys, json, time

for line in sys.stdin:
    if not line: break
    req = json.loads(line.strip())
    req_id = req.get("id")
    method = req.get("method")
    if method == "health":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": req_id, "result": {"protocolVersion": 2, "capabilities": []}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "loadLinks":
        # Emit two progress notifications
        p1 = {"jsonrpc": "2.0", "method": "progress", "params": {"id": req_id, "type": "stream", "url": "https://example.com/stream1"}}
        sys.stdout.write(json.dumps(p1) + "\n")
        sys.stdout.flush()
        time.sleep(0.01)

        p2 = {"jsonrpc": "2.0", "method": "progress", "params": {"id": req_id, "type": "subtitle", "lang": "English"}}
        sys.stdout.write(json.dumps(p2) + "\n")
        sys.stdout.flush()
        time.sleep(0.01)

        final_resp = {"jsonrpc": "2.0", "id": req_id, "result": {"streams": [1], "subtitles": [1]}}
        sys.stdout.write(json.dumps(final_resp) + "\n")
        sys.stdout.flush()
"#;

    let (_dir, _, jar) = create_fake_worker_script(script);
    let wrapper_path = _dir.path().join("launcher.sh");
    let script_path = _dir.path().join("fake_worker.py");
    fs::write(
        &wrapper_path,
        format!("#!/bin/sh\nexec python3 {} \"$@\"\n", script_path.display()),
    )
    .unwrap();
    let mut perms = fs::metadata(&wrapper_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&wrapper_path, perms).unwrap();

    let worker = Worker::new(WorkerConfig {
        java: wrapper_path,
        jar,
        data_dir: None,
    });

    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel();
    let installed: Vec<InstalledPlugin> = vec![];

    let res = worker
        .request_with_progress(&installed, "loadLinks", json!({}), Some(prog_tx))
        .await
        .unwrap();

    assert_eq!(res["streams"].as_array().unwrap().len(), 1);

    // Verify progress events arrived
    let mut events = Vec::new();
    while let Ok(event) = prog_rx.try_recv() {
        events.push(event);
    }

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "stream");
    assert_eq!(events[1]["type"], "subtitle");
}
