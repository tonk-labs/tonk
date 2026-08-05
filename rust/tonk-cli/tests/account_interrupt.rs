//! Process-level coverage for interrupting account handoff waits.

#![cfg(unix)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(Into::into)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into())
}

fn read_path(stream: &mut TcpStream) -> String {
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
            return headers
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path")
                .to_string();
        }
    }
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
    let mut child = Command::new(binary())
        .args([
            "account",
            "link",
            "--no-open",
            "--service-url",
            &service_url,
            "--account-url",
            "http://127.0.0.1/account/link",
        ])
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("TONK_SPOTS_STATE", temp.path().join("spots"))
        .env("TONK_TELEMETRY_STATE", temp.path().join("telemetry"))
        .env("TONK_UPDATE_STATE", temp.path().join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("TONK_TELEMETRY")
        .env_remove("TONK_SPOT")
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start tonk account link");

    pending
        .recv_timeout(Duration::from_secs(10))
        .expect("account link reaches a pending poll");
    // The command starts a 500 ms backoff after receiving the pending
    // response. Place SIGINT inside that gap rather than racing the response.
    std::thread::sleep(Duration::from_millis(100));
    let signal = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(signal.success(), "kill -INT failed with {signal}");

    let Some(status) = wait_for_exit(&mut child, Duration::from_secs(2)) else {
        child.kill().expect("kill stuck tonk process");
        child.wait().expect("reap stuck tonk process");
        panic!("tonk account link stayed alive after SIGINT during poll backoff");
    };
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("piped stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(4), "stderr: {stderr}");
    assert!(
        stderr.contains("account link cancelled"),
        "stderr: {stderr}"
    );
}
