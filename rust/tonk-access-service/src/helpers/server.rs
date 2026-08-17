//! UCAN access service test server.
//!
//! This module provides a local UCAN access service for integration testing.
//! It implements the same handler logic as the Cloudflare Worker but runs
//! as a native HTTP server with CORS support for browser-based testing.

use super::AccessServiceAddress;
use crate::shortcut::{Shortcut, object_key_for, requested_ttl, unavailable_invite_html};
use dialog_common::helpers::{Provider, Service};
use dialog_remote_s3::helpers::LocalS3;
use dialog_remote_s3::{Address, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE, CACHE_CONTROL, CONTENT_TYPE,
    HeaderValue, LOCATION,
};
use hyper::server::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

/// In-memory shortcut store: object key → (unix-seconds expiry, target).
type Shortcuts = Arc<RwLock<HashMap<String, (u64, String)>>>;

/// A running UCAN access service test server instance.
pub struct AccessServer {
    /// The endpoint URL where the access service is listening
    pub endpoint: String,
    /// The backing S3 server
    pub s3_server: LocalS3,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
}

impl AccessServer {
    /// Start a UCAN access service backed by a local S3 server.
    ///
    /// # Arguments
    ///
    /// * `s3_server` - A running LocalS3 server instance
    /// * `bucket` - The bucket name to use
    /// * `access_key` - AWS access key ID for S3 authentication
    /// * `secret_key` - AWS secret access key for S3 authentication
    pub async fn start(
        s3_server: LocalS3,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        deployment: Option<tonk_worker_api::DeploymentConfig>,
    ) -> anyhow::Result<Self> {
        // Create S3 credentials for the authorizer
        let address = Address::builder(&s3_server.endpoint)
            .region("us-east-1")
            .bucket(bucket)
            .path_style(true)
            .build()?;

        let credential = S3Credential::new(access_key, secret_key);

        // Create UcanAuthorizer - the core of our service
        let authorizer = Arc::new(RwLock::new(UcanAuthorizer::new(address, Some(credential))));

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let endpoint = format!("http://{}", addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let shortcuts: Shortcuts = Arc::new(RwLock::new(HashMap::new()));
        let deployment = Arc::new(deployment);
        let authorizer_clone = authorizer.clone();
        let server_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let authorizer = authorizer_clone.clone();
                            let shortcuts = shortcuts.clone();
                            let deployment = deployment.clone();
                            tokio::spawn(async move {
                                let service = hyper::service::service_fn(move |req| {
                                    let authorizer = authorizer.clone();
                                    let shortcuts = shortcuts.clone();
                                    let deployment = deployment.clone();
                                    async move {
                                        handle_request(req, authorizer, shortcuts, deployment).await
                                    }
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

        Ok(AccessServer {
            endpoint,
            s3_server,
            shutdown_tx,
            server_handle,
        })
    }
}

/// Handle an incoming UCAN access service request.
///
/// This implements the same logic as the Cloudflare Worker handler:
/// - POST /ucan/ → Authorize UCAN and return presigned URL
/// - PUT /@ → Store a shortcut target, respond with its hash
/// - GET /@/{hash} → Permanent relative redirect to the stored target
/// - GET /.well-known/tonk → Deployment configuration, when configured
/// - OPTIONS → CORS preflight
async fn handle_request(
    req: Request<Incoming>,
    authorizer: Arc<RwLock<UcanAuthorizer>>,
    shortcuts: Shortcuts,
    deployment: Arc<Option<tonk_worker_api::DeploymentConfig>>,
) -> Result<Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible> {
    use bytes::Bytes;
    use http_body_util::Full;

    // Handle CORS preflight. Like the Worker handlers, the preflight
    // carries its own cache lifetime; only the preflight can be cached.
    if req.method() == Method::OPTIONS {
        let mut response = cors_response(
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Full::new(Bytes::new()))
                .unwrap(),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static(crate::PREFLIGHT_MAX_AGE),
        );
        return Ok(response);
    }

    if req.method() == Method::GET && req.uri().path() == "/.well-known/tonk" {
        let response = match deployment.as_ref() {
            Some(config) => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(config).expect("deployment config serializes"),
                )))
                .unwrap(),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("Not Found")))
                .unwrap(),
        };
        return Ok(cors_response(response));
    }
    if req.method() == Method::PUT && req.uri().path() == "/@" {
        return Ok(cors_response(store_shortcut(req, shortcuts).await));
    }
    if req.method() == Method::GET
        && let Some(hash) = req.uri().path().strip_prefix("/@/")
    {
        return Ok(cors_response(serve_shortcut(hash, shortcuts).await));
    }

    // Only accept POST requests to /ucan/
    if req.method() != Method::POST {
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Full::new(Bytes::from("Method not allowed")))
                .unwrap(),
        ));
    }

    // Read request body
    use http_body_util::BodyExt;
    let body_bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            return Ok(cors_response(
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from(format!(
                        "Failed to read body: {}",
                        e
                    ))))
                    .unwrap(),
            ));
        }
    };

    // Authorize the UCAN container using UcanAuthorizer
    let authorizer = authorizer.read().await;
    match authorizer.authorize(&body_bytes).await {
        Ok(descriptor) => {
            // Serialize the AuthorizedRequest as CBOR
            match serde_ipld_dagcbor::to_vec(&descriptor) {
                Ok(cbor_bytes) => Ok(cors_response(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/cbor")
                        .body(Full::new(Bytes::from(cbor_bytes)))
                        .unwrap(),
                )),
                Err(e) => Ok(cors_response(
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from(format!(
                            "Failed to encode response: {}",
                            e
                        ))))
                        .unwrap(),
                )),
            }
        }
        Err(e) => Ok(cors_response(
            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Full::new(Bytes::from(format!(
                    "Authorization failed: {}",
                    e
                ))))
                .unwrap(),
        )),
    }
}

