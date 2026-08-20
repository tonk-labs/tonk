//! Customer registration with the access service.
//!
//! Enrollment runs after the account attaches: the device signs a
//! `/customer/enroll` invocation on the account's subject through its
//! `root → device` grant, depositing a delegation that lets the service
//! write into the account space, and the service emails an activation
//! link. Everything is same-origin — the access service is the
//! deployment serving this page — so endpoints derive from the request
//! origin and the service DID comes from `/.well-known/tonk`.

use axum::{
    Json,
    extract::{Extension, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::time::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::customer::{CustomerStatus, Receipt};
use tonk_account::pending::{PendingQueue, PendingWork};
use tonk_account::{CUSTOMER_CREDENTIAL_SITE, PENDING_WORK_CREDENTIAL_SITE};
use tonk_common::log;
use tonk_identity::request::{build_enroll_invocation, build_enroll_invocation_with_deposits};
use url::Url;

use super::AppState;
use super::http::{HttpError, get, post_cbor};
use crate::TonkWorkerError;
use crate::axum::RequestOrigin;

/// What this device recorded about its account's customer registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerRecord {
    /// The customer DID, which is the account root.
    pub customer: String,
    /// The email enrollment named.
    pub email: String,
    /// The status the service last answered.
    pub status: CustomerStatus,
    /// When this device enrolled, as a unix timestamp in seconds.
    pub enrolled_at: u64,
}

/// `POST /api/customer/enroll` request body.
#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    /// Address the activation link is sent to. Absent on the login path,
    /// where the account's recorded email is used instead.
    pub email: Option<String>,
    /// Hex-encoded account-signed deposits the passkey ceremony minted.
    /// Preferred over the device-issued fallback: an account-signed
    /// deposit survives revocation of the device that carried it. Empty
    /// when enrollment runs without a fresh ceremony (a resend).
    #[serde(default)]
    pub deposits: Vec<String>,
}

/// The answer to a customer state read: the service's view when it has
/// one, and the locally recorded enrollment either way.
#[derive(Debug, Serialize)]
pub struct CustomerState {
    /// The account root DID the customer registration is keyed by.
    pub customer: String,
    /// The service's current answer; absent when unregistered there.
    pub status: Option<CustomerStatus>,
    /// The email this device enrolled with, when it did.
    pub email: Option<String>,
}

/// POST `/api/customer/enroll` → enroll this profile's account as a
/// customer of the same-origin access service. Idempotent: re-enrolling
/// while registered resends the activation email.
#[wasm_compat]
pub async fn enroll(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
    Json(request): Json<EnrollRequest>,
) -> Result<Json<Receipt>, TonkWorkerError> {
    let state = state.read().await;
    let link = super::account::account_link(&state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let root_did = link.issuer().clone();
    let email = match request.email {
        Some(email) => email,
        // The login path names no address: the account's recorded email
        // is authoritative there.
        None => super::account_devices::account_summary(&state)
            .await?
            .email
            .ok_or_else(|| {
                TonkWorkerError::NotFound(
                    "the account records no email address to enroll with".to_string(),
                )
            })?,
    };
    let device = state.profile.signer().signer().clone();

    // The deposits — the service's scoped grants into the account
    // space — ride in the container alongside the invocation. The
    // account-signed set a ceremony minted is preferred; without one, a
    // device-issued set chained through the `root → device` grant is the
    // fallback the service walks back to the customer.
    let body = if request.deposits.is_empty() {
        let service_did = service_did(origin.url()).await?;
        build_enroll_invocation(device, &link, &service_did, &email).await
    } else {
        let deposits = request
            .deposits
            .iter()
            .map(hex::decode)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                TonkWorkerError::Router(format!("a ceremony deposit is not hex: {error}"))
            })?;
        build_enroll_invocation_with_deposits(device, &link, &email, &deposits).await
    }
    .map_err(|error| {
        TonkWorkerError::Internal(format!("failed to build the enroll invocation: {error}"))
    })?;

    let endpoint = ucan_endpoint(origin.url())?;
    let receipt = match post_cbor(&endpoint, &body).await {
        Ok(response) => serde_json::from_slice::<Receipt>(&response.body).map_err(|error| {
            TonkWorkerError::Internal(format!(
                "enrollment answered an unreadable receipt: {error}"
            ))
        })?,
        // An already-active customer is where the login path lands, and
        // it is the outcome enrollment exists to reach: answer it as
        // one rather than surfacing a refusal.
        Err(HttpError::Upstream(failure)) if failure.code.as_deref() == Some("CustomerActive") => {
            Receipt {
                customer: root_did.clone(),
                status: CustomerStatus::Active,
            }
        }
        Err(error) => return Err(error.into()),
    };

    save_customer(
        &state,
        &CustomerRecord {
            customer: receipt.customer.to_string(),
            email,
            status: receipt.status,
            enrolled_at: Timestamp::now().to_unix(),
        },
    )
    .await?;
    Ok(Json(receipt))
}

