//! End-to-end `tonk update`: the real binary against a fake release
//! served from a local listener.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Serve a fixed set of `path -> body` responses until dropped.
/// Anything unmapped gets a 404, which is how the "missing manifest"
/// case is exercised.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    serve_recording(routes).0
}

/// As [`serve`], but retain every requested path for routing assertions.
fn serve_recording(routes: Vec<(String, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 2048];
            let Ok(n) = stream.read(&mut buffer) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..n]).into_owned();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_owned();
            recorded.lock().expect("record request").push(path.clone());
            let body = routes
                .iter()
                .find(|(route, _)| *route == path)
                .map(|(_, body)| body.clone());
            let response = match body {
                Some(body) => {
                    let mut head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    head.extend_from_slice(&body);
                    head
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    (format!("http://127.0.0.1:{port}/releases"), requests)
}

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

/// The `install_dir` a receipt needs to describe this test's binary,
/// matching what `update::run`'s `running_binary()` resolves inside
/// the child process (canonicalized, so a symlinked target dir still
/// matches).
fn running_install_dir() -> String {
    let bin = binary();
    std::fs::canonicalize(&bin)
        .unwrap_or(bin)
        .parent()
        .expect("binary has a parent dir")
        .to_string_lossy()
        .into_owned()
}

/// Run `tonk update` against `endpoint` with state isolated in `state_dir`.
fn run_update(endpoint: &str, state_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("update")
        .args(args)
        .env("TONK_UPDATE_ENDPOINT", endpoint)
        .env("TONK_UPDATE_STATE", state_dir)
        // Isolated so the test never reads the developer's real
        // telemetry choice; without a key nothing is sent anyway.
        .env("TONK_TELEMETRY_STATE", state_dir)
        // Ambient installer input must not override the matching
        // receipt (or the safe stable default).
        .env("TONK_CHANNEL", "stable")
        .env_remove("TONK_POSTHOG_KEY")
        .output()
        .expect("run tonk update")
}

fn manifest_body(version: &str, commit: &str, channel: &str) -> Vec<u8> {
    format!(
        r#"{{"version":"{version}","commit":"{commit}","channel":"{channel}","built_at":"2026-07-16T00:00:00Z"}}"#
    )
    .into_bytes()
}

fn write_receipt(state_dir: &std::path::Path, channel: &str, install_dir: &str, commit: &str) {
    std::fs::write(
        state_dir.join("install.json"),
        format!(
            r#"{{"channel":"{channel}","version":"0.4.0","commit":"{commit}","install_dir":"{install_dir}","installed_at":"2026-07-16T00:00:00Z"}}"#
        ),
    )
    .expect("write receipt");
}

#[dialog_common::test]
fn it_fetches_stable_for_a_matching_stable_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let install_dir = running_install_dir();
    write_receipt(dir.path(), "stable", &install_dir, "abc1234def");

    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.4.0", "abc1234def", "stable"),
    )]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("already current"), "stdout: {stdout}");
    assert!(stdout.contains("abc1234"), "stdout: {stdout}");
    assert!(stdout.contains("stable"), "stdout: {stdout}");
}

#[dialog_common::test]
fn it_fetches_staging_for_a_matching_staging_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let install_dir = running_install_dir();
    write_receipt(dir.path(), "staging", &install_dir, "abc1234def");

    let endpoint = serve(vec![(
        "/releases/download/tonk-staging/manifest.json".to_owned(),
        manifest_body("0.4.0", "abc1234def", "staging"),
    )]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("already current"), "stdout: {stdout}");
    assert!(stdout.contains("staging"), "stdout: {stdout}");
}

fn assert_only_stable_was_requested(requests: &Arc<Mutex<Vec<String>>>) {
    let requests = requests.lock().expect("read requests");
    assert!(
        !requests.is_empty(),
        "expected at least one release request"
    );
    assert!(
        requests
            .iter()
            .all(|path| path.contains("/latest/download/")),
        "unexpected non-stable request: {requests:?}"
    );
}

#[dialog_common::test]
fn it_defaults_to_stable_without_a_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (endpoint, requests) = serve_recording(vec![]);

    let output = run_update(&endpoint, dir.path(), &[]);
    assert!(!output.status.success());
    assert_only_stable_was_requested(&requests);
}

#[dialog_common::test]
fn it_defaults_to_stable_for_an_unknown_receipt_channel() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "nightly", &running_install_dir(), "abc1234def");
    let (endpoint, requests) = serve_recording(vec![]);

    let output = run_update(&endpoint, dir.path(), &[]);
    assert!(!output.status.success());
    assert_only_stable_was_requested(&requests);
}

#[dialog_common::test]
fn it_ignores_a_receipt_for_a_different_install_dir_when_selecting_channel() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(
        dir.path(),
        "staging",
        "/some/other/install/dir",
        "abc1234def",
    );
    let (endpoint, requests) = serve_recording(vec![]);

    let output = run_update(&endpoint, dir.path(), &[]);
    assert!(!output.status.success());
    assert_only_stable_was_requested(&requests);
}

#[dialog_common::test]
fn it_rejects_a_manifest_for_a_different_channel() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "stable", &running_install_dir(), "abc1234def");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.4.0", "abc1234def", "staging"),
    )]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("manifest says channel staging") && stderr.contains("selected stable"),
        "stderr: {stderr}"
    );
}

#[dialog_common::test]
fn it_fails_loudly_when_the_channel_has_no_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Empty route table: every request 404s.
    let endpoint = serve(vec![]);

    let output = run_update(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(stderr.contains("no manifest.json"), "stderr: {stderr}");
}

