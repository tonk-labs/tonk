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
        let Ok((mut stream, _)) = listener.accept() else { return };
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

/// Run `tonk guide` with telemetry pointed at `endpoint` and state
/// isolated in `state_dir`. `extra_env` overrides for opt-out cases.
fn run_tonk_guide(state_dir: &std::path::Path, endpoint: &str, extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tonk"));
    cmd.arg("guide")
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env("TONK_POSTHOG_KEY", "test-key")
        .env("TONK_POSTHOG_ENDPOINT", endpoint)
        .env_remove("DO_NOT_TRACK")
        .env_remove("TONK_TELEMETRY");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("tonk binary runs")
}

#[dialog_common::test]
fn guide_posts_one_command_run_event() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_guide(state.path(), &endpoint, &[]);
    assert!(output.status.success());

    let request = rx.recv_timeout(Duration::from_secs(5)).expect("request arrives");
    assert!(request.starts_with("POST /batch/"));
    assert!(request.contains("\"cli_command_run\""));
    assert!(request.contains("\"command\":\"guide\""));
    assert!(request.contains("\"success\":true"));
    assert!(request.contains("\"duration_ms\""));

    // First run printed the notice; second run must not.
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("tonk telemetry off"), "notice on first run: {stderr}");
}

#[dialog_common::test]
fn do_not_track_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let output = run_tonk_guide(state.path(), &endpoint, &[("DO_NOT_TRACK", "1")]);
    assert!(output.status.success());
    assert!(
        rx.recv_timeout(Duration::from_secs(1)).is_err(),
        "no request may reach the endpoint under DO_NOT_TRACK=1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!stderr.contains("tonk telemetry off"), "no notice when opted out");
}

#[dialog_common::test]
fn notice_prints_only_once() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, _rx) = mpsc::channel();
    serve_once(listener, tx);

    let state = tempfile::tempdir().expect("tempdir");
    let first = run_tonk_guide(state.path(), &endpoint, &[]);
    assert!(String::from_utf8_lossy(&first.stderr).contains("tonk telemetry off"));

    // Second run: endpoint may be gone (serve_once handled one
    // request); flush is best-effort so the command still succeeds.
    let second = run_tonk_guide(state.path(), &endpoint, &[]);
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains("tonk telemetry off"));
}
