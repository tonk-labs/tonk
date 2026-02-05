//! UCAN access service test server.
//!
//! This module provides a local UCAN access service for integration testing.
//! It implements the same handler logic as the Cloudflare Worker but runs
//! as a native HTTP server with CORS support for browser-based testing.

use super::AccessServiceAddress;
use dialog_common::helpers::{Provider, Service};
use dialog_s3_credentials::ucan::UcanAuthorizer;
use dialog_s3_credentials::{Address, s3};
use dialog_storage::s3::helpers::LocalS3;
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_TYPE,
};
use hyper::server::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

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
    ) -> anyhow::Result<Self> {
        // Create S3 credentials for the authorizer
        let address = Address::new(&s3_server.endpoint, "us-east-1", bucket);
        let s3_credentials =
            s3::Credentials::private(address, access_key, secret_key)?.with_path_style(true);

        // Create UcanAuthorizer - the core of our service
        let authorizer = Arc::new(RwLock::new(UcanAuthorizer::new(s3_credentials)));

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let endpoint = format!("http://{}", addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let authorizer_clone = authorizer.clone();
        let server_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let authorizer = authorizer_clone.clone();
                            tokio::spawn(async move {
                                let service = hyper::service::service_fn(move |req| {
                                    let authorizer = authorizer.clone();
                                    async move {
                                        handle_request(req, authorizer).await
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
/// - OPTIONS /ucan/ → CORS preflight
async fn handle_request(
    req: Request<Incoming>,
    authorizer: Arc<RwLock<UcanAuthorizer>>,
) -> Result<Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible> {
    use bytes::Bytes;
    use http_body_util::Full;

    // Handle CORS preflight
    if req.method() == Method::OPTIONS {
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Full::new(Bytes::new()))
                .unwrap(),
        ));
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

/// Add CORS headers to a response.
fn cors_response<T>(mut response: Response<T>) -> Response<T> {
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        "POST, OPTIONS".parse().unwrap(),
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
}

impl Default for AccessServiceSettings {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret-key".to_string(),
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
