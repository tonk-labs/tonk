//! Process-level coverage for interrupting account handoff waits.

#![cfg(unix)]

use std::io::{BufRead as _, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = stream.read(&mut buffer).expect("read request");
        request.extend_from_slice(&buffer[..count]);
        let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..headers_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= headers_end + 4 + content_length {
            let path = headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path")
                .to_string();
            let body = String::from_utf8_lossy(&request[headers_end + 4..]).into_owned();
            return (path, body);
        }
    }
}

fn read_path(stream: &mut TcpStream) -> String {
    read_request(stream).0
}

fn pending_link_server() -> (String, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind account service");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("account service address")
    );
    let (pending_tx, pending_rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut reported_pending = false;
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { return };
            let path = read_path(&mut stream);
            let status = match path.as_str() {
                "/links" => "200 OK",
                "/links/consume" => "202 Accepted",
                other => panic!("unexpected account-service request {other}"),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            )
            .expect("write response");
            stream.flush().expect("flush response");
            if path == "/links/consume" && !reported_pending {
                reported_pending = true;
                pending_tx.send(()).expect("report pending poll");
            }
        }
    });
    (endpoint, pending_rx)
}

/// A service that treats a link token the way a real one does: `tokenHash`
/// is created once and every repeat is a conflict. Reports each accepted
/// creation and each consume poll so a test can act at a known point.
fn single_use_link_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind account service");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("account service address")
    );
    let (events_tx, events_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut issued: Vec<String> = Vec::new();
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { return };
            let (path, body) = read_request(&mut stream);
            let field = |name: &str| {
                serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| value[name].as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            let (status, payload) = match path.as_str() {
                "/links" => {
                    let token = field("tokenHash");
                    if issued.contains(&token) {
                        (
                            "409 Conflict",
                            r#"{"error":{"code":"CONFLICT","message":"conflicts with existing state"}}"#,
                        )
                    } else {
                        issued.push(token.clone());
                        let _ = events_tx.send(format!("create {token}"));
                        ("201 Created", "{}")
                    }
                }
                "/links/consume" => {
                    let _ = events_tx.send(format!("consume {}", field("secret")));
                    ("202 Accepted", r#"{"pending":true}"#)
                }
                other => panic!("unexpected account-service request {other}"),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{payload}",
                payload.len()
            )
            .expect("write response");
            stream.flush().expect("flush response");
        }
    });
    (endpoint, events_rx)
}

fn spawn_link(home: &std::path::Path, service_url: &str) -> Child {
    Command::new(binary())
        .args([
            "account",
            "link",
            "--no-open",
            "--service-url",
            service_url,
            "--account-url",
            "http://127.0.0.1/account/link",
        ])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPOTS_STATE", home.join("spots"))
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("TONK_TELEMETRY")
        .env_remove("TONK_SPOT")
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start tonk account link")
}

/// The handoff URL the command prints, without blocking the test forever if
/// it never gets one.
fn handoff_url(child: &mut Child) -> String {
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
    url_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("account link prints a handoff URL")
}

fn wait_for_event(events: &mpsc::Receiver<String>, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        let event = events
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("account service never saw {expected}"));
        if event == expected {
            return;
        }
    }
}

/// SIGINT a command already inside its poll wait and return its stderr.
fn interrupt(child: &mut Child) -> String {
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success(), "kill -INT failed with {signal}");
    let status = wait_for_exit(child, Duration::from_secs(5)).unwrap_or_else(|| {
        child.kill().expect("kill stuck tonk process");
        child.wait().expect("reap stuck tonk process");
        panic!("tonk account link stayed alive after SIGINT");
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
fn account_link_can_be_interrupted_during_poll_backoff() {
    let temp = tempfile::tempdir().expect("temporary profile");
    let (service_url, pending) = pending_link_server();
    let mut child = spawn_link(temp.path(), &service_url);

    pending
        .recv_timeout(Duration::from_secs(10))
        .expect("account link reaches a pending poll");
    // The command starts a 500 ms backoff after receiving the pending
    // response. Place SIGINT inside that gap rather than racing the response.
    std::thread::sleep(Duration::from_millis(100));
    let stderr = interrupt(&mut child);

    assert!(
        stderr.contains("account link cancelled"),
        "stderr: {stderr}"
    );
}

/// A cancelled handoff leaves its one-time token spent at the service. The
/// next attempt must start a fresh handoff rather than re-offering a token
/// the service will never accept again, which otherwise leaves the profile
/// unable to link at all until it logs out.
#[test]
fn account_link_restarts_a_handoff_the_service_will_not_recreate() {
    let temp = tempfile::tempdir().expect("temporary profile");
    let (service_url, events) = single_use_link_server();

    let mut first = spawn_link(temp.path(), &service_url);
    let first_url = handoff_url(&mut first);
    let first_secret = first_url.rsplit('#').next().expect("handoff secret");
    // SIGINT only counts once the command is inside its poll wait; before
    // that Tokio has not installed a handler and the default action applies.
    wait_for_event(&events, &format!("consume {first_secret}"));
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&mut first);

    let mut second = spawn_link(temp.path(), &service_url);
    let second_url = handoff_url(&mut second);
    let second_secret = second_url.rsplit('#').next().expect("handoff secret");
    wait_for_event(&events, &format!("consume {second_secret}"));
    std::thread::sleep(Duration::from_millis(100));
    let stderr = interrupt(&mut second);

    assert_ne!(first_url, second_url);
    assert!(
        stderr.contains("could not resume the pending handoff"),
        "stderr: {stderr}"
    );
}
