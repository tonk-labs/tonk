use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::{Link, Mode, Options, Shape};

/// A server that echoes every byte back, and its address.
async fn echo() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 64 * 1024];
                loop {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if stream.write_all(&buffer[..n]).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    address
}

async fn connect(link: &Link) -> TcpStream {
    TcpStream::connect(link.url().trim_start_matches("http://"))
        .await
        .unwrap()
}

/// Send `bytes` and read as many back, or fail after `limit`.
async fn round_trip(stream: &mut TcpStream, bytes: &[u8], limit: Duration) -> Option<Duration> {
    let began = Instant::now();
    stream.write_all(bytes).await.ok()?;
    let mut back = vec![0u8; bytes.len()];
    tokio::time::timeout(limit, stream.read_exact(&mut back))
        .await
        .ok()?
        .ok()?;
    Some(began.elapsed())
}

fn shaped(latency_ms: u64) -> Shape {
    Shape {
        latency: Duration::from_millis(latency_ms),
        ..Shape::default()
    }
}

#[tokio::test]
async fn a_round_trip_costs_the_latency_and_a_new_connection_one_more() {
    let link = Link::start(&echo().await, shaped(200)).await.unwrap();
    let mut stream = connect(&link).await;
    let first = round_trip(&mut stream, b"ping", Duration::from_secs(5))
        .await
        .expect("answered");
    let second = round_trip(&mut stream, b"ping", Duration::from_secs(5))
        .await
        .expect("answered");
    assert!(
        first >= Duration::from_millis(400) && first < Duration::from_millis(700),
        "handshake and round trip: {first:?}"
    );
    assert!(
        second >= Duration::from_millis(200) && second < Duration::from_millis(400),
        "round trip: {second:?}"
    );
}

#[tokio::test]
async fn a_stall_holds_bytes_until_the_link_opens_and_then_delivers_them() {
    let link = Link::start(&echo().await, Shape::default()).await.unwrap();
    let mut stream = connect(&link).await;
    round_trip(&mut stream, b"warm", Duration::from_secs(2))
        .await
        .expect("open link answers");

    link.set(Shape {
        mode: Mode::Stall,
        ..Shape::default()
    });
    assert!(
        round_trip(&mut stream, b"held", Duration::from_millis(500))
            .await
            .is_none(),
        "a stalled link delivers nothing"
    );
    link.set(Shape::default());
    let mut back = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut back))
        .await
        .expect("delivered once open")
        .unwrap();
    assert_eq!(&back, b"held");
}

#[tokio::test]
async fn going_offline_resets_open_connections_and_refuses_new_ones() {
    let link = Link::start(&echo().await, Shape::default()).await.unwrap();
    let mut stream = connect(&link).await;
    round_trip(&mut stream, b"warm", Duration::from_secs(2))
        .await
        .expect("open link answers");

    link.set(Shape {
        mode: Mode::Offline,
        ..Shape::default()
    });
    let mut buffer = [0u8; 4];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer))
        .await
        .expect("the reset arrives");
    assert!(
        matches!(read, Err(ref error) if error.kind() == std::io::ErrorKind::ConnectionReset)
            || matches!(read, Ok(0)),
        "{read:?}"
    );

    let mut fresh = connect(&link).await;
    let read = tokio::time::timeout(Duration::from_secs(2), fresh.read(&mut buffer))
        .await
        .expect("refused at once");
    assert!(!matches!(read, Ok(n) if n > 0), "{read:?}");
    assert_eq!(link.stats().refused, 1);
}

