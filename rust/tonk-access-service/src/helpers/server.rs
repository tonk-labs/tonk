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
use crate::store::ingest::{IngestStore, SqliteIngest};
use crate::store::sqlite::SqliteStore;
use async_trait::async_trait;
use dialog_common::helpers::{Provider, Service};
use dialog_credentials::Ed25519Signer;
use dialog_remote_s3::helpers::LocalS3;
use dialog_remote_s3::{Address, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;

/// The authorizer this server runs, revocation checking included.
type ServerAuthorizer = UcanAuthorizer<
    dialog_remote_ucan_s3::DefaultResolver,
    crate::revocation::checker::IndexedRevocations<
        Arc<crate::revocation::index::MemoryRevocationIndex>,
    >,
>;
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
    ingest: SqliteIngest,
    emails: Arc<CapturedEmail>,
    sender: AnnouncedEmail,
    service: Ed25519Signer,
    origin: String,
    purger: crate::deletion::NativeSpacePurger,
    /// Revocations recorded by `/ucan/revoke`. In memory, as the
    /// worker's KV namespace is. Shared with the authorizer, which
    /// reads it back while verifying every presented chain.
    revocations: Arc<crate::revocation::index::MemoryRevocationIndex>,
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
        state_dir: Option<&std::path::Path>,
    ) -> anyhow::Result<Self> {
        // Create S3 credentials for the authorizer
        let address = Address::builder(&s3_server.endpoint)
            .region("us-east-1")
            .bucket(bucket)
            .path_style(true)
            .build()?;

        let credential = S3Credential::new(access_key, secret_key);

        // Create UcanAuthorizer - the core of our service
        let purger = crate::deletion::NativeSpacePurger::new(address.clone(), credential.clone());
        // The authorizer checks revocation itself, per link, during the
        // chain walk. It reads the same index `/ucan/revoke` writes, so
        // a revocation recorded by one request governs the next.
        let revocations: Arc<crate::revocation::index::MemoryRevocationIndex> = Default::default();
        let authorizer = Arc::new(RwLock::new(
            UcanAuthorizer::new(address, Some(credential)).with_revocations(
                crate::revocation::checker::IndexedRevocations(revocations.clone()),
            ),
        ));

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let endpoint = format!("http://{}", addr);

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let emails = Arc::new(CapturedEmail::default());
        // A persistent state dir keeps the service's identity stable
        // across restarts; rotating it would orphan the deposits and the
        // enrollment records the published service DID anchors.
        let service = match state_dir {
            Some(dir) => persistent_signer(dir).await?,
            None => Ed25519Signer::generate()
                .await
                .map_err(|err| anyhow::anyhow!("service signer: {err:?}"))?,
        };
        let service_did = service.did().to_string();
        let (store, ingest) = match state_dir {
            Some(dir) => (
                SqliteStore::open(&dir.join("control.sqlite"))
                    .map_err(|err| anyhow::anyhow!("{err}"))?,
                SqliteIngest::open(&dir.join("ingest.sqlite"))
                    .map_err(|err| anyhow::anyhow!("{err}"))?,
            ),
            None => (
                SqliteStore::in_memory().map_err(|err| anyhow::anyhow!("{err}"))?,
                SqliteIngest::in_memory().map_err(|err| anyhow::anyhow!("{err}"))?,
            ),
        };
        let registration = Arc::new(RegistrationState {
            store,
            ingest,
            emails: emails.clone(),
            sender: AnnouncedEmail(emails.clone()),
            service,
            // Activation links open on the page origin, which behind a
            // dev proxy is not this server's own address.
            origin: public_origin.unwrap_or_else(|| endpoint.clone()),
            purger,
            revocations,
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
    authorizer: Arc<RwLock<ServerAuthorizer>>,
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
    // Test-only shortcut past the registration ceremony: make a subject
    // servable by provisioning it under a synthetic active customer.
    // For tests whose subject is a repository DID they hold no signer
    // for; anything testing registration itself drives the real
    // endpoints.
    if req.method() == Method::POST && req.uri().path() == "/_test/provision" {
        use http_body_util::BodyExt;

        let body = req.into_body().collect().await.map(|c| c.to_bytes());
        let subject = body
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value["subject"].as_str().map(str::to_owned));
        let Some(subject) = subject else {
            return Ok(cors_response(
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from("a subject is required")))
                    .unwrap(),
            ));
        };
        return Ok(cors_response(
            match provision_for_tests(&registration.store, &subject).await {
                Ok(()) => Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
                Err(error) => Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from(error.to_string())))
                    .unwrap(),
            },
        ));
    }
    if req.method() == Method::GET && req.uri().path() == "/_test/ingest" {
        let count = registration.ingest.invocations().unwrap_or_default();
        return Ok(cors_response(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::json!({ "invocations": count }).to_string(),
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
    if crate::deletion::is_deletion(&body_bytes) {
        let response = match crate::deletion::delete(
            &registration.store,
            &registration.purger,
            &body_bytes,
            unix_now(),
        )
        .await
        {
            Ok(receipt) => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&receipt).expect("deletion receipt serializes"),
                )))
                .unwrap(),
            Err(error) => Response::builder()
                .status(error.status())
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&serde_json::json!({ "error": error }))
                        .expect("deletion refusal serializes"),
                )))
                .unwrap(),
        };
        return Ok(cors_response(response));
    }
    if crate::deletion::is_customer_deletion(&body_bytes) {
        let response = if crate::deletion::command_for_native_handler(&body_bytes)
            == crate::deletion::CUSTOMER_PLAN_COMMAND.map(str::to_string)
        {
            match crate::deletion::customer_plan(&registration.store, &body_bytes, unix_now()).await
            {
                Ok(plan) => Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(
                        serde_json::to_vec(&plan).expect("deletion plan serializes"),
                    )))
                    .unwrap(),
                Err(error) => deletion_error_response(error),
            }
        } else {
            match crate::deletion::delete_customer(
                &registration.store,
                &registration.purger,
                &body_bytes,
                unix_now(),
            )
            .await
            {
                Ok(receipt) => Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Full::new(Bytes::from(
                        serde_json::to_vec(&receipt).expect("deletion receipt serializes"),
                    )))
                    .unwrap(),
                Err(error) => deletion_error_response(error),
            }
        };
        return Ok(cors_response(response));
    }
    // Revocation writes to the index rather than reading it, so it is
    // answered before the presign path, mirroring the worker.
    if crate::revoke::is_revocation(&body_bytes) {
        let response = match crate::revoke::revoke(
            &registration.store,
            &registration.revocations,
            &body_bytes,
        )
        .await
        {
            Ok(receipt) => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&receipt).expect("revoke receipt serializes"),
                )))
                .unwrap(),
            Err(error) => Response::builder()
                .status(error.status())
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(
                    serde_json::to_vec(&serde_json::json!({ "error": error }))
                        .expect("revoke refusal serializes"),
                )))
                .unwrap(),
        };
        return Ok(cors_response(response));
    }
    if registration_command(&body_bytes).is_some() {
        let env = Registration {
            store: &registration.store,
            email: &registration.sender,
            service: &registration.service,
            origin: &registration.origin,
            activation_ttl: 24 * 60 * 60,
            now: unix_now(),
            container: &body_bytes,
            // The same index the authorizer reads, so a revocation
            // recorded by `/ucan/revoke` governs registration too.
            revocations: &crate::revocation::checker::IndexedRevocations(
                registration.revocations.clone(),
            ),
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
    let outcome = authorizer.authorize(&body_bytes).await;
    if outcome.is_ok()
        && let Some(subject) = crate::deletion::subject(&body_bytes)
    {
        use crate::store::{ConsumerDeletionState, Store};
        match registration.store.consumer(subject.as_str()).await {
            Ok(Some(consumer)) if consumer.deletion_state != ConsumerDeletionState::Active => {
                return Ok(cors_response(
                    Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .body(Full::new(Bytes::from(
                            "Authorization failed: hosted space is deleting or deleted",
                        )))
                        .unwrap(),
                ));
            }
            Err(error) => {
                return Ok(cors_response(
                    Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .body(Full::new(Bytes::from(format!(
                            "consumer deletion state unavailable: {error}"
                        ))))
                        .unwrap(),
                ));
            }
            _ => {}
        }
    }
    // The provisioning gate, mirroring the worker: a subject is served
    // only while an active customer pays for it. Registration commands
    // returned above, so enrolling and activating stay possible while
    // this denies the data plane.
    if outcome.is_ok() {
        match crate::provisioning::container_subject(&body_bytes) {
            Some(subject) => match crate::provisioning::screen(&registration.store, &subject).await
            {
                Ok(Ok(())) => {}
                Ok(Err(reason)) => {
                    return Ok(cors_response(authorize_error_response(
                        StatusCode::FORBIDDEN,
                        &reason,
                    )));
                }
                Err(error) => {
                    // Fails closed, but as our own unavailability rather
                    // than a denial billed to the customer.
                    eprintln!("presign refused, control store unreachable: {error}");
                    return Ok(cors_response(authorize_error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &unavailable_provisioning(),
                    )));
                }
            },
            None => {
                return Ok(cors_response(authorize_error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &unavailable_provisioning(),
                )));
            }
        }
    }
    // Metering mirrors the worker: permits and attributable denials are
    // recorded, infra failures and unparseable containers are not.
    let metered = match &outcome {
        Ok(descriptor) => {
            let bytes = descriptor
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.parse().ok())
                .unwrap_or(0);
            Some(("ok", None, bytes))
        }
        Err(dialog_remote_s3::S3Error::Authorization(reason)) => {
            Some(("denied", Some(format!("{reason:?}")), 0))
        }
        Err(_) => None,
    };
    if let Some((label, reason, bytes)) = metered
        && let Some(record) =
            crate::metering::collect(&body_bytes, label, reason, bytes, unix_now())
        && let Err(error) = registration.ingest.record(&record).await
    {
        eprintln!("metering write failed: {error}");
    }
    match outcome {
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
        // The refusal travels as itself. Rendering it into prose here
        // would undo the point of the authorizer naming what failed: a
        // client parsing the body could no longer tell an expired proof
        // from a revoked one, and every denial through this server
        // would arrive unclassified.
        Err(dialog_remote_s3::S3Error::Authorization(reason)) => Ok(cors_response(
            authorize_error_response(authorize_status(&reason), &reason),
        )),
        Err(other) => Ok(cors_response(
            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Full::new(Bytes::from(format!(
                    "Authorization failed: {other}"
                ))))
                .unwrap(),
        )),
    }
}