/// GET `/api/customer` → the account's registration state: the service's
/// live answer joined with the locally recorded enrollment.
#[wasm_compat]
pub async fn get_state(
    State(state): State<AppState>,
    Extension(origin): Extension<RequestOrigin>,
) -> Result<Json<CustomerState>, TonkWorkerError> {
    let state = state.read().await;
    let root = super::identity::root_did(&state).await?;
    let record = load_customer(&state).await?;
    let endpoint = origin
        .url()
        .join(&format!("customer/{root}"))
        .map_err(|error| TonkWorkerError::Internal(format!("customer probe url: {error}")))?;
    let status = match get(&endpoint).await {
        Ok(response) => {
            let receipt: Receipt = serde_json::from_slice(&response.body).map_err(|error| {
                TonkWorkerError::Internal(format!("customer probe answered garbage: {error}"))
            })?;
            // Keep the recorded status current, so the panel can tell a
            // pending activation from a finished one without the probe.
            if let Some(record) = &record
                && record.status != receipt.status
            {
                let refreshed = CustomerRecord {
                    status: receipt.status,
                    ..record.clone()
                };
                save_customer(&state, &refreshed).await?;
            }
            // This probe is what notices activation, so it is where
            // work deferred during the wait gets replayed.
            if receipt.status == CustomerStatus::Active {
                drain_pending(&state).await;
            }
            Some(receipt.status)
        }
        Err(HttpError::Upstream(failure)) if failure.status == 404 => None,
        Err(error) => return Err(error.into()),
    };
    Ok(Json(CustomerState {
        customer: root.to_string(),
        status,
        email: record.map(|record| record.email),
    }))
}

/// `POST /api/custody/provision` request body: the custody DID a
/// passkey enrollment derived, and the consent chain the custody key
/// minted for `/provider/add`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionCustodyRequest {
    /// The custody space's subject.
    pub custody: String,
    /// Hex-encoded consent delegation chain.
    pub consent_hex: String,
}

/// POST `/api/custody/provision` → provision a custody space under
/// this profile's account. The page runs the enrollment ceremony and
/// hands the consent here; the call is idempotent, and retryable — the
/// published cell, not this row, is the account's durability.
#[wasm_compat]
pub async fn provision_custody(
    State(state): State<AppState>,
    Json(request): Json<ProvisionCustodyRequest>,
) -> Result<Json<()>, TonkWorkerError> {
    let state = state.read().await;
    let custody: dialog_varsig::Did = request
        .custody
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid custody DID: {error:?}")))?;
    let bytes = hex::decode(&request.consent_hex)
        .map_err(|error| TonkWorkerError::Router(format!("consent is not hex: {error}")))?;
    let consent = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("consent does not decode: {error}")))?;
    provision_or_defer(&state, &custody, &consent, Some("custody")).await?;
    Ok(Json(()))
}