#[tokio::test]
async fn a_quiet_connection_is_forgotten_after_the_idle_cut() {
    let link = Link::start(
        &echo().await,
        Shape {
            idle_cut: Duration::from_millis(300),
            ..Shape::default()
        },
    )
    .await
    .unwrap();
    let mut busy = connect(&link).await;
    let mut quiet = connect(&link).await;
    round_trip(&mut quiet, b"warm", Duration::from_secs(2))
        .await
        .expect("answers while fresh");
    for _ in 0..6 {
        round_trip(&mut busy, b"beat", Duration::from_secs(2))
            .await
            .expect("a connection kept busy is kept");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(link.stats().forgotten, 1, "only the quiet one");
    assert!(
        round_trip(&mut quiet, b"late", Duration::from_millis(500))
            .await
            .is_none(),
        "nothing crosses a forgotten connection, and nothing says so"
    );
}

#[tokio::test]
async fn bandwidth_paces_a_large_answer() {
    let link = Link::start(
        &echo().await,
        Shape {
            kbps: 1_600,
            ..Shape::default()
        },
    )
    .await
    .unwrap();
    let mut stream = connect(&link).await;
    // 100 KB up and 100 KB down at 200 KB/s each way, pipelined: about
    // half a second, not instant.
    let took = round_trip(&mut stream, &vec![7u8; 100_000], Duration::from_secs(5))
        .await
        .expect("answered");
    assert!(
        took >= Duration::from_millis(450) && took < Duration::from_millis(1500),
        "{took:?}"
    );
}

#[tokio::test]
async fn each_http_exchange_is_logged_with_its_kind_and_timing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 1024];
        for _ in 0..2 {
            let _ = stream.read(&mut buffer).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .await
                .unwrap();
        }
    });
    let log = std::env::temp_dir().join(format!("tonk-link-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    let link = Link::start_with(
        Options {
            name: "s3".into(),
            listen: 0,
            upstream: format!("http://{upstream}"),
            shape: shaped(100),
        },
        Some(&log),
    )
    .await
    .unwrap();
    let mut stream = connect(&link).await;
    for request in [
        "GET /bucket/block HTTP/1.1\r\nhost: x\r\n\r\n",
        "PUT /bucket/cell HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\n\r\n",
    ] {
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = vec![0u8; 40];
        stream.read_exact(&mut answer).await.unwrap();
    }
    drop(stream);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let exchanges = link.exchanges();
    let kinds: Vec<&str> = exchanges.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["object-get", "object-put"]);
    for exchange in &exchanges {
        assert_eq!(exchange.status, 200);
        let waited = exchange.first_byte_ms.unwrap() - exchange.t0;
        assert!(waited >= 95, "the client waited a round trip: {waited} ms");
    }
    let logged = std::fs::read_to_string(&log).unwrap();
    assert_eq!(logged.lines().count(), 2);
    assert_eq!(link.stats().exchanges.get("object-get"), Some(&1));
    let _ = std::fs::remove_file(&log);
}

/// GET `target` from the control server at `address`, returning the body.
async fn control(address: &str, target: &str) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn the_gate_switches_the_network_around_the_runs_profile() {
    let page = Link::start_with(
        Options {
            name: "page".into(),
            listen: 0,
            upstream: echo().await,
            shape: Shape::profile("mobile").unwrap(),
        },
        None,
    )
    .await
    .unwrap();
    let ucan = Link::start_with(
        Options {
            name: "ucan".into(),
            listen: 0,
            upstream: echo().await,
            shape: Shape::profile("mobile").unwrap(),
        },
        None,
    )
    .await
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(super::serve_control(
        listener,
        vec![page.handle(), ucan.handle()],
    ));

    control(&address, "/__gate?mode=stall").await;
    assert_eq!(page.shape().mode, Mode::Stall);
    assert_eq!(page.shape().kbps, 8_000, "a stall keeps the path");

    control(&address, "/__gate?mode=latency&ms=2000").await;
    assert_eq!(ucan.shape().mode, Mode::Open);
    assert_eq!(ucan.shape().latency, Duration::from_secs(2));

    control(&address, "/__gate?mode=open").await;
    assert_eq!(
        ucan.shape(),
        Shape::profile("mobile").unwrap(),
        "open returns to the run's profile"
    );

    control(&address, "/profile?name=slow3g&link=ucan").await;
    control(&address, "/__gate?mode=open").await;
    assert_eq!(ucan.shape().name, "slow3g", "a profile becomes the base");
    assert_eq!(page.shape().name, "mobile", "only the named link changed");

    let stats: serde_json::Value =
        serde_json::from_str(&control(&address, "/stats").await).unwrap();
    assert_eq!(stats.as_array().map(Vec::len), Some(2));
    assert!(
        control(&address, "/profile?name=dialup")
            .await
            .contains("no profile")
    );
}

#[tokio::test]
async fn a_request_left_unanswered_is_listed_in_flight() {
    // A server that reads the request and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 1024];
        let _ = stream.read(&mut buffer).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
    let link = Link::start_with(
        Options {
            name: "ucan".into(),
            listen: 0,
            upstream,
            shape: Shape::default(),
        },
        None,
    )
    .await
    .unwrap();
    let mut stream = connect(&link).await;
    stream
        .write_all(b"POST /ucan/ HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\n\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let pending = link.handle().inflight();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].kind, "ucan");
    assert_eq!(pending[0].first_byte_ms, None, "never answered");
    assert!(link.exchanges().is_empty(), "nothing has ended");
}
