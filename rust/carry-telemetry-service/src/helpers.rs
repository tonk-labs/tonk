//! Test helpers for the carry telemetry service.
//!
//! Provides a local HTTP server that mirrors the Cloudflare Worker behavior
//! but runs natively, storing received pings in memory for test assertions.

use crate::Ping;
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_MAX_AGE,
};
use hyper::server::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// A recorded telemetry ping for test assertions.
#[derive(Debug, Clone)]
pub struct RecordedPing {
    pub id: String,
    pub command: String,
    pub version: String,
}

/// A running telemetry test server that records pings in memory.
pub struct TelemetryTestServer {
    pub endpoint: String,
    pings: Arc<Mutex<Vec<RecordedPing>>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
}

impl TelemetryTestServer {
    /// Start a local telemetry test server on a random port.
    pub async fn start() -> anyhow::Result<Self> {
        let pings: Arc<Mutex<Vec<RecordedPing>>> = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let endpoint = format!("http://{}", addr);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let pings_clone = pings.clone();
        let server_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let pings = pings_clone.clone();
                            tokio::spawn(async move {
                                let service = hyper::service::service_fn(move |req| {
                                    let pings = pings.clone();
                                    async move { handle_request(req, pings).await }
                                });
                                let _ = http1::Builder::new()
                                    .serve_connection(TokioIo::new(stream), service)
                                    .await;
                            });
                        }
                    }
                }
            }
        });

        Ok(Self {
            endpoint,
            pings,
            shutdown_tx,
            server_handle,
        })
    }

    /// Get all recorded pings.
    pub async fn recorded_pings(&self) -> Vec<RecordedPing> {
        self.pings.lock().await.clone()
    }

    /// Clear all recorded pings.
    pub async fn clear(&self) {
        self.pings.lock().await.clear();
    }

    /// Shut down the server.
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.server_handle.await;
    }
}

async fn handle_request(
    req: Request<Incoming>,
    pings: Arc<Mutex<Vec<RecordedPing>>>,
) -> Result<Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible> {
    use bytes::Bytes;
    use http_body_util::Full;

    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // GET /health
    if method == Method::GET && path == "/health" {
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("OK")))
            .unwrap());
    }

    // OPTIONS /ping (CORS preflight)
    if method == Method::OPTIONS && path == "/ping" {
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(ACCESS_CONTROL_ALLOW_METHODS, "POST, OPTIONS")
                .header(ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type")
                .header(ACCESS_CONTROL_MAX_AGE, "86400")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        ));
    }

    // POST /ping
    if method == Method::POST && path == "/ping" {
        use http_body_util::BodyExt;
        let body_bytes = match req.into_body().collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from("Bad request")))
                    .unwrap());
            }
        };

        let ping: Ping = match serde_json::from_slice(&body_bytes) {
            Ok(p) => p,
            Err(_) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from("Bad request")))
                    .unwrap());
            }
        };

        if crate::validate_ping(&ping).is_err() {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from("Bad request")))
                .unwrap());
        }

        pings.lock().await.push(RecordedPing {
            id: ping.id,
            command: ping.command,
            version: ping.version,
        });

        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("OK")))
                .unwrap(),
        ));
    }

    // 404 for everything else
    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from("Not found")))
        .unwrap())
}

fn cors_response<T>(mut response: Response<T>) -> Response<T> {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    response
}