/// `POST /api/custody/queue` request body: a custody cell that could
/// not be published yet.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueCustodyRequest {
    /// The custody space whose cell is waiting.
    pub custody: String,
    /// Hex-encoded sealed envelope to publish.
    pub sealed_hex: String,
}

/// POST `/api/custody/queue` → record a custody cell for publication
/// once its space is provisioned and the account confirms its email.
/// The page publishes it — only a fresh passkey assertion can sign for
/// the custody key — so this records rather than performs.
#[wasm_compat]
pub async fn queue_custody(
    State(state): State<AppState>,
    Json(request): Json<QueueCustodyRequest>,
) -> Result<Json<()>, TonkWorkerError> {
    let state = state.read().await;
    defer(
        &state,
        PendingWork::PublishCustody {
            custody: request.custody,
            sealed_hex: request.sealed_hex,
        },
    )
    .await?;
    Ok(Json(()))
}

/// Provision `consumer`, or queue it when the account has not yet
/// confirmed its email.
///
/// The access service provisions nothing for a customer that is only
/// `Registered`, so that refusal is not a failure here — it is the
/// expected answer during the window between enrolling and clicking the
/// emailed link. The entry replays from the status probe that notices
/// activation. Every other refusal propagates.
pub(crate) async fn provision_or_defer(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
    consent: &dialog_ucan_core::DelegationChain,
    kind: Option<&str>,
) -> Result<(), TonkWorkerError> {
    match provision_consumer(state, consumer, consent, kind).await {
        Err(TonkWorkerError::Upstream { ref code, .. })
            if code.as_deref() == Some("CustomerInactive") =>
        {
            log!("{consumer} queued until the account confirms its email");
            defer(
                state,
                PendingWork::Provision {
                    consumer: consumer.to_string(),
                    consent_hex: hex::encode(consent.to_bytes().map_err(|error| {
                        TonkWorkerError::Internal(format!("consent does not encode: {error}"))
                    })?),
                    consumer_kind: kind.map(str::to_owned),
                },
            )
            .await
        }
        other => other,
    }
}

/// Provision `consumer` with the same-origin access service under this
/// profile's account, depositing `consent` — the space's powerline to
/// the account. A consumer another customer already provides is not an
/// error here: the space exists and works locally either way, and the
/// caller treats this whole call as best effort.
pub(crate) async fn provision_consumer(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
    consent: &dialog_ucan_core::DelegationChain,
    kind: Option<&str>,
) -> Result<(), TonkWorkerError> {
    use tonk_identity::request::build_provider_add_invocation;

    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let device = state.profile.signer().signer().clone();
    let body = build_provider_add_invocation(device, &link, consumer, consent, kind)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to build the add invocation: {error}"))
        })?;
    let origin = service_origin()?;
    match post_cbor(&ucan_endpoint(&origin)?, &body).await {
        Ok(_) => Ok(()),
        Err(HttpError::Upstream(failure))
            if failure.code.as_deref() == Some("ConsumerProvided") =>
        {
            log!("consumer {consumer} already has a provider; leaving it");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Deprovision `consumer` at `origin`: the device-signed
/// `/provider/remove` that deletes the hosted space. Authority is the
/// account's own chain — any linked device may exercise it.
pub(crate) async fn deprovision_consumer(
    state: &crate::worker::TonkState,
    origin: &Url,
    consumer: &dialog_varsig::Did,
) -> Result<(), TonkWorkerError> {
    use tonk_identity::request::build_provider_remove_invocation;

    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let device = state.profile.signer().signer().clone();
    let body = build_provider_remove_invocation(device, &link, consumer)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to build the remove invocation: {error}"))
        })?;
    post_cbor(&ucan_endpoint(origin)?, &body).await?;
    Ok(())
}

