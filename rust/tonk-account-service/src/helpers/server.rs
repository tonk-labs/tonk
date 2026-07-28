//! Native account service test server.
//!
//! Routes the same HTTP surface as the Cloudflare Worker (see
//! `src/handlers/`) onto native backends: a `SqliteStore::in_memory()`,
//! a `MemoryChainStore`, and a shared `CapturedEmail`. Route paths,
//! JSON field names, status codes, and CORS headers all match the
//! worker exactly, so a caller (a test or a bench scenario) can't tell
//! the difference.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_LENGTH, CONTENT_TYPE,
};
use hyper::server::conn::http1;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::auth::{
    authorize, authorize_root, optional_revocation, required_string, string_argument,
};
use crate::chains::MemoryChainStore;
use crate::core::accounts::{CreateAccount, create_account};
use crate::core::backup::{get_chain, list_chains, put_chain};
use crate::core::codes::{generate_code, request_code};
use crate::core::devices::{DeviceView, list_devices, register_device, revoke_device};
use crate::core::links::{complete_link, consume_link, create_link, resolve_link};
use crate::email::CapturedEmail;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::ceremony_error;
use crate::revocations::{MemoryRevocationStore, PublishError, publish};
use crate::store::Store;
use crate::store::sqlite::SqliteStore;
use tonk_identity::revocation::VerifyError;

/// The backends a running [`AccountServer`] routes requests onto.
struct Backends {
    store: SqliteStore,
    chains: MemoryChainStore,
    revocations: MemoryRevocationStore,
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
    /// by `SqliteStore::in_memory()`, `MemoryChainStore::default()`, and
    /// a shared `CapturedEmail`.
    pub async fn start() -> AccountServer {
        let emails = Arc::new(CapturedEmail::default());
        let backends = Arc::new(Backends {
            store: SqliteStore::in_memory().expect("in-memory sqlite store"),
            chains: MemoryChainStore::default(),
            revocations: MemoryRevocationStore::default(),
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
        (Method::POST, "/codes") => codes_route(req, &backends).await,
        (Method::POST, "/accounts") => accounts_route(req, &backends).await,
        (Method::POST, "/revocations") => revocations_route(req, &backends).await,
        (Method::POST, "/devices/list") => devices_list_route(req, &backends).await,
        (Method::POST, "/devices/register") => devices_register_route(req, &backends).await,
        (Method::POST, "/devices/link") => devices_link_route(req, &backends).await,
        (Method::POST, "/devices/revoke") => devices_revoke_route(req, &backends).await,
        (Method::POST, "/links") => links_create_route(req, &backends).await,
        (Method::POST, "/links/resolve") => links_resolve_route(req, &backends).await,
        (Method::POST, "/links/complete") => links_complete_route(req, &backends).await,
        (Method::POST, "/links/consume") => links_consume_route(req, &backends).await,
        (Method::POST, "/chains/put") => chains_put_route(req, &backends).await,
        (Method::POST, "/chains/list") => chains_list_route(req, &backends).await,
        (Method::POST, "/chains/get") => chains_get_route(req, &backends).await,
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

/// A device row as serialized to API callers, matching the worker
/// handler's wire shape exactly.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceJson {
    did: String,
    name: String,
    status: String,
    delegation_cid: String,
    delegation_hex: String,
    created_at: u64,
}

impl From<DeviceView> for DeviceJson {
    fn from(view: DeviceView) -> Self {
        DeviceJson {
            did: view.did,
            name: view.name,
            status: view.status,
            delegation_cid: view.delegation_cid,
            delegation_hex: view.delegation_hex,
            created_at: view.created_at,
        }
    }
}

/// `POST /codes` → request a verification code.
async fn codes_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    #[derive(Deserialize)]
    struct CodeRequest {
        email: String,
    }

    let body: CodeRequest = parse_json(req).await?;
    let code = generate_code();
    request_code(
        &backends.store,
        backends.emails.as_ref(),
        &body.email,
        &code,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;

    Ok(json_response(StatusCode::OK, &serde_json::json!({})))
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
    let request = CreateAccount {
        email: required_string(&caller.arguments, "email").map_err(ceremony_error)?,
        code: required_string(&caller.arguments, "code").map_err(ceremony_error)?,
        credential_id: required_string(&caller.arguments, "credentialId")
            .map_err(ceremony_error)?,
        device_did: required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
        device_name: required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
        delegation_hex: required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
        root_did: caller.root_did,
    };
    let account_id = create_account(&backends.store, &request, unix_now())
        .await
        .map_err(ceremony_error)?;

    Ok(json_response(
        StatusCode::CREATED,
        &serde_json::json!({ "accountId": account_id }),
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

    register_device(
        &backends.store,
        &account,
        &device_did,
        &device_name,
        &delegation_hex,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;

    Ok(json_response(StatusCode::OK, &serde_json::json!({})))
}

/// `POST /devices/list` → list the devices registered under an account.
async fn devices_list_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "device", "list"])
        .await
        .map_err(ceremony_error)?;

    let devices: Vec<DeviceJson> = list_devices(&backends.store, &caller.account)
        .await
        .map_err(ceremony_error)?
        .into_iter()
        .map(DeviceJson::from)
        .collect();

    Ok(json_response(StatusCode::OK, &devices))
}

/// `POST /devices/register` → register a new device under an account.
async fn devices_register_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "device", "register"])
        .await
        .map_err(ceremony_error)?;