#[dialog_common::test]
fn it_toggles_the_background_check_without_touching_the_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Unroutable endpoint: proves the toggle never fetches.
    let endpoint = "http://127.0.0.1:1/releases";

    let output = run_update(endpoint, dir.path(), &["--disable-check"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("disabled"));

    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    assert!(state.contains("\"check_enabled\": false"), "state: {state}");

    let output = run_update(endpoint, dir.path(), &["--enable-check"]);
    assert!(output.status.success());
    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    assert!(state.contains("\"check_enabled\": true"), "state: {state}");
}

/// Run a command that reaches main's exit path with no repo and no
/// network of its own, so the nag runs against a controlled cache.
///
/// NOT `--version`: clap handles that inside `Cli::parse()` and exits
/// before main's tail ever runs, so the check and nag would never run
/// and these tests would pass while proving nothing. `tonk telemetry`
/// (status) needs no repo, prints two predictable lines, and exits
/// through the tail. Without `TONK_POSTHOG_KEY` it sends nothing.
fn run_probe(
    endpoint: &str,
    state_dir: &std::path::Path,
    extra: &[(&str, &str)],
) -> std::process::Output {
    let mut cmd = Command::new(binary());
    cmd.arg("telemetry")
        .env("TONK_UPDATE_ENDPOINT", endpoint)
        .env("TONK_UPDATE_STATE", state_dir)
        // Keep `tonk telemetry` off the real telemetry.json too.
        .env("TONK_TELEMETRY_STATE", state_dir)
        .env_remove("CI")
        .env("TONK_CHANNEL", "stable")
        .env_remove("TONK_NO_UPDATE_CHECK")
        .env_remove("TONK_POSTHOG_KEY");
    for (key, value) in extra {
        cmd.env(key, value);
    }
    cmd.output().expect("run tonk telemetry")
}

#[dialog_common::test]
fn it_nags_on_stderr_when_the_release_is_newer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (endpoint, requests) = serve_recording(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stderr.contains("99.0.0 is available"), "stderr: {stderr}");
    assert!(stderr.contains("run `tonk update`"), "stderr: {stderr}");
    // stdout is parsed by agents and must stay clean.
    assert!(!stdout.contains("is available"), "stdout: {stdout}");
    assert_only_stable_was_requested(&requests);
}

#[dialog_common::test]
fn it_checks_staging_in_the_background_for_a_matching_staging_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "staging", &running_install_dir(), "abc1234def");
    let (endpoint, requests) = serve_recording(vec![(
        "/releases/download/tonk-staging/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "staging"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(output.status.success());
    let requests = requests.lock().expect("read requests");
    assert_eq!(
        requests.as_slice(),
        ["/releases/download/tonk-staging/manifest.json"]
    );
}

#[dialog_common::test]
fn it_checks_stable_in_the_background_for_a_matching_stable_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "stable", &running_install_dir(), "abc1234def");
    let (endpoint, requests) = serve_recording(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(output.status.success());
    assert_only_stable_was_requested(&requests);
}

#[dialog_common::test]
fn it_refreshes_and_does_not_nag_with_cached_data_from_another_channel() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "stable", &running_install_dir(), "abc1234def");
    std::fs::write(
        dir.path().join("update.json"),
        r#"{
  "check_enabled": true,
  "channel": "staging",
  "last_checked_at": "2099-01-01T00:00:00Z",
  "last_nagged_at": null,
  "latest_version": "99.0.0",
  "latest_commit": "fff9999"
}"#,
    )
    .expect("write stale channel cache");
    let (endpoint, requests) = serve_recording(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.0.1", "aaa0001", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
    assert_only_stable_was_requested(&requests);

    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    let parsed: serde_json::Value = serde_json::from_str(&state).expect("parse state");
    assert_eq!(parsed["channel"], "stable", "state: {state}");
    assert_eq!(parsed["latest_version"], "0.0.1", "state: {state}");
    assert_eq!(parsed["latest_commit"], "aaa0001", "state: {state}");
}

#[dialog_common::test]
fn it_does_not_cache_a_background_manifest_for_a_different_channel() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_receipt(dir.path(), "stable", &running_install_dir(), "abc1234def");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "staging"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));

    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    let parsed: serde_json::Value = serde_json::from_str(&state).expect("parse state");
    assert!(parsed["latest_version"].is_null(), "state: {state}");
    assert!(parsed["latest_commit"].is_null(), "state: {state}");
    assert!(!parsed["last_checked_at"].is_null(), "state: {state}");
}

#[dialog_common::test]
fn it_does_not_nag_when_the_release_is_not_newer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.0.1", "aaa0001", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
}

#[dialog_common::test]
fn it_does_not_nag_when_opted_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[("TONK_NO_UPDATE_CHECK", "1")]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
    // Opting out must not even leave state behind.
    assert!(!dir.path().join("update.json").exists());
}

#[dialog_common::test]
fn it_does_not_nag_in_ci() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999", "stable"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[("CI", "true")]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
}

#[dialog_common::test]
fn it_stays_silent_and_succeeds_when_the_check_cannot_reach_the_release() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Nothing listening: the check must fail invisibly.
    let output = run_probe("http://127.0.0.1:1/releases", dir.path(), &[]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("error"), "stderr: {stderr}");
    assert!(!stderr.contains("is available"), "stderr: {stderr}");
    // last_checked_at must actually advance, so an offline machine backs
    // off to a daily retry instead of re-checking on every command. Parse
    // rather than substring-match: serde always emits the key, so
    // `contains("last_checked_at")` would pass even when it is null.
    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    let parsed: serde_json::Value = serde_json::from_str(&state).expect("parse state");
    assert!(
        !parsed["last_checked_at"].is_null(),
        "last_checked_at should have advanced despite the failed check: {state}"
    );
}