/// The status an authorization refusal answers with.
///
/// Shares the worker's mapping rather than restating it, so the two
/// deployments cannot answer the same refusal differently.
fn authorize_status(reason: &dialog_capability::access::AuthorizeError) -> StatusCode {
    StatusCode::from_u16(crate::error::Refusal::Authorization(reason.clone()).status())
        .unwrap_or(StatusCode::FORBIDDEN)
}

/// Current time as unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is past the epoch")
        .as_secs()
}

/// Provision `subject` under a synthetic active customer, so the
/// provisioning gate serves it. Idempotent, and derived from the
/// subject so two subjects never collide on one provider row.
async fn provision_for_tests(store: &SqliteStore, subject: &str) -> anyhow::Result<()> {
    use crate::store::{ConsumerKind, SIGNUP_PLAN, Store};

    let provider = format!("did:test:provider-for-{subject}");
    if store
        .customer(&provider)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .is_none()
    {
        store
            .enroll_customer(&provider, "tests@example.com", b"", SIGNUP_PLAN, 0)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        store
            .activate_customer(&provider, "test", 1)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    store
        .add_consumer(subject, &provider, 0, ConsumerKind::Space)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(())
}

/// The refusal a gate that could not reach a verdict answers with.
fn unavailable_provisioning() -> dialog_capability::access::AuthorizeError {
    dialog_capability::access::AuthorizeError::Unavailable {
        detail: "provisioning registry unavailable, retry shortly".to_string(),
    }
}

/// Answer a refusal as the worker does: the serde-tagged
/// [`AuthorizeError`] itself, which is what the client parses back out.
/// A plain-text body would classify as unclassified on the other side.
fn authorize_error_response(
    status: StatusCode,
    reason: &dialog_capability::access::AuthorizeError,
) -> Response<http_body_util::Full<bytes::Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(
            serde_json::to_vec(reason).expect("authorize refusal serializes"),
        )))
        .unwrap()
}

