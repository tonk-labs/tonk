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

fn isolated_command(home: &std::path::Path) -> Command {
    let mut command = Command::new(binary());
    command
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
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT");
    command
}

fn spawn_link(home: &std::path::Path) -> Child {
    isolated_command(home)
        .args(["account", "login", "--no-open"])
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

/// Logout is complete before provider cleanup is attempted. A different CLI
/// process must therefore be able to retry the signed intent and retire it
/// without reconstructing authority that logout deliberately cleared.
#[test]
fn offline_logout_cleanup_survives_a_process_restart() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
    use dialog_capability::Subject;
    use dialog_effects::storage::Directory;
    use dialog_operator::{DeriveOperator as _, Profile};
    use dialog_storage::provider::storage::{NativeSpace, Storage};

    #[derive(Clone)]
    struct DetachState {
        online: Arc<AtomicBool>,
        attempts: Arc<AtomicUsize>,
    }

    async fn detach(
        State(state): State<DetachState>,
    ) -> Result<Json<serde_json::Value>, StatusCode> {
        state.attempts.fetch_add(1, Ordering::SeqCst);
        if !state.online.load(Ordering::SeqCst) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(Json(serde_json::json!({ "outcome": "detached" })))
    }

    fn profile_base(home: &std::path::Path) -> std::path::PathBuf {
        #[cfg(target_os = "macos")]
        {
            home.join("Library/Application Support/dialog")
        }
        #[cfg(not(target_os = "macos"))]
        {
            home.join("data/dialog")
        }
    }

    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    runtime.block_on(async {
        let temp = tempfile::tempdir().expect("temporary profile");
        let home = temp.path();
        let store = tonk_cli::space::SpaceStore::at(home.join("spaces"));
        std::fs::create_dir_all(store.account_dir()).expect("account state directory");
        let sentinel = store.root().join("local-space-sentinel");
        std::fs::write(&sentinel, b"local work").expect("local-space sentinel");

        let online = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("detach listener");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/devices/detach", post(detach))
            .with_state(DetachState {
                online: online.clone(),
                attempts: attempts.clone(),
            });
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("detach server");
        });

        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(tonk_cli::site::PROFILE_NAME)
            .at(Directory::At(
                profile_base(home).to_string_lossy().into_owned(),
            ))
            .perform(&storage)
            .await
            .expect("isolated CLI profile");
        let account_dir = store
            .account_dir()
            .canonicalize()
            .expect("canonical account directory");
        let operator = profile
            .derive(b"tonk/account-state/v1")
            .allow(Subject::any())
            .base(Directory::At(account_dir.to_string_lossy().into_owned()))
            .build(storage)
            .await
            .expect("account operator");
        let root = dialog_credentials::Ed25519Signer::generate()
            .await
            .expect("account root");
        let link = tonk_identity::delegation::mint_device_delegation(root.clone(), &profile.did())
            .await
            .expect("account grant");
        let descriptor =
            tonk_account::AccountRepositoryDescriptorV1::sign(&root, "http://127.0.0.1:9/ucan/")
                .await
                .expect("account descriptor");
        let attached_at = 1;
        tonk_cli::account::attach_exact_for_process_test(
            &profile,
            &operator,
            &store,
            &endpoint,
            "fixture-credential",
            link,
            descriptor.bytes(),
            "process-restart-generation",
            attached_at,
        )
        .await
        .expect("activate exact account generation");
        drop(operator);
        drop(profile);

        let logout = isolated_command(home)
            .args(["account", "logout"])
            .output()
            .expect("run offline logout");
        assert!(
            logout.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&logout.stderr)
        );
        let stdout = String::from_utf8_lossy(&logout.stdout);
        let stderr = String::from_utf8_lossy(&logout.stderr);
        assert!(
            stdout.contains("local identity, spaces, and edits remain"),
            "stdout: {stdout}"
        );
        assert!(
            stderr.contains("Provider cleanup is queued"),
            "stderr: {stderr}"
        );
        assert!(stderr.contains("tonk account status"), "stderr: {stderr}");
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"local work");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        // A fresh process reaches the same profile/store after the provider
        // returns. Its status boundary drains the durable outbox.
        online.store(true, Ordering::SeqCst);
        let status = isolated_command(home)
            .args(["account", "status"])
            .output()
            .expect("run restarted status");
        assert!(
            status.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

        let state_file = std::fs::read_dir(store.account_dir())
            .expect("account state entries")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(tonk_cli::account::ACCOUNT_SESSION_SITE)
                            && name.ends_with(".json")
                    })
            })
            .expect("account-session state file");
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state_file).unwrap()).unwrap();
        assert_eq!(state["version"], 2);
        assert_eq!(state["pending_detaches"], serde_json::json!([]));
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"local work");
        server.abort();
    });
}