/// The worker's own origin, which the access service serves. Known only
/// inside a service-worker scope; callers outside one (native tests)
/// carry an origin of their own through `RequestOrigin` instead.
fn service_origin() -> Result<Url, TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let origin = super::repository::worker_origin().ok_or_else(|| {
            TonkWorkerError::Internal("the worker origin is unavailable".to_string())
        })?;
        format!("{origin}/").parse().map_err(|error| {
            TonkWorkerError::Internal(format!("worker origin is not a URL: {error}"))
        })
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    Err(TonkWorkerError::Internal(
        "the worker origin is only known in a service-worker scope".to_string(),
    ))
}

/// The service DID from the same-origin deployment configuration.
async fn service_did(origin: &Url) -> Result<dialog_varsig::Did, TonkWorkerError> {
    let endpoint = origin
        .join(".well-known/tonk")
        .map_err(|error| TonkWorkerError::Internal(format!("deployment config url: {error}")))?;
    let response = get(&endpoint).await?;
    let config: tonk_worker_api::DeploymentConfig = serde_json::from_slice(&response.body)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("deployment configuration is invalid: {error}"))
        })?;
    let did = config.service_did.ok_or_else(|| {
        TonkWorkerError::Internal(
            "this deployment publishes no service identity, so enrollment cannot address it"
                .to_string(),
        )
    })?;
    did.parse().map_err(|error| {
        TonkWorkerError::Internal(format!("deployment service DID is invalid: {error:?}"))
    })
}

/// The same-origin `/ucan/` endpoint.
fn ucan_endpoint(origin: &Url) -> Result<Url, TonkWorkerError> {
    origin
        .join("ucan/")
        .map_err(|error| TonkWorkerError::Internal(format!("ucan endpoint url: {error}")))
}

async fn load_customer(
    state: &crate::worker::TonkState,
) -> Result<Option<CustomerRecord>, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(CUSTOMER_CREDENTIAL_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load the customer record: {error}"
            )));
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(record) => Ok(Some(record)),
        Err(error) => {
            // An unreadable record only loses a cached view the probe
            // rebuilds, so log rather than fail the read.
            log!("stored customer record is unreadable: {error}");
            Ok(None)
        }
    }
}

async fn save_customer(
    state: &crate::worker::TonkState,
    record: &CustomerRecord,
) -> Result<(), TonkWorkerError> {
    let bytes = serde_json::to_vec(record).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize the customer record: {error}"))
    })?;
    state
        .profile
        .credential()
        .site(CUSTOMER_CREDENTIAL_SITE)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save the customer record: {error}"))
        })
}

async fn load_pending(state: &crate::worker::TonkState) -> Result<PendingQueue, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(PENDING_WORK_CREDENTIAL_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(PendingQueue::default()),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load pending work: {error}"
            )));
        }
    };
    if bytes.is_empty() {
        return Ok(PendingQueue::default());
    }
    match serde_json::from_slice(&bytes) {
        Ok(queue) => Ok(queue),
        Err(error) => {
            // An unreadable queue would otherwise wedge every later
            // append. The work it held is recoverable — re-running the
            // ceremony re-queues it — where a permanently poisoned site
            // is not.
            log!("stored pending work is unreadable, starting a fresh queue: {error}");
            Ok(PendingQueue::default())
        }
    }
}

async fn save_pending(
    state: &crate::worker::TonkState,
    queue: &PendingQueue,
) -> Result<(), TonkWorkerError> {
    let bytes = serde_json::to_vec(queue).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize pending work: {error}"))
    })?;
    state
        .profile
        .credential()
        .site(PENDING_WORK_CREDENTIAL_SITE)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("failed to save pending work: {error}")))
}

/// Record `work` for replay once the account confirms its email, then
/// try to drain immediately — an already-active customer must not wait
/// for the next status probe.
pub(crate) async fn defer(
    state: &crate::worker::TonkState,
    work: PendingWork,
) -> Result<(), TonkWorkerError> {
    let mut queue = load_pending(state).await?;
    queue.push(work);
    save_pending(state, &queue).await?;
    drain_pending(state).await;
    Ok(())
}