fn deletion_error_response(
    error: crate::deletion::Error,
) -> Response<http_body_util::Full<bytes::Bytes>> {
    Response::builder()
        .status(error.status())
        .header(CONTENT_TYPE, "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(
            serde_json::to_vec(&serde_json::json!({ "error": error }))
                .expect("deletion refusal serializes"),
        )))
        .unwrap()
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
    /// Directory the service persists its state under: control and
    /// ingest databases, the service signing key, and a snapshot of the
    /// blob store. Absent means fully in-memory, the shape tests want; a
    /// dev stack sets it so a restart stops wiping registrations and
    /// every synced block — and the delegations retained in account
    /// repositories with them.
    pub state_dir: Option<std::path::PathBuf>,
}

impl Default for AccessServiceSettings {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret-key".to_string(),
            deployment: None,
            public_origin: None,
            state_dir: None,
        }
    }
}

/// The service's signing identity from `{dir}/service.key`, minting and
/// persisting a fresh seed on first start.
async fn persistent_signer(dir: &std::path::Path) -> anyhow::Result<Ed25519Signer> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("service.key");
    if let Ok(seed) = std::fs::read_to_string(&path) {
        return crate::service::signer_from_hex(seed.trim())
            .map_err(|message| anyhow::anyhow!("stored service key is unusable: {message}"));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|err| anyhow::anyhow!("no entropy source: {err}"))?;
    let encoded = hex::encode(seed);
    std::fs::write(&path, &encoded)?;
    crate::service::signer_from_hex(&encoded)
        .map_err(|message| anyhow::anyhow!("fresh service key is unusable: {message}"))
}

