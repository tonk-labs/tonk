//! Native account service test server.
//!
//! Routes the same HTTP surface as the Cloudflare Worker (see
//! `src/handlers/`) onto native backends: a `SqliteStore::in_memory()`,
//! a shared `CapturedEmail`. Route paths,
//! JSON field names, status codes, and CORS headers all match the
//! worker exactly, except for native-only `GET /_test/*` inspection routes
//! used by out-of-process tests.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_TYPE,
};
use hyper::server::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use tokio::net::TcpListener;

use crate::auth::{authorize_root, optional_passkey_metadata, required_string};
use crate::core::accounts::{CreateAccount, create_account};
use crate::core::devices::link_device;
use crate::email::CapturedEmail;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::ceremony_error;
use crate::store::Store;
use crate::store::sqlite::SqliteStore;

/// The backends a running [`AccountServer`] routes requests onto.
struct Backends {
    store: SqliteStore,
    emails: Arc<CapturedEmail>,
}

/// A running native account service, for integration tests and
/// browser-ceremony bench scenarios that can't reach Cloudflare.
pub struct AccountServer {
    /// The endpoint URL the server is listening on, e.g.
    /// `http://127.0.0.1:PORT`.
    pub endpoint: String,
    /// Verification-code emails captured instead of sent, so a caller
    /// can read a code back out directly.
    pub emails: Arc<CapturedEmail>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
}