    let device_did = string_argument(&caller, "did").map_err(ceremony_error)?;
    let device_name = string_argument(&caller, "name").map_err(ceremony_error)?;
    let delegation_hex = string_argument(&caller, "delegation").map_err(ceremony_error)?;

    register_device(
        &backends.store,
        &caller.account,
        &device_did,
        &device_name,
        &delegation_hex,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;

    Ok(json_response(StatusCode::OK, &serde_json::json!({})))
}

/// `POST /revocations` → publish a self-certifying immutable artifact.
async fn revocations_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    const MAX_BYTES: usize = 64 * 1024;
    let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type.split(';').next() != Some("application/cbor") {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "Content-Type must be application/cbor",
        ));
    }
    if req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_BYTES)
    {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "revocation artifact exceeds 64 KiB",
        ));
    }
    let bytes = body_bytes(req).await?;
    if bytes.len() > MAX_BYTES {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "revocation artifact exceeds 64 KiB",
        ));
    }
    let outcome = publish(&backends.revocations, &bytes)
        .await
        .map_err(|error| match error {
            PublishError::Verification(VerifyError::Malformed(message)) => {
                ServiceError::new(ErrorCode::InvalidArgument, message)
            }
            PublishError::Verification(VerifyError::Unauthorized(message)) => {
                ServiceError::new(ErrorCode::Forbidden, message)
            }
            PublishError::Store(error) => {
                eprintln!("revocation publication failed: {error}");
                ServiceError::new(ErrorCode::InternalError, "internal error")
            }
        })?;
    Ok(json_response(
        StatusCode::ACCEPTED,
        &serde_json::json!({
            "targetCid": outcome.verified.target_cid,
            "artifactCid": outcome.verified.artifact_cid,
            "stored": outcome.stored,
        }),
    ))
}