/// Current time as unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is past the epoch")
        .as_secs()
}

/// PUT /@ → validate and store a shortcut target, mirroring the
/// Cloudflare Worker handler over an in-memory store.
async fn store_shortcut(
    req: Request<Incoming>,
    shortcuts: Shortcuts,
) -> Response<http_body_util::Full<bytes::Bytes>> {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};

    let ttl = match requested_ttl(req.uri().query()) {
        Ok(ttl) => ttl,
        Err(reason) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(reason)))
                .unwrap();
        }
    };
    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(format!("Failed to read body: {e}"))))
                .unwrap();
        }
    };

    match Shortcut::new(&body) {
        Ok(shortcut) => {
            let hash = shortcut.hash_str();
            shortcuts
                .write()
                .await
                .insert(shortcut.object_key(), (unix_now() + ttl, shortcut.target));
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/plain")
                .body(Full::new(Bytes::from(hash)))
                .unwrap()
        }
        Err(reason) => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from(reason)))
            .unwrap(),
    }
}

/// GET /@/{hash} → permanent relative redirect to the stored target.
async fn serve_shortcut(
    hash: &str,
    shortcuts: Shortcuts,
) -> Response<http_body_util::Full<bytes::Bytes>> {
    use bytes::Bytes;
    use http_body_util::Full;

    let key = match object_key_for(hash) {
        Ok(key) => key,
        Err(reason) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::from(reason)))
                .unwrap();
        }
    };

    let not_found = || {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .header(CACHE_CONTROL, "no-store")
            .body(Full::new(Bytes::from(unavailable_invite_html())))
            .unwrap()
    };
    match shortcuts.read().await.get(&key) {
        Some((expires, target)) => {
            let remaining = expires.saturating_sub(unix_now());
            if remaining == 0 {
                return not_found();
            }
            Response::builder()
                .status(StatusCode::MOVED_PERMANENTLY)
                .header(LOCATION, target)
                .header(
                    CACHE_CONTROL,
                    format!("public, max-age={}", remaining.min(86_400)),
                )
                .body(Full::new(Bytes::new()))
                .unwrap()
        }
        None => not_found(),
    }
}

/// Add CORS headers to a response.
fn cors_response<T>(mut response: Response<T>) -> Response<T> {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        "GET, PUT, POST, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type".parse().unwrap(),
    );
    headers.insert(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        "Content-Type".parse().unwrap(),
    );
    response
}

#[async_trait::async_trait]
impl Provider for AccessServer {
    async fn stop(self) -> anyhow::Result<()> {
        // Send shutdown signal - ignore error if receiver is already dropped
        let _ = self.shutdown_tx.send(());
        // Wait for the server task to complete
        let _ = self.server_handle.await;
        self.s3_server.stop().await
    }
}

/// Settings for configuring the UCAN access service test server.
#[derive(Debug, Clone)]
pub struct AccessServiceSettings {
    /// The bucket name to create. Defaults to "test-bucket".
    pub bucket: String,
    /// AWS access key ID. Defaults to "test-access-key".
    pub access_key_id: String,
    /// AWS secret access key. Defaults to "test-secret-key".
    pub secret_access_key: String,
    /// Served from `GET /.well-known/tonk` when set; 404 otherwise.
    pub deployment: Option<tonk_worker_api::DeploymentConfig>,
}

impl Default for AccessServiceSettings {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret-key".to_string(),
            deployment: None,
        }
    }
}

/// Provider function for AccessServiceAddress.
///
/// Starts both an S3 server and a UCAN access service.
#[dialog_common::provider]
pub async fn access_service(
    settings: AccessServiceSettings,
) -> anyhow::Result<Service<AccessServiceAddress, AccessServer>> {
    let bucket = if settings.bucket.is_empty() {
        "test-bucket"
    } else {
        &settings.bucket
    };

    // Start the S3 server
    let s3_server = LocalS3::start_with_auth(
        &settings.access_key_id,
        &settings.secret_access_key,
        &[bucket],
    )
    .await?;

    let s3_endpoint = s3_server.endpoint.clone();

    // Start the UCAN access service
    let access_server = AccessServer::start(
        s3_server,
        bucket,
        &settings.access_key_id,
        &settings.secret_access_key,
        settings.deployment,
    )
    .await?;

    let address = AccessServiceAddress {
        access_service_url: access_server.endpoint.clone(),
        s3_endpoint,
        bucket: bucket.to_string(),
        access_key_id: settings.access_key_id,
        secret_access_key: settings.secret_access_key,
    };

    Ok(Service::new(address, access_server))
}