/// Replay queued work in the order it was recorded, stopping at the
/// first entry that does not complete.
///
/// Stopping rather than skipping is the ordering invariant: a custody
/// cell may only be published once its DID is provisioned, so an entry
/// that fails must hold back everything behind it. Best effort by
/// design — a drain that cannot run leaves the queue exactly as it was,
/// and the next probe tries again.
pub(crate) async fn drain_pending(state: &crate::worker::TonkState) {
    let mut queue = match load_pending(state).await {
        Ok(queue) => queue,
        Err(error) => return log!("pending work unreadable: {error}"),
    };
    if queue.is_empty() {
        return;
    }

    let mut completed = 0;
    for work in queue.entries() {
        match run_pending(state, work).await {
            Ok(()) => completed += 1,
            Err(error) => {
                log!("pending work for {} still waiting: {error}", work.subject());
                break;
            }
        }
    }
    if completed == 0 {
        return;
    }
    queue.retain_after(completed);
    if let Err(error) = save_pending(state, &queue).await {
        // The work itself succeeded; failing to shorten the queue only
        // costs a repeat, and every entry is idempotent.
        log!("pending work ran but the queue could not be shortened: {error}");
    }
}

/// Perform one queued entry.
///
/// A custody publish needs the PRF-derived custody key, which only a
/// page holding a fresh passkey assertion has, so the worker cannot run
/// one and reports it as still waiting. That halts the drain by design:
/// the publish must not be overtaken by entries recorded after it.
async fn run_pending(
    state: &crate::worker::TonkState,
    work: &PendingWork,
) -> Result<(), TonkWorkerError> {
    match work {
        PendingWork::Provision {
            consumer,
            consent_hex,
            consumer_kind,
        } => {
            let consumer: dialog_varsig::Did = consumer.parse().map_err(|error| {
                TonkWorkerError::Router(format!("queued consumer DID is invalid: {error:?}"))
            })?;
            let bytes = hex::decode(consent_hex).map_err(|error| {
                TonkWorkerError::Router(format!("queued consent is not hex: {error}"))
            })?;
            let consent =
                dialog_ucan_core::DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
                    TonkWorkerError::Router(format!("queued consent does not decode: {error}"))
                })?;
            provision_consumer(state, &consumer, &consent, consumer_kind.as_deref()).await
        }
        PendingWork::PublishCustody { custody, .. } => Err(TonkWorkerError::Router(format!(
            "custody publish for {custody} needs a passkey assertion, so a page must run it"
        ))),
    }
}

/// `GET /api/customer/pending` → the queued work a page may have to
/// run: the account panel reads this to learn whether it must raise a
/// passkey assertion and publish a custody cell.
#[wasm_compat]
pub async fn get_pending(
    State(state): State<AppState>,
) -> Result<Json<PendingQueue>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(load_pending(&state).await?))
}

/// `POST /api/customer/pending/custody` request body: the custody DID
/// whose queued publish a page just completed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedCustodyRequest {
    /// The custody space whose cell is now published.
    pub custody: String,
}

/// `POST /api/customer/pending/custody` → drop the queued publish for
/// `custody`, which a page completed with its own assertion, then drain
/// whatever it was holding back.
#[wasm_compat]
pub async fn complete_pending_custody(
    State(state): State<AppState>,
    Json(request): Json<CompletedCustodyRequest>,
) -> Result<Json<()>, TonkWorkerError> {
    let state = state.read().await;
    let mut queue = load_pending(&state).await?;
    let before = queue.len();
    queue.0.retain(|work| {
        !matches!(work, PendingWork::PublishCustody { custody, .. } if custody == &request.custody)
    });
    if queue.len() != before {
        save_pending(&state, &queue).await?;
    }
    drain_pending(&state).await;
    Ok(Json(()))
}

pub(crate) async fn clear_customer(
    state: &crate::worker::TonkState,
) -> Result<(), TonkWorkerError> {
    state
        .profile
        .credential()
        .site(CUSTOMER_CREDENTIAL_SITE)
        .save(Vec::<u8>::new())
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to clear the customer record: {error}"))
        })
}
