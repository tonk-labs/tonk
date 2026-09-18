//! End-to-end telemetry: the real binary posts one `/batch` request
//! per command, and stays completely silent when opted out.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::time::Duration;

/// Accept one connection, read one HTTP request (headers +
/// content-length body), reply 200, send the raw request over `tx`.
fn serve_once(listener: TcpListener, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buffer).into_owned();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length: usize = text[..split]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if text[split + 4..].len() >= length {
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                            );
                            let _ = tx.send(text);
                            break;
                        }
                    }
                }
            }
        }
    });
}

/// Path to the `tonk` binary under test. The compile-time
/// `CARGO_BIN_EXE_tonk` points into the sandbox the tests were BUILT
/// in; when CI runs them from a `cargo nextest archive` on another
/// machine that path is gone, and nextest supplies the remapped
/// location at runtime instead.
fn tonk_bin() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

/// Run `tonk help glossary` with telemetry pointed at `endpoint` and state
/// isolated in `state_dir`. `extra_env` overrides for opt-out cases.
fn run_tonk_help(
    state_dir: &std::path::Path,
    endpoint: &str,
    extra_env: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(tonk_bin());
    cmd.args(["help", "glossary"])
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env("TONK_POSTHOG_KEY", "test-key")
        .env("TONK_POSTHOG_ENDPOINT", endpoint)
        .env_remove("DO_NOT_TRACK")
        .env_remove("TONK_TELEMETRY")
        // This branch's release check runs at the end of every command.
        // These tests are about telemetry, not updates: opting out keeps
        // them off the network, and stops them stamping the developer's
        // real update.json. TONK_UPDATE_STATE is belt-and-braces — if the
        // check ever runs anyway, it must not touch real state.
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("TONK_UPDATE_STATE", state_dir);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("tonk binary runs")
}

fn run_tonk_account_status(
    home: &std::path::Path,
    endpoint: &str,
    extra_env: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(tonk_bin());
    cmd.args(["account", "status", "--json"])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPACES_STATE", home.join("spaces"))
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_POSTHOG_KEY", "test-key")
        .env("TONK_POSTHOG_ENDPOINT", endpoint)
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env_remove("DO_NOT_TRACK")
        .env_remove("TONK_TELEMETRY")
        .env_remove("TONK_SPACE");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("tonk account status runs")
}

fn request_body(request: &str) -> serde_json::Value {
    let (_, body) = request.split_once("\r\n\r\n").expect("HTTP body");
    serde_json::from_str(body).expect("JSON batch")
}

#[dialog_common::test]
fn help_posts_one_command_run_event() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_help(state.path(), &endpoint, &[]);
    assert!(output.status.success());

    let request = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("request arrives");
    assert!(request.starts_with("POST /batch/"));
    assert!(request.contains("\"cli_command_run\""));
    assert!(request.contains("\"command\":\"help\""));
    assert!(request.contains("\"success\":true"));
    assert!(request.contains("\"duration_ms\""));
    assert!(request.contains("\"environment\":\"cli\""));

    // First run printed the notice; second run must not.
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("tonk telemetry off"),
        "notice on first run: {stderr}"
    );
}

#[dialog_common::test]
fn do_not_track_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_help(state.path(), &endpoint, &[("DO_NOT_TRACK", "1")]);
    assert!(output.status.success());
    assert!(
        rx.recv_timeout(Duration::from_secs(1)).is_err(),
        "no request may reach the endpoint under DO_NOT_TRACK=1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        !stderr.contains("tonk telemetry off"),
        "no notice when opted out"
    );
}

#[dialog_common::test]
fn notice_prints_only_once() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, _rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let first = run_tonk_help(state.path(), &endpoint, &[]);
    assert!(String::from_utf8_lossy(&first.stderr).contains("tonk telemetry off"));

    // Second run: endpoint may be gone (serve_once handled one
    // request); flush is best-effort so the command still succeeds.
    let second = run_tonk_help(state.path(), &endpoint, &[]);
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains("tonk telemetry off"));
}

#[dialog_common::test]
fn account_command_batches_shared_events_and_generic_summary() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);

    let home = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_account_status(home.path(), &endpoint, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("request arrives");
    let payload = request_body(&request);
    let batch = payload["batch"].as_array().expect("batch array");
    let account = batch
        .iter()
        .filter(|event| event["event"] == "account_event")
        .collect::<Vec<_>>();
    let generic = batch
        .iter()
        .filter(|event| event["event"] == "cli_command_run")
        .collect::<Vec<_>>();
    assert_eq!(generic.len(), 1);
    assert_eq!(account.len(), 2);
    assert_eq!(account[0]["properties"]["action"], "load_account");
    assert_eq!(account[0]["properties"]["phase"], "started");
    assert_eq!(account[1]["properties"]["phase"], "finished");
    assert!(generic[0]["properties"].get("stage").is_none());
    assert!(generic[0]["properties"].get("failure_kind").is_none());

    let wire = serde_json::to_string(&payload).unwrap();
    let home_text = home.path().to_string_lossy();
    for sentinel in [
        "person@example.com",
        "did:key:",
        "callback=http",
        "delegation",
        home_text.as_ref(),
    ] {
        assert!(
            !wire.contains(sentinel),
            "batch exposed {sentinel:?}: {wire}"
        );
    }
}

#[dialog_common::test]
fn opted_out_account_command_sends_no_batch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);
    let home = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_account_status(home.path(), &endpoint, &[("DO_NOT_TRACK", "1")]);
    assert!(output.status.success());
    assert!(rx.recv_timeout(Duration::from_secs(1)).is_err());
}