/// `POST /devices/revoke` → revoke a device under an account.
async fn devices_revoke_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "device", "revoke"])
        .await
        .map_err(ceremony_error)?;

    let device_did = string_argument(&caller, "did").map_err(ceremony_error)?;
    let revocation = optional_revocation(&caller)
        .map_err(ceremony_error)?
        .ok_or_else(|| {
            ServiceError::new(
                ErrorCode::InvalidArgument,
                "a signed revocation artifact is required",
            )
        })?;
    let outcome = revoke_device(
        &backends.store,
        &backends.revocations,
        &caller.account,
        &caller.device.device_did,
        &device_did,
        &revocation,
    )
    .await
    .map_err(ceremony_error)?;

    Ok(json_response(
        StatusCode::OK,
        &serde_json::json!({
            "attestation": outcome.attestation.as_str(),
            "projection": outcome.projection.as_str(),
            "targetCid": outcome.target_cid,
            "artifactCid": outcome.artifact_cid,
            "stored": outcome.stored,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkCreateRequest {
    token_hash: String,
    device_did: String,
    device_name: String,
}

#[derive(Deserialize)]
struct LinkSecretRequest {
    secret: String,
}

async fn links_create_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body: LinkCreateRequest = parse_json(req).await?;
    create_link(
        &backends.store,
        &body.token_hash,
        &body.device_did,
        &body.device_name,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;
    Ok(json_response(StatusCode::CREATED, &serde_json::json!({})))
}

async fn links_resolve_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body: LinkSecretRequest = parse_json(req).await?;
    let link = resolve_link(&backends.store, &body.secret, unix_now())
        .await
        .map_err(ceremony_error)?;
    Ok(json_response(StatusCode::OK, &link))
}

async fn links_complete_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize_root(&body, &["account", "link", "complete"])
        .await
        .map_err(ceremony_error)?;
    complete_link(
        &backends.store,
        &caller.root_did,
        &required_string(&caller.arguments, "tokenHash").map_err(ceremony_error)?,
        &required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
        &required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
        &required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
        unix_now(),
    )
    .await
    .map_err(ceremony_error)?;
    Ok(json_response(StatusCode::OK, &serde_json::json!({})))
}

async fn links_consume_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body: LinkSecretRequest = parse_json(req).await?;
    match consume_link(&backends.store, &body.secret, unix_now())
        .await
        .map_err(ceremony_error)?
    {
        Some(consumed) => Ok(json_response(StatusCode::OK, &consumed)),
        None => Ok(json_response(
            StatusCode::ACCEPTED,
            &serde_json::json!({ "pending": true }),
        )),
    }
}

/// `POST /chains/put` → back up a delegation chain, returning its
/// content-addressed key.
async fn chains_put_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "chain", "put"])
        .await
        .map_err(ceremony_error)?;

    let chain_hex = string_argument(&caller, "chain").map_err(ceremony_error)?;
    let bytes = hex::decode(&chain_hex).map_err(|err| {
        ServiceError::new(ErrorCode::InvalidArgument, format!("bad chain hex: {err}"))
    })?;

    let key = put_chain(&backends.chains, &caller.account, &bytes)
        .await
        .map_err(ceremony_error)?;

    Ok(json_response(
        StatusCode::OK,
        &serde_json::json!({ "key": key }),
    ))
}

/// `POST /chains/list` → list the chain keys backed up under an
/// account.
async fn chains_list_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "chain", "list"])
        .await
        .map_err(ceremony_error)?;

    let keys = list_chains(&backends.chains, &caller.account)
        .await
        .map_err(ceremony_error)?;

    Ok(json_response(StatusCode::OK, &keys))
}

/// `POST /chains/get` → fetch the bytes backed up under a chain key.
async fn chains_get_route(
    req: Request<Incoming>,
    backends: &Backends,
) -> Result<Response<Full<Bytes>>, ServiceError> {
    let body = body_bytes(req).await?;
    let caller = authorize(&backends.store, &body, &["account", "chain", "get"])
        .await
        .map_err(ceremony_error)?;

    let key = string_argument(&caller, "key").map_err(ceremony_error)?;
    let bytes = get_chain(&backends.chains, &caller.account, &key)
        .await
        .map_err(ceremony_error)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/octet-stream")
        .body(Full::new(Bytes::from(bytes)))
        .expect("well-formed response"))
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

/// Collect and parse a request's body as JSON.
async fn parse_json<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, ServiceError> {
    let bytes = body_bytes(req).await?;
    serde_json::from_slice(&bytes).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to parse request body: {err}"),
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