impl AccountServer {
    /// Start an account service on an ephemeral localhost port, backed
    /// by `SqliteStore::in_memory()` and
    /// a shared `CapturedEmail`.
    pub async fn start() -> AccountServer {
        let emails = Arc::new(CapturedEmail::default());
        let backends = Arc::new(Backends {
            store: SqliteStore::in_memory().expect("in-memory sqlite store"),
            emails: emails.clone(),
        });

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral localhost port");
        let addr = listener.local_addr().expect("bound listener has an addr");
        let endpoint = format!("http://{addr}");

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let server_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    result = listener.accept() => {
                        if let Ok((stream, _)) = result {
                            let backends = backends.clone();
                            tokio::spawn(async move {
                                let service = hyper::service::service_fn(move |req| {
                                    let backends = backends.clone();
                                    async move { handle_request(req, backends).await }
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

        AccountServer {
            endpoint,
            emails,
            shutdown_tx,
            server_handle,
        }
    }

    /// Stop the server and wait for its task to finish.
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.server_handle.await;
    }
}

/// Dispatch a request onto the matching route, adding CORS headers to
/// every response except the unauthenticated `GET /` and `GET /health`
/// checks — mirroring `crate::handlers::with_cors_headers`, which the
/// worker applies to the same set of routes.
async fn handle_request(
    req: Request<Incoming>,
    backends: Arc<Backends>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    if req.method() == Method::OPTIONS {
        return Ok(cors_response(no_content()));
    }

    let response = match (req.method().clone(), req.uri().path()) {
        (Method::GET, "/") => return Ok(info_response()),
        (Method::GET, "/health") => return Ok(health_response()),
        (Method::GET, "/_test/emails") => emails_route(&backends),
        (Method::POST, "/accounts") => accounts_route(req, &backends).await,
        (Method::POST, "/devices/link") => devices_link_route(req, &backends).await,
        _ => Err(ServiceError::new(
            ErrorCode::NotFound,
            "no such route".to_string(),
        )),
    };

    Ok(cors_response(match response {
        Ok(response) => response,
        Err(err) => error_response(err),
    }))
}

/// `GET /` → service info. Not CORS-wrapped, matching the worker.
fn info_response() -> Response<Full<Bytes>> {
    json_response(
        StatusCode::OK,
        &serde_json::json!({
            "service": "tonk-account-service",
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}

/// `GET /health` → liveness check. Not CORS-wrapped, matching the
/// worker.
fn health_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from("OK")))
        .expect("static response is well-formed")
}

/// `GET /_test/emails` → a non-draining snapshot of captured codes.
fn emails_route(backends: &Backends) -> Result<Response<Full<Bytes>>, ServiceError> {
    let emails =
        backends.emails.0.lock().map_err(|_| {
            ServiceError::new(ErrorCode::InternalError, "captured email lock poisoned")
        })?;
    let snapshot: Vec<_> = emails
        .iter()
        .map(|(address, code)| serde_json::json!({ "address": address, "code": code }))
        .collect();
    Ok(json_response(StatusCode::OK, &snapshot))
}

/// `POST /accounts` → create a new account.
async fn accounts_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize_root(&body, &["account", "create"])
        .await
        .map_err(ceremony_error)?;
    let now = unix_now();
    let passkey = optional_passkey_metadata(&caller.arguments, now).map_err(ceremony_error)?;
    let request = CreateAccount {
        email: required_string(&caller.arguments, "email").map_err(ceremony_error)?,
        credential_id: required_string(&caller.arguments, "credentialId")
            .map_err(ceremony_error)?,
        device_did: required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
        device_name: required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
        delegation_hex: required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
        repository_descriptor_hex: required_string(&caller.arguments, "repositoryDescriptor")
            .map_err(ceremony_error)?,
        root_did: caller.root_did,
        passkey,
    };
    let account_id = create_account(&backends.store, &request, now)
        .await
        .map_err(ceremony_error)?;
    let account = backends
        .store
        .account_by_root(&request.root_did)
        .await
        .map_err(|error| ceremony_error(error.into()))?
        .ok_or_else(|| ServiceError::new(ErrorCode::InternalError, "created account missing"))?;
    let descriptor_hex = account
        .repository_descriptor
        .map(hex::encode)
        .ok_or_else(|| ServiceError::new(ErrorCode::InternalError, "descriptor missing"))?;

    Ok(json_response(
        StatusCode::CREATED,
        &serde_json::json!({
            "accountId": account_id,
            "descriptorHex": descriptor_hex,
        }),
    ))
}

/// `POST /devices/link` → register a device from a root-key ceremony.
async fn devices_link_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize_root(&body, &["account", "device", "link"])
        .await
        .map_err(ceremony_error)?;
    let account = backends
        .store
        .account_by_root(&caller.root_did)
        .await
        .map_err(|err| ceremony_error(err.into()))?
        .ok_or_else(|| {
            ceremony_error(crate::core::CeremonyError::Unauthorized(
                "unknown account".to_string(),
            ))
        })?;
    let device_did = required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?;
    let device_name = required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?;
    let delegation_hex =
        required_string(&caller.arguments, "delegation").map_err(ceremony_error)?;
    let descriptor = account.repository_descriptor.as_ref().ok_or_else(|| {
        ceremony_error(crate::core::CeremonyError::Conflict(
            tonk_account::UNESTABLISHED_ACCOUNT_CONFLICT.to_string(),
        ))
    })?;
    let descriptor_hex = hex::encode(descriptor);

    let attachment_id = link_device(
        &backends.store,
        &account,
        &device_did,
        &device_name,
        &delegation_hex,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;

    Ok(json_response(
        StatusCode::OK,
        &serde_json::json!({
            "attachmentId": attachment_id,
            "descriptorHex": descriptor_hex,
        }),
    ))
}

/// Current time as unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is past the epoch")
        .as_secs()
}

/// Collect a request's body as raw bytes.
async fn body_bytes(req: Request<Incoming>) -> Result<Vec<u8>, ServiceError> {
    req.into_body()
        .collect()
        .await
        .map(|collected| collected.to_bytes().to_vec())
        .map_err(|err| {
            ServiceError::new(
                ErrorCode::InvalidArgument,
                format!("failed to read request body: {err}"),
            )
        })
}

/// Build a JSON response with the given status.
fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(body).expect("response body serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("well-formed response")
}

/// Build the JSON error envelope for a [`ServiceError`], with its
/// matching HTTP status.
fn error_response(err: ServiceError) -> Response<Full<Bytes>> {
    let status =
        StatusCode::from_u16(err.code.status_code()).expect("error codes map to valid statuses");
    json_response(status, &serde_json::json!({ "error": err }))
}

/// An empty `204 No Content` response, for CORS preflight.
fn no_content() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("well-formed response")
}

/// Add CORS headers to a response, matching
/// `crate::handlers::with_cors_headers`.
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
