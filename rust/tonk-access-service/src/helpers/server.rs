//! UCAN access service test server.
//!
//! This module provides a local UCAN access service for integration testing.
//! It implements the same handler logic as the Cloudflare Worker but runs
//! as a native HTTP server with CORS support for browser-based testing.

use super::AccessServiceAddress;
use crate::email::{CapturedEmail, EmailError, EmailSender};
use crate::registration::{Registration, registration_command};
use crate::service::did_document;
use crate::shortcut::{Shortcut, object_key_for, requested_ttl, unavailable_invite_html};
use crate::store::sqlite::SqliteStore;
use async_trait::async_trait;
use dialog_common::helpers::{Provider, Service};
use dialog_credentials::Ed25519Signer;
use dialog_remote_s3::helpers::LocalS3;
use dialog_remote_s3::{Address, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;
use dialog_varsig::Principal;
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
    /// Activation emails captured instead of delivered.
    pub emails: Arc<CapturedEmail>,
    /// The service's signing DID, issuer of activation delegations.
    pub service_did: String,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
}

/// Everything the registration commands execute against, natively:
/// the in-memory control store, captured email, and a per-server
/// service signer.
struct RegistrationState {
    store: SqliteStore,
    emails: Arc<CapturedEmail>,
    sender: AnnouncedEmail,
    service: Ed25519Signer,
    origin: String,
}

/// Captures activation emails and announces them on stdout, so a human
/// driving a local server can complete sign-up: nothing is ever sent.
struct AnnouncedEmail(Arc<CapturedEmail>);

#[async_trait]
impl EmailSender for AnnouncedEmail {
    async fn send_activation(&self, email: &str, link: &str) -> Result<(), EmailError> {
        println!("ACCESS_ACTIVATION_EMAIL {email} {link}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        self.0.send_activation(email, link).await
    }
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
        public_origin: Option<String>,
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

        let emails = Arc::new(CapturedEmail::default());
        let service = Ed25519Signer::generate()
            .await
            .map_err(|err| anyhow::anyhow!("service signer: {err:?}"))?;
        let service_did = service.did().to_string();
        let registration = Arc::new(RegistrationState {
            store: SqliteStore::in_memory().map_err(|err| anyhow::anyhow!("{err}"))?,
            emails: emails.clone(),
            sender: AnnouncedEmail(emails.clone()),
            service,
            // Activation links open on the page origin, which behind a
            // dev proxy is not this server's own address.
            origin: public_origin.unwrap_or_else(|| endpoint.clone()),
        });

        let shortcuts: Shortcuts = Arc::new(RwLock::new(HashMap::new()));
        let deployment = Arc::new(deployment);
        let authorizer_clone = authorizer.clone();
        let registration_clone = registration.clone();
        let server_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let authorizer = authorizer_clone.clone();
                            let shortcuts = shortcuts.clone();
                            let deployment = deployment.clone();
                            let registration = registration_clone.clone();
                            tokio::spawn(async move {
                                let service = hyper::service::service_fn(move |req| {
                                    let authorizer = authorizer.clone();
                                    let shortcuts = shortcuts.clone();
                                    let deployment = deployment.clone();
                                    let registration = registration.clone();
                                    async move {
                                        handle_request(req, authorizer, shortcuts, deployment, registration).await
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
            emails,
            service_did,
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
    registration: Arc<RegistrationState>,
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
            Some(config) => {
                // The server owns its generated identity, so discovery
                // carries it without every caller having to thread it in.
                let mut config = config.clone();
                if config.service_did.is_none() {
                    config.service_did = Some(registration.service.did().to_string());
                }
                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(
                        serde_json::to_vec(&config).expect("deployment config serializes"),
                    )))
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from("Not Found")))
                .unwrap(),
        };
        return Ok(cors_response(response));
    }
    if req.method() == Method::GET && req.uri().path() == "/.well-known/did.json" {
        let host = req
            .uri()
            .authority()
            .map(ToString::to_string)
            .unwrap_or_default();
        let document = did_document(&host, &registration.service);
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&document).expect("did document serializes"),
                )))
                .unwrap(),
        ));
    }
    // Test-only inspection: activation emails are captured, never sent,
    // so integration tests read them back here.
    if req.method() == Method::GET && req.uri().path() == "/_test/emails" {
        let emails = registration
            .emails
            .0
            .lock()
            .expect("captured email mutex poisoned")
            .clone();
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&emails).expect("captured emails serialize"),
                )))
                .unwrap(),
        ));
    }
    if req.method() == Method::GET && req.uri().path() == "/_test/service" {
        let body = serde_json::json!({ "did": registration.service.did().to_string() });
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&body).expect("service did serializes"),
                )))
                .unwrap(),
        ));
    }
    // Registration state probe, polled by enrolling clients. Mirrors the
    // Worker handler.
    if req.method() == Method::GET
        && let Some(did) = req.uri().path().strip_prefix("/customer/")
    {
        use crate::store::Store;
        use tonk_account::customer::{Receipt, RegistrationError};

        let response = match registration.store.customer(did).await {
            Ok(Some(customer)) => match customer.did.parse() {
                Ok(parsed) => {
                    let receipt = Receipt {
                        customer: parsed,
                        status: customer.status,
                    };
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/json")
                        .body(Full::new(Bytes::from(
                            serde_json::to_vec(&receipt).expect("receipt serializes"),
                        )))
                        .unwrap()
                }
                Err(_) => Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("stored customer did is malformed")))
                    .unwrap(),
            },
            Ok(None) => {
                let refusal = RegistrationError::UnknownCustomer;
                Response::builder()
                    .status(refusal.status())
                    .header(CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(
                        serde_json::to_vec(&serde_json::json!({ "error": refusal }))
                            .expect("refusal serializes"),
                    )))
                    .unwrap()
            }
            Err(err) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from(format!(
                    "customer registry is unavailable: {err}"
                ))))
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

    // Registration commands ride the same endpoint; anything else falls
    // through to the presign path untouched. Mirrors the Worker handler.
    if registration_command(&body_bytes).is_some() {
        let env = Registration {
            store: &registration.store,
            email: &registration.sender,
            service: &registration.service,
            origin: &registration.origin,
            activation_ttl: 24 * 60 * 60,
            now: unix_now(),
            container: &body_bytes,
        };
        let response = match env.handle().await {
            Ok(receipt) => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&receipt).expect("receipt serializes"),
                )))
                .unwrap(),
            Err(err) => Response::builder()
                .status(err.status())
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&serde_json::json!({ "error": err }))
                        .expect("refusal serializes"),
                )))
                .unwrap(),
        };
        return Ok(cors_response(response));
    }

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
    /// Origin activation links open on, when it differs from the
    /// server's own address (a dev proxy in front of it).
    pub public_origin: Option<String>,
}

impl Default for AccessServiceSettings {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret-key".to_string(),
            deployment: None,
            public_origin: None,
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
        settings.public_origin,
    )
    .await?;

    let address = AccessServiceAddress {
        access_service_url: access_server.endpoint.clone(),
        s3_endpoint,
        bucket: bucket.to_string(),
        access_key_id: settings.access_key_id,
        secret_access_key: settings.secret_access_key,
        service_did: access_server.service_did.clone(),
    };

    Ok(Service::new(address, access_server))
}
