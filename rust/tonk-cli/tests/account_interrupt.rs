//! Process-level coverage for interrupting the account link wait.
//!
//! Linking binds a loopback callback and waits for the browser to post
//! the grant back; nothing is registered anywhere in between. These
//! tests pin the wait's two escape hatches: Ctrl-C cancels cleanly, and
//! a fresh run binds a fresh callback rather than resuming anything.

#![cfg(unix)]

use std::io::{BufRead as _, Read as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

fn spawn_link(home: &std::path::Path) -> Child {
    Command::new(binary())
        .args(["account", "login", "--no-open"])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPACES_STATE", home.join("spaces"))
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("TONK_TELEMETRY")
        .env_remove("TONK_SPACE")
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start tonk account login")
}

/// The approval URL the command prints, without blocking the test forever
/// if it never gets one. Printing it means the loopback callback is bound
/// and the command is inside its wait.
fn approval_url(child: &mut Child) -> String {
    let stdout = child.stdout.take().expect("piped stdout");
    let (url_tx, url_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if line.starts_with("http") {
                let _ = url_tx.send(line);
                return;
            }
        }
    });
    match url_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(url) => url,
        Err(error) => {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("piped stderr")
                .read_to_string(&mut stderr)
                .expect("read stderr");
            panic!("account link prints an approval URL: {error}; stderr: {stderr}")
        }
    }
}

/// SIGINT a command already inside its callback wait and return its stderr.
fn interrupt(child: &mut Child) -> String {
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success(), "kill -INT failed with {signal}");
    let status = wait_for_exit(child, Duration::from_secs(5)).unwrap_or_else(|| {
        child.kill().expect("kill stuck tonk process");
        child.wait().expect("reap stuck tonk process");
        panic!("tonk account login stayed alive after SIGINT");
    });
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("piped stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(4), "stderr: {stderr}");
    stderr
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("inspect tonk process") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn account_link_can_be_interrupted_during_the_callback_wait() {
    let temp = tempfile::tempdir().expect("temporary profile");
    let mut child = spawn_link(temp.path());

    let url = approval_url(&mut child);
    assert!(
        url.contains("callback=") && url.contains("audience="),
        "approval URL carries the loopback callback and audience: {url}"
    );
    // The URL only prints once the callback listener is bound, so the
    // command is inside its wait; give the ctrl_c handler a beat.
    std::thread::sleep(Duration::from_millis(100));
    let stderr = interrupt(&mut child);

    assert!(
        stderr.contains("account login cancelled"),
        "stderr: {stderr}"
    );
}

/// An interrupted link leaves nothing to resume: the next run binds its
/// own fresh callback and prints a URL of its own.
#[test]
fn account_link_starts_fresh_after_an_interrupt() {
    let temp = tempfile::tempdir().expect("temporary profile");

    let mut first = spawn_link(temp.path());
    let first_url = approval_url(&mut first);
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&mut first);

    let mut second = spawn_link(temp.path());
    let second_url = approval_url(&mut second);
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&mut second);

    assert_ne!(
        first_url, second_url,
        "each attempt binds its own loopback callback"
    );
}