/// Dev durability for the in-memory blob store: hydrate it from a
/// directory at start and mirror it back on a short cadence. The store
/// is only reachable over its S3 API — presigned uploads go straight to
/// it, never through this server — so the mirror polls a listing rather
/// than hooking writes. Cheap at development sizes, and the price of not
/// losing every synced block (and the delegations retained in account
/// repositories) to a restart.
mod blob_snapshot {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use dialog_remote_s3::request::S3Request;
    use dialog_remote_s3::s3::S3Credential;
    use dialog_remote_s3::{Address, Permit};

    async fn permit(
        credential: &S3Credential,
        address: &Address,
        method: &str,
        path: &str,
        params: Option<Vec<(String, String)>>,
    ) -> anyhow::Result<Permit> {
        S3Request {
            method: method.to_string(),
            path: path.to_string(),
            params,
            ..Default::default()
        }
        .attest(credential.clone())
        .redeem(address)
        .await
        .map_err(|err| anyhow::anyhow!("presign {method} {path}: {err:?}"))
    }

    async fn perform(permit: Permit, body: Option<Vec<u8>>) -> anyhow::Result<reqwest::Response> {
        let client = reqwest::Client::new();
        let mut request = match permit.method.as_str() {
            "PUT" => client.put(permit.url),
            "DELETE" => client.delete(permit.url),
            _ => client.get(permit.url),
        };
        for (name, value) in &permit.headers {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        Ok(request.send().await?.error_for_status()?)
    }

    /// One flat file per object: the key percent-encoded, so keys with
    /// `/` never collide with directory structure.
    fn file_for(dir: &Path, key: &str) -> PathBuf {
        dir.join(urlencoding::encode(key).into_owned())
    }

    fn key_for(file: &Path) -> Option<String> {
        let name = file.file_name()?.to_str()?;
        urlencoding::decode(name).ok().map(|key| key.into_owned())
    }

    /// Extract `(key, etag)` pairs and the continuation token from a
    /// ListObjectsV2 answer. A hand parse, deliberately: this is a dev
    /// helper talking to one known server, not a general S3 client.
    fn parse_listing(xml: &str) -> (Vec<(String, String)>, Option<String>) {
        fn tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            xml.split(open.as_str())
                .skip(1)
                .filter_map(|rest| rest.split(close.as_str()).next())
                .collect()
        }
        let mut objects = Vec::new();
        for contents in xml.split("<Contents>").skip(1) {
            let keys = tags(contents, "Key");
            let etags = tags(contents, "ETag");
            if let (Some(key), Some(etag)) = (keys.first(), etags.first()) {
                objects.push((key.to_string(), etag.to_string()));
            }
        }
        let token = tags(xml, "NextContinuationToken")
            .first()
            .map(|token| token.to_string());
        (objects, token)
    }

