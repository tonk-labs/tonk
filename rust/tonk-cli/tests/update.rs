//! End-to-end `tonk update`: the real binary against a fake release
//! served from a local listener.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::process::Command;

/// Serve a fixed set of `path -> body` responses until dropped.
/// Anything unmapped gets a 404, which is how the "missing manifest"
/// case is exercised.
fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
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
    format!("http://127.0.0.1:{port}/releases")
}

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
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
        .env_remove("TONK_CHANNEL")
        .env_remove("TONK_POSTHOG_KEY")
        .output()
        .expect("run tonk update")
}

fn manifest_body(version: &str, commit: &str) -> Vec<u8> {
    format!(
        r#"{{"version":"{version}","commit":"{commit}","channel":"stable","built_at":"2026-07-16T00:00:00Z"}}"#
    )
    .into_bytes()
}

#[dialog_common::test]
fn it_reports_already_current_when_the_receipt_matches_the_release() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("install.json"),
        r#"{"channel":"stable","version":"0.4.0","commit":"abc1234def","install_dir":"/usr/local/bin","installed_at":"2026-07-16T00:00:00Z"}"#,
    )
    .expect("write receipt");

    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.4.0", "abc1234def"),
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
        .env_remove("TONK_CHANNEL")
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
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stderr.contains("99.0.0 is available"), "stderr: {stderr}");
    assert!(stderr.contains("run `tonk update`"), "stderr: {stderr}");
    // stdout is parsed by agents and must stay clean.
    assert!(!stdout.contains("is available"), "stdout: {stdout}");
}

#[dialog_common::test]
fn it_does_not_nag_when_the_release_is_not_newer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("0.0.1", "aaa0001"),
    )]);

    let output = run_probe(&endpoint, dir.path(), &[]);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("is available"));
}

#[dialog_common::test]
fn it_does_not_nag_when_opted_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = serve(vec![(
        "/releases/latest/download/manifest.json".to_owned(),
        manifest_body("99.0.0", "fff9999"),
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
        manifest_body("99.0.0", "fff9999"),
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
    // last_checked_at still advanced, so an offline machine backs off.
    let state = std::fs::read_to_string(dir.path().join("update.json")).expect("state");
    assert!(state.contains("last_checked_at"), "state: {state}");
}