    async fn list(
        credential: &S3Credential,
        address: &Address,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let mut objects = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut params = vec![("list-type".to_string(), "2".to_string())];
            if let Some(token) = &token {
                params.push(("continuation-token".to_string(), token.clone()));
            }
            let permit = permit(credential, address, "GET", "", Some(params)).await?;
            let body = perform(permit, None).await?.text().await?;
            let (page, next) = parse_listing(&body);
            objects.extend(page);
            match next {
                Some(next) => token = Some(next),
                None => return Ok(objects),
            }
        }
    }

    /// Upload every snapshotted object into the fresh store.
    pub async fn hydrate(
        credential: &S3Credential,
        address: &Address,
        dir: &Path,
    ) -> anyhow::Result<usize> {
        std::fs::create_dir_all(dir)?;
        let mut restored = 0;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(key) = key_for(&path) else { continue };
            let body = std::fs::read(&path)?;
            let permit = permit(credential, address, "PUT", &key, None).await?;
            perform(permit, Some(body)).await?;
            restored += 1;
        }
        Ok(restored)
    }

    /// Mirror the store into `dir` forever, on a short cadence. Every
    /// pass fetches objects whose ETag changed since the last one and
    /// removes files whose key is gone.
    pub async fn mirror(credential: S3Credential, address: Address, dir: PathBuf) {
        let mut seen: HashMap<String, String> = HashMap::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let listing = match list(&credential, &address).await {
                Ok(listing) => listing,
                Err(error) => {
                    eprintln!("blob snapshot listing failed: {error}");
                    continue;
                }
            };
            let live: HashMap<String, String> = listing.into_iter().collect();
            for (key, etag) in &live {
                if seen.get(key) == Some(etag) {
                    continue;
                }
                let fetched = async {
                    let permit = permit(&credential, &address, "GET", key, None).await?;
                    let body = perform(permit, None).await?.bytes().await?;
                    let target = file_for(&dir, key);
                    let staged = target.with_extension("tmp");
                    std::fs::write(&staged, &body)?;
                    std::fs::rename(&staged, &target)?;
                    Ok::<(), anyhow::Error>(())
                }
                .await;
                match fetched {
                    Ok(()) => {
                        seen.insert(key.clone(), etag.clone());
                    }
                    Err(error) => eprintln!("blob snapshot of {key} failed: {error}"),
                }
            }
            seen.retain(|key, _| {
                if live.contains_key(key) {
                    return true;
                }
                let _ = std::fs::remove_file(file_for(&dir, key));
                false
            });
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

    // With a state dir, refill the fresh in-memory store from the last
    // snapshot before anything can talk to it, then keep mirroring it
    // back for the next restart.
    if let Some(state_dir) = &settings.state_dir {
        let address = Address::builder(&s3_endpoint)
            .region("us-east-1")
            .bucket(bucket)
            .path_style(true)
            .build()?;
        let credential = S3Credential::new(&settings.access_key_id, &settings.secret_access_key);
        let blobs = state_dir.join("blobs");
        let restored = blob_snapshot::hydrate(&credential, &address, &blobs).await?;
        if restored > 0 {
            println!(
                "ACCESS_STATE restored {restored} blobs from {}",
                blobs.display()
            );
        }
        tokio::spawn(blob_snapshot::mirror(credential, address, blobs));
    }

    // Start the UCAN access service
    let access_server = AccessServer::start(
        s3_server,
        bucket,
        &settings.access_key_id,
        &settings.secret_access_key,
        settings.deployment,
        settings.public_origin,
        settings.state_dir.as_deref(),
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
