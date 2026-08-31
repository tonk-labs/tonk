//! Customer registration with the access service.
//!
//! Enrollment runs after the account attaches: the device signs a
//! `/customer/enroll` invocation on the account's subject through its
//! `root → device` grant, depositing a delegation that lets the service
//! write into the account space, and the service emails an activation
//! link. Everything is same-origin — the access service is the
//! deployment serving this page — so endpoints derive from the request
//! origin and the service DID comes from `/.well-known/tonk`.

use async_trait::async_trait;
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
/// Enroll this profile's account as a customer, and record the answer.
///
/// The work behind both the `tonk:enroll` command and the route that
/// predates it. Takes plain values rather than extractors so a command
/// handler, which has no request to pull them from, can call it.
///
/// `email` absent means the account's own recorded address, which is
/// what the login and resend paths want. Empty `deposits` means no
/// ceremony is at hand, so the service is offered a device-issued set
/// chained through the `root -> device` grant instead.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
pub(crate) async fn enroll_customer(
    state: &crate::worker::TonkState,
    origin: &url::Url,
    email: Option<String>,
    deposits: &[String],
) -> Result<Receipt, TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let root_did = link.issuer().clone();
    let email = match email {
        Some(email) => email,
        // The login path names no address: the account's recorded email
        // is authoritative there.
        None => super::account_devices::account_summary(state)
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
    let body = if deposits.is_empty() {
        let service_did = service_did(origin).await?;
        build_enroll_invocation(device, &link, &service_did, &email).await
    } else {
        let deposits = deposits
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

    let endpoint = ucan_endpoint(origin)?;
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
                // Synthesized locally, so it names no service-provided
                // provider; whatever was recorded before stands.
                provider: None,
            }
        }
        Err(error) => return Err(error.into()),
    };
    if receipt.customer != root_did {
        return Err(TonkWorkerError::Internal(
            "enrollment receipt named a different account".to_string(),
        ));
    }

    let record = CustomerRecord {
        customer: receipt.customer.to_string(),
        email: email.clone(),
        status: receipt.status,
        enrolled_at: Timestamp::now().to_unix(),
    };
    // The same answer as a fact on profile main, so every device on this
    // account reads the registration state by query rather than by
    // probing the service itself.
    //
    // The sync endpoint comes from the receipt: the SERVICE decides
    // where its customers sync and says so, rather than each client
    // deriving an address from whichever origin it happened to reach.
    // An older service that answers no remote leaves whatever was
    // recorded before in place.
    persist_customer_projections(
        &WorkerCustomerProjectionPort(state),
        &record,
        receipt.provider.as_deref(),
    )
    .await?;
    Ok(receipt)
}

/// Runs the `tonk:enroll` command.
///
/// The outcome is the `AccountCustomer` fact the core already writes, so
/// there is nothing to answer: a caller that used to await the receipt
/// subscribes to that fact instead, and sees the same state arrive on
/// every other tab and device at the same time.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct EnrollCustomerHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl EnrollCustomerHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::EnrollCustomer::trigger_attributes(),
        }
    }
}

/// The address and deposits a `tonk:enroll` transient carries.
///
/// Both fields are optional in meaning though present on the wire: a
/// command's fields are scalars and a concept resolves only when every
/// one is there, so "unset" is the empty string. Empty email means the
/// account's recorded address; empty deposits mean no ceremony is at
/// hand.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_enrollment(facts: &crate::reactor::EntityFacts) -> Option<(Option<String>, Vec<String>)> {
    use crate::reactor::Decode as _;
    let command = facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::EnrollCustomer::decode(entity, facts))?;
    let email = (!command.email.0.trim().is_empty()).then(|| command.email.0.trim().to_owned());
    let deposits = command
        .deposits
        .0
        .split(',')
        .map(str::trim)
        .filter(|deposit| !deposit.is_empty())
        .map(str::to_owned)
        .collect();
    Some((email, deposits))
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for EnrollCustomerHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_enrollment(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let decoded = decode_enrollment(facts);
        let env = env.clone();
        Box::pin(async move {
            let Some((email, deposits)) = decoded else {
                log!("tonk:enroll: unparseable command; skipping");
                return;
            };
            let tonk = env.state().read().await;
            // The access service is same-origin: it serves this page, so
            // the deployment enrollment registers with is the one the
            // user is actually on. A command has no request to read that
            // from, so it comes from the worker's own scope.
            let origin = match service_origin() {
                Ok(origin) => origin,
                Err(error) => {
                    log!("tonk:enroll: {error}");
                    return;
                }
            };
            match enroll_customer(&tonk, &origin, email, &deposits).await {
                Ok(receipt) => log!("tonk:enroll: {} is {:?}", receipt.customer, receipt.status),
                Err(error) => log!("tonk:enroll failed: {error}"),
            }
        })
    }
}

/// POST `/api/customer/activated` → record the receipt the activation
/// page received.
///
/// Activation happens on a page that posts the emailed invocation
/// straight to the service's `/ucan/` endpoint, so the receipt — and
/// with it the provider address the service names — lands somewhere the
/// worker never sees. Without this the fact is written only when
/// something later calls the status probe, which leaves a just-activated
/// account creating local-only spaces because nothing recorded who
/// serves it.
///
/// Takes the receipt rather than re-deriving anything: the service
/// already said who it is, and this is where that answer is kept.
#[wasm_compat]
pub async fn activated(
    State(state): State<AppState>,
    Json(receipt): Json<Receipt>,
) -> Result<Json<()>, TonkWorkerError> {
    let state = state.read().await;
    let email = load_customer(&state)
        .await?
        .map(|record| record.email)
        .unwrap_or_default();
    record_customer_status(&state, receipt.status, &email, receipt.provider.as_deref()).await?;
    // Anything held back while the account was unserved can run now.
    drain_pending(&state).await;
    Ok(Json(()))
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
            // Reconcile the FACT on every probe, not only when the
            // status changed. Two guards used to stand in the way: a
            // missing local record (this device may never have enrolled)
            // and an unchanged status. The second is the one that bit —
            // by the time anything reads the provider, some earlier
            // probe has already flipped the stored record to `Active`,
            // so the statuses match forever after and the address is
            // never written. The probe is the only place a device
            // learns the provider without enrolling, so it has to write
            // what it learned every time.
            let email = record
                .as_ref()
                .map(|record| record.email.clone())
                .unwrap_or_default();
            if let Err(error) =
                record_customer_status(&state, receipt.status, &email, receipt.provider.as_deref())
                    .await
            {
                log!("account customer status not recorded: {error}");
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
    /// Hex-encoded pre-signed publish invocation, minted by the
    /// ceremony that sealed the envelope.
    pub invocation_hex: String,
}

/// POST `/api/custody/queue` → record a custody cell for publication
/// once its space is provisioned and the account confirms its email.
/// The ceremony pre-signed the publish invocation, so the worker drains
/// this itself — no page, no assertion. The cell is also recorded on
/// profile main right away: the account's own sync is the durability
/// channel, and the vault copy only bootstraps a brand-new browser.
#[wasm_compat]
pub async fn queue_custody(
    State(state): State<AppState>,
    Json(request): Json<QueueCustodyRequest>,
) -> Result<Json<()>, TonkWorkerError> {
    let state = state.read().await;
    record_custody_cell(&state, &request.custody, &request.sealed_hex).await?;
    // An empty invocation means the ceremony already published the cell
    // (a passkey enrolled on an active account): the record above is
    // all that was left to do.
    if !request.invocation_hex.is_empty() {
        defer(
            &state,
            PendingWork::PublishCustody {
                custody: request.custody,
                sealed_hex: request.sealed_hex,
                invocation_hex: request.invocation_hex,
            },
        )
        .await?;
        // Nothing may be waiting on activation at all: a provisioning
        // deposit queued just ahead of this can land right away.
        drain_pending(&state).await;
    }
    Ok(Json(()))
}

/// Record `custody`'s sealed cell on profile main, so the account's own
/// sync carries the recovery envelope to every device that holds the
/// profile.
async fn record_custody_cell(
    state: &crate::worker::TonkState,
    custody: &str,
    sealed_hex: &str,
) -> Result<(), TonkWorkerError> {
    let custody: dialog_varsig::Did = custody
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("custody DID is invalid: {error:?}")))?;
    let cell = hex::decode(sealed_hex)
        .map_err(|error| TonkWorkerError::Router(format!("sealed cell is not hex: {error}")))?;
    let account = super::identity::root_did(state).await?;
    state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(tonk_schema::CustodyCell::new(custody, account, cell))
        .commit()
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to record the custody cell: {error}"))
        })?;
    Ok(())
}

/// Persist the complete first-custody handoff in recovery-safe order.
///
/// The local sealed cell is written first. The provision and publish entries
/// are then appended to one queue value in that exact order and saved once,
/// after which draining is best effort. Repeating the operation is safe:
/// branch assertions are idempotent and [`PendingQueue::push_all`] suppresses
/// exact duplicates without changing order.
pub(crate) async fn persist_custody_setup(
    state: &crate::worker::TonkState,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> Result<(), TonkWorkerError> {
    record_custody_cell(state, custody, sealed_hex).await?;
    append_pending(
        state,
        [
            PendingWork::Provision {
                consumer: custody.to_owned(),
                consent_hex: consent_hex.to_owned(),
                consumer_kind: Some("custody".to_owned()),
            },
            PendingWork::PublishCustody {
                custody: custody.to_owned(),
                sealed_hex: sealed_hex.to_owned(),
                invocation_hex: invocation_hex.to_owned(),
            },
        ],
    )
    .await?;
    drain_pending(state).await;
    Ok(())
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

/// Exact local enrollment observation used before replaying an at-least-once
/// enrollment effect. An unreadable record is a mismatch here, never absence:
/// the recovery saga must not erase ambiguity by resubmitting authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomerObservation {
    Missing,
    Exact,
    Mismatch,
}

#[derive(Clone)]
pub(crate) enum CustomerRecordProjection {
    Missing,
    Present(CustomerRecord),
    Corrupt,
}

#[derive(Clone)]
pub(crate) struct AccountCustomerProjection {
    pub(crate) status: String,
    pub(crate) email: String,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub(crate) trait CustomerProjectionPort {
    async fn load_customer_record(&self) -> Result<CustomerRecordProjection, TonkWorkerError>;
    async fn load_account_customer(
        &self,
    ) -> Result<Option<AccountCustomerProjection>, TonkWorkerError>;
    async fn save_customer_record(&self, record: &CustomerRecord) -> Result<(), TonkWorkerError>;
    async fn save_account_customer(
        &self,
        status: CustomerStatus,
        email: &str,
        provider: Option<&str>,
    ) -> Result<(), TonkWorkerError>;
}

struct WorkerCustomerProjectionPort<'a>(&'a crate::worker::TonkState);

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl CustomerProjectionPort for WorkerCustomerProjectionPort<'_> {
    async fn load_customer_record(&self) -> Result<CustomerRecordProjection, TonkWorkerError> {
        let bytes = match self
            .0
            .profile
            .credential()
            .site(CUSTOMER_CREDENTIAL_SITE)
            .load::<Vec<u8>>()
            .perform(&self.0.operator)
            .await
        {
            Ok(bytes) if bytes.is_empty() => return Ok(CustomerRecordProjection::Missing),
            Ok(bytes) => bytes,
            Err(error) if crate::credential::is_missing(&error) => {
                return Ok(CustomerRecordProjection::Missing);
            }
            Err(error) => {
                return Err(TonkWorkerError::Internal(format!(
                    "failed to load the customer record: {error}"
                )));
            }
        };
        Ok(match serde_json::from_slice(&bytes) {
            Ok(record) => CustomerRecordProjection::Present(record),
            Err(_) => CustomerRecordProjection::Corrupt,
        })
    }

    async fn load_account_customer(
        &self,
    ) -> Result<Option<AccountCustomerProjection>, TonkWorkerError> {
        Ok(load_account_customer(self.0)
            .await?
            .map(|record| AccountCustomerProjection {
                status: record.status.0,
                email: record.email.0,
            }))
    }

    async fn save_customer_record(&self, record: &CustomerRecord) -> Result<(), TonkWorkerError> {
        save_customer(self.0, record).await
    }

    async fn save_account_customer(
        &self,
        status: CustomerStatus,
        email: &str,
        provider: Option<&str>,
    ) -> Result<(), TonkWorkerError> {
        record_customer_status(self.0, status, email, provider).await
    }
}

/// Persist the device-local enrollment record before the shared profile-main
/// projection. A retry repeats both idempotent writes; advancing the setup saga
/// is left to an exact observation after both have succeeded.
pub(crate) async fn persist_customer_projections(
    port: &impl CustomerProjectionPort,
    record: &CustomerRecord,
    provider: Option<&str>,
) -> Result<(), TonkWorkerError> {
    port.save_customer_record(record).await?;
    port.save_account_customer(record.status, &record.email, provider)
        .await
}

pub(crate) async fn observe_customer_projections(
    port: &impl CustomerProjectionPort,
    expected_root: &str,
    expected_email: &str,
) -> Result<CustomerObservation, TonkWorkerError> {
    let local = port.load_customer_record().await?;
    let projected = port.load_account_customer().await?;
    match (local, projected) {
        (CustomerRecordProjection::Missing, None) => Ok(CustomerObservation::Missing),
        (CustomerRecordProjection::Corrupt, _) => Ok(CustomerObservation::Mismatch),
        (CustomerRecordProjection::Present(local), Some(projected))
            if local.customer == expected_root
                && local.email == expected_email
                && projected.email == expected_email
                && projected.status == local.status.as_str() =>
        {
            Ok(CustomerObservation::Exact)
        }
        // One projection can survive a crash between the two writes. Replay
        // the idempotent enrollment so both become durable before advancing.
        (CustomerRecordProjection::Missing, Some(projected))
            if projected.email == expected_email =>
        {
            Ok(CustomerObservation::Missing)
        }
        (CustomerRecordProjection::Present(local), None)
            if local.customer == expected_root && local.email == expected_email =>
        {
            Ok(CustomerObservation::Missing)
        }
        _ => Ok(CustomerObservation::Mismatch),
    }
}

pub(crate) async fn observe_customer(
    state: &crate::worker::TonkState,
    expected_root: &str,
    expected_email: &str,
) -> Result<CustomerObservation, TonkWorkerError> {
    observe_customer_projections(
        &WorkerCustomerProjectionPort(state),
        expected_root,
        expected_email,
    )
    .await
}

/// Whether a provider actually serves this account — the precondition
/// for wiring a space to a remote.
///
/// Every other [`Registration`] answers `false`: awaiting activation,
/// suspended, and never registered are all states in which the access
/// service's provisioning gate refuses the subject, so attaching an
/// upstream would produce a space that syncs to a 403. Failing closed
/// costs a local-only space the share button can still sync; failing
/// open costs a space wired to a remote that refuses it.
///
/// Callers that need to explain WHY should read [`registration`]
/// directly — this collapses four states into a yes/no.
pub(crate) async fn is_active(state: &crate::worker::TonkState) -> bool {
    matches!(registration(state).await, Registration::Served { .. })
}

/// The provider serving this account, from the fact on profile main.
///
/// The one answer every attach path should use: the service names it in
/// the registration receipt, so it does not depend on which origin a
/// later request arrives on, and it reaches other devices through sync
/// rather than being re-derived from each page's own location.
pub(crate) async fn provider_address(state: &crate::worker::TonkState) -> Option<String> {
    account_customer(state).await?.provider().map(str::to_owned)
}

/// How far this account got through registering with a provider.
///
/// The share flow's whole decision, in one read. A space with no remote
/// cannot be shared, and what to do about it depends entirely on this:
/// attach and go, finish confirming an email, or register from scratch.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Registration {
    /// No provider and no enrollment: nothing has been registered.
    Unregistered,
    /// Enrolled, but the activation email has not been confirmed, so no
    /// provider serves this account yet. `email` is where the link went.
    AwaitingActivation {
        /// The address the activation link was sent to.
        email: String,
    },
    /// Registered and served. `provider` is where spaces attach.
    Served {
        /// The provider serving this account.
        provider: String,
    },
    /// Registered, but service was withdrawn. No email confirms this
    /// away; it is terminal until an operator says otherwise.
    Suspended,
}

/// Read how far this account got through registering.
///
/// The provider address is the primary signal, because the service names
/// it only once it actually serves the customer (see the access
/// service's `enroll`, which deliberately answers none). So "has a
/// provider" is "completed registration", and status only refines what
/// an absent one means.
pub(crate) async fn registration(state: &crate::worker::TonkState) -> Registration {
    let Some(customer) = account_customer(state).await else {
        return Registration::Unregistered;
    };
    if customer.status.0 == "Suspended" {
        return Registration::Suspended;
    }
    match customer.provider() {
        Some(provider) => Registration::Served {
            provider: provider.to_owned(),
        },
        // Active with no recorded address: the status write landed
        // before the one carrying the provider, which happens when a
        // space is created in the moment right after activation. The
        // account is served — the status says so — and the caller
        // resolves the address elsewhere, so this must not read as
        // "awaiting activation" and leave the space local-only.
        None if customer.status.0 == "Active" => Registration::Served {
            provider: String::new(),
        },
        // Enrolled far enough to record an address, but not far enough
        // to be served: the activation link is still unclicked.
        None if !customer.email.0.is_empty() => Registration::AwaitingActivation {
            email: customer.email.0.clone(),
        },
        None => Registration::Unregistered,
    }
}

/// This account's registration fact, absent when nothing recorded one.
pub(crate) async fn account_customer(
    state: &crate::worker::TonkState,
) -> Option<tonk_schema::AccountCustomer> {
    load_account_customer(state).await.ok().flatten()
}

async fn load_account_customer(
    state: &crate::worker::TonkState,
) -> Result<Option<tonk_schema::AccountCustomer>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{AccountCustomer, prelude::DidExt as _};

    let account = super::identity::root_did(state).await?;
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("acquire account branch: {error}")))?;
    let rows: Vec<AccountCustomer> = branch
        .handle()
        .query()
        .select(Query::<AccountCustomer> {
            this: Term::from(account.this()),
            status: Term::var("status"),
            email: Term::var("email"),
            provider: Term::var("provider"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("query account customer: {error}")))?;
    Ok(rows.into_iter().next())
}

/// Record the account's registration state as a fact on profile main,
/// so it reaches every device on the account.
///
/// Called wherever the service's answer is learned — enrollment, and the
/// status probe that notices activation — so the fact tracks the
/// service rather than drifting from it.
pub(crate) async fn record_customer_status(
    state: &crate::worker::TonkState,
    status: CustomerStatus,
    email: &str,
    provider: Option<&str>,
) -> Result<(), TonkWorkerError> {
    use tonk_schema::{AccountCustomer, prelude::DidExt as _};

    let account = super::identity::root_did(state).await?;
    // An absent address means "unchanged", not "no provider": a receipt
    // that names none — every enrollment receipt does — must not blank
    // one a later activation already recorded.
    let provider = match provider {
        Some(provider) => Some(provider.to_owned()),
        None => provider_address(state).await,
    };
    state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountCustomer::new(
            account.this(),
            status.as_str(),
            email.to_owned(),
            provider,
        ))
        .commit()
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("commit account customer status: {error}"))
        })?;

    // Activation (or enrollment, or a suspension) is a new answer about
    // the address, and the registration form reads that answer from the
    // overlay rather than from this row. Without writing it here an
    // address asked about BEFORE registering keeps its `unregistered`
    // answer forever, and the form goes on offering to create an
    // account for one that has just finished activating.
    //
    // Gated exactly as `mod email_status` is: the lookup it shares this
    // row with is the worker's own, so the module is not built for a
    // native non-test target and neither is the call into it.
    #[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
    super::email_status::record(
        state,
        email,
        super::email_status::state_for_customer(status),
    )
    .await;

    Ok(())
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
    serde_json::from_slice(&bytes).map_err(|error| {
        // Pending custody work is the only durable copy of a bounded publish
        // authorization after the passkey ceremony. Treat unreadable bytes as
        // an ambiguity to preserve, never as an empty queue that is safe to
        // overwrite.
        TonkWorkerError::Internal(format!("stored pending work is unreadable: {error}"))
    })
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

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
trait PendingQueueStore {
    async fn load_queue(&self) -> Result<PendingQueue, TonkWorkerError>;
    async fn save_queue(&self, queue: &PendingQueue) -> Result<(), TonkWorkerError>;
}

/// Exact status of the account-setup custody pair in the durable pending queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustodySetupObservation {
    /// Provision and/or publish is still recorded in a valid replay order.
    Pending,
    /// No entry for this custody subject remains after it was durably queued.
    Complete,
    /// Entries for the subject exist but do not reproduce the staged pair.
    Mismatch,
}

fn observe_custody_queue(
    queue: &PendingQueue,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> CustodySetupObservation {
    let mut provision_at = None;
    let mut publish_at = None;
    for (index, work) in queue.entries().iter().enumerate() {
        match work {
            PendingWork::Provision {
                consumer,
                consent_hex: recorded_consent,
                consumer_kind,
            } if consumer == custody => {
                if provision_at.is_some()
                    || recorded_consent != consent_hex
                    || consumer_kind.as_deref() != Some("custody")
                {
                    return CustodySetupObservation::Mismatch;
                }
                provision_at = Some(index);
            }
            PendingWork::PublishCustody {
                custody: recorded_custody,
                sealed_hex: recorded_sealed,
                invocation_hex: recorded_invocation,
            } if recorded_custody == custody => {
                if publish_at.is_some()
                    || recorded_sealed != sealed_hex
                    || recorded_invocation != invocation_hex
                {
                    return CustodySetupObservation::Mismatch;
                }
                publish_at = Some(index);
            }
            _ => {}
        }
    }

    match (provision_at, publish_at) {
        (None, None) => CustodySetupObservation::Complete,
        (None, Some(_)) => CustodySetupObservation::Pending,
        (Some(provision), Some(publish)) if provision < publish => CustodySetupObservation::Pending,
        (Some(_), None) | (Some(_), Some(_)) => CustodySetupObservation::Mismatch,
    }
}

fn replace_custody_publish_in_queue(
    queue: &mut PendingQueue,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    previous_invocation_hex: &str,
    replacement_invocation_hex: &str,
) -> (CustodySetupObservation, bool) {
    let mut replacement_at = None;
    for (index, work) in queue.0.iter().enumerate() {
        if let PendingWork::PublishCustody {
            custody: recorded_custody,
            sealed_hex: recorded_sealed,
            invocation_hex,
        } = work
            && recorded_custody == custody
            && recorded_sealed == sealed_hex
            && (invocation_hex == previous_invocation_hex
                || invocation_hex == replacement_invocation_hex)
        {
            if replacement_at.is_some() {
                return (CustodySetupObservation::Mismatch, false);
            }
            replacement_at = Some(index);
        }
    }

    let mut changed = false;
    if let Some(index) = replacement_at {
        let PendingWork::PublishCustody { invocation_hex, .. } = &mut queue.0[index] else {
            unreachable!("the selected entry was a custody publish")
        };
        if invocation_hex == previous_invocation_hex && invocation_hex != replacement_invocation_hex
        {
            *invocation_hex = replacement_invocation_hex.to_owned();
            changed = true;
        }
    }
    (
        observe_custody_queue(
            queue,
            custody,
            consent_hex,
            sealed_hex,
            replacement_invocation_hex,
        ),
        changed,
    )
}

struct WorkerPendingQueueStore<'a>(&'a crate::worker::TonkState);

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl PendingQueueStore for WorkerPendingQueueStore<'_> {
    async fn load_queue(&self) -> Result<PendingQueue, TonkWorkerError> {
        load_pending(self.0).await
    }

    async fn save_queue(&self, queue: &PendingQueue) -> Result<(), TonkWorkerError> {
        save_pending(self.0, queue).await
    }
}

async fn append_to_pending_store_locked(
    store: &impl PendingQueueStore,
    work: impl IntoIterator<Item = PendingWork>,
) -> Result<(), TonkWorkerError> {
    let mut queue = store.load_queue().await?;
    queue.push_all(work);
    store.save_queue(&queue).await
}

struct PendingMutationGuard {
    _cross_worker: super::browser_lock::CrossWorkerGuard,
    _local: tokio::sync::OwnedMutexGuard<()>,
}

#[derive(Clone)]
struct PendingMutationScope {
    lock_name: String,
    local: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl PendingMutationScope {
    fn new(profile_did: &[u8], local: std::sync::Arc<tokio::sync::Mutex<()>>) -> Self {
        let profile_hash = blake3::hash(profile_did);
        Self {
            lock_name: format!("tonk-pending-effects-v1:{profile_hash}"),
            local,
        }
    }

    fn for_state(state: &crate::worker::TonkState) -> Self {
        Self::new(
            state.profile.did().as_ref().as_bytes(),
            state.pending_work_lock.clone(),
        )
    }

    async fn acquire(&self) -> Result<PendingMutationGuard, TonkWorkerError> {
        let cross_worker = super::browser_lock::acquire(&self.lock_name)
            .await
            .map_err(|()| {
                TonkWorkerError::Internal(
                    "pending effects require cross-worker serialization; reload and retry"
                        .to_string(),
                )
            })?;
        let local = self.local.clone().lock_owned().await;
        Ok(PendingMutationGuard {
            _cross_worker: cross_worker,
            _local: local,
        })
    }
}

async fn acquire_pending_mutation(
    state: &crate::worker::TonkState,
) -> Result<PendingMutationGuard, TonkWorkerError> {
    PendingMutationScope::for_state(state).acquire().await
}

/// The sole append seam for the durable pending queue. Both lock layers are
/// acquired here, so production routes and concurrency tests cannot provide a
/// test-only mutex around an otherwise racy load/append/save sequence.
async fn append_to_pending_store(
    scope: &PendingMutationScope,
    store: &impl PendingQueueStore,
    work: impl IntoIterator<Item = PendingWork>,
) -> Result<(), TonkWorkerError> {
    let _mutation = scope.acquire().await?;
    append_to_pending_store_locked(store, work).await
}

async fn observe_custody_in_pending_store(
    scope: &PendingMutationScope,
    store: &impl PendingQueueStore,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> Result<CustodySetupObservation, TonkWorkerError> {
    let _mutation = scope.acquire().await?;
    let queue = store.load_queue().await?;
    Ok(observe_custody_queue(
        &queue,
        custody,
        consent_hex,
        sealed_hex,
        invocation_hex,
    ))
}

async fn replace_custody_publish_in_pending_store(
    scope: &PendingMutationScope,
    store: &impl PendingQueueStore,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    previous_invocation_hex: &str,
    replacement_invocation_hex: &str,
) -> Result<CustodySetupObservation, TonkWorkerError> {
    let _mutation = scope.acquire().await?;
    let mut queue = store.load_queue().await?;
    let (observation, changed) = replace_custody_publish_in_queue(
        &mut queue,
        custody,
        consent_hex,
        sealed_hex,
        previous_invocation_hex,
        replacement_invocation_hex,
    );
    if changed {
        store.save_queue(&queue).await?;
    }
    Ok(observation)
}

/// Record `work` for replay once the account confirms its email, then
/// try to drain immediately — an already-active customer must not wait
/// for the next status probe.
pub(crate) async fn defer(
    state: &crate::worker::TonkState,
    work: PendingWork,
) -> Result<(), TonkWorkerError> {
    append_pending(state, [work]).await?;
    drain_pending(state).await;
    Ok(())
}

/// Observe an already-queued account-setup custody pair under the same lock
/// used by every append and drain. Absence is meaningful only after the saga's
/// `CustodyQueued` checkpoint proves the pair was saved successfully.
pub(crate) async fn observe_custody_setup(
    state: &crate::worker::TonkState,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> Result<CustodySetupObservation, TonkWorkerError> {
    observe_custody_in_pending_store(
        &PendingMutationScope::for_state(state),
        &WorkerPendingQueueStore(state),
        custody,
        consent_hex,
        sealed_hex,
        invocation_hex,
    )
    .await
}

/// Replace only the matching queued custody publish authorization, retaining
/// its position behind Provision. If activation already drained the pair,
/// absence remains Complete and the replacement is not re-queued.
pub(crate) async fn replace_custody_publish(
    state: &crate::worker::TonkState,
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    previous_invocation_hex: &str,
    replacement_invocation_hex: &str,
) -> Result<CustodySetupObservation, TonkWorkerError> {
    replace_custody_publish_in_pending_store(
        &PendingMutationScope::for_state(state),
        &WorkerPendingQueueStore(state),
        custody,
        consent_hex,
        sealed_hex,
        previous_invocation_hex,
        replacement_invocation_hex,
    )
    .await
}

async fn append_pending(
    state: &crate::worker::TonkState,
    work: impl IntoIterator<Item = PendingWork>,
) -> Result<(), TonkWorkerError> {
    append_to_pending_store(
        &PendingMutationScope::for_state(state),
        &WorkerPendingQueueStore(state),
        work,
    )
    .await
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
    let _mutation = match acquire_pending_mutation(state).await {
        Ok(guard) => guard,
        Err(error) => return log!("pending work lock unavailable: {error}"),
    };
    drain_pending_locked(state).await;
}

async fn drain_pending_locked(state: &crate::worker::TonkState) {
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
/// A custody publish uses the bounded pre-signed invocation captured while
/// the page held the PRF-derived custody key. That lets the worker replay it
/// later without reopening the key. A failed provision or publish still
/// halts the drain so no later entry can overtake the custody pair.
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
        PendingWork::PublishCustody {
            custody,
            sealed_hex,
            invocation_hex,
        } => {
            if invocation_hex.is_empty() {
                // An entry from before invocations were queued: nothing
                // can sign for it any more, so it is void rather than a
                // block on the queue.
                log!("dropping a custody publish for {custody} queued without an invocation");
                return Ok(());
            }
            let invocation = hex::decode(invocation_hex).map_err(|error| {
                TonkWorkerError::Router(format!("queued invocation is not hex: {error}"))
            })?;
            let sealed = hex::decode(sealed_hex).map_err(|error| {
                TonkWorkerError::Router(format!("queued cell is not hex: {error}"))
            })?;
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            {
                let origin = service_origin()?;
                let endpoint = ucan_endpoint(&origin)?;
                tonk_identity::custody::submit_publish(&invocation, &sealed, endpoint.as_str())
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!("custody publish for {custody}: {error}"))
                    })?;
                log!("custody cell for {custody} published from the queued invocation");
                Ok(())
            }
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            {
                let _ = (invocation, sealed);
                Err(TonkWorkerError::Internal(format!(
                    "the custody publish for {custody} drains in the worker runtime"
                )))
            }
        }
    }
}

/// `GET /api/customer/pending` → the queued work still waiting: the
/// account panel reads this to say a backup is on its way; the worker
/// itself drains it.
#[wasm_compat]
pub async fn get_pending(
    State(state): State<AppState>,
) -> Result<Json<PendingQueue>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(load_pending(&state).await?))
}

/// Record a customer at `status`, so a test can put the profile in the
/// state the gate reads without standing up an access service.
///
/// Writes the fact on profile main — what [`is_active`] actually reads —
/// alongside the device-local record the account panel renders, so a
/// test sets up the same pair the enrollment path does.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn record_test_customer(
    state: &crate::worker::TonkState,
    status: CustomerStatus,
) -> Result<(), TonkWorkerError> {
    const EMAIL: &str = "customer@example.test";
    const PROVIDER: &str = "https://example.test/ucan/";
    let customer = super::identity::root_did(state).await?.to_string();
    save_customer(
        state,
        &CustomerRecord {
            customer,
            email: EMAIL.to_owned(),
            status,
            enrolled_at: 0,
        },
    )
    .await?;
    // A provider only for a served customer, mirroring what the access
    // service actually answers: enrollment names none, because an
    // unactivated customer gets neither service nor provisioning. A
    // fixture that recorded one anyway would make `Registered`
    // indistinguishable from `Active` and quietly defeat the gate the
    // tests exist to check.
    let provider = matches!(status, CustomerStatus::Active).then_some(PROVIDER);
    record_customer_status(state, status, EMAIL, provider).await
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use tonk_account::pending::{PendingQueue, PendingWork};

    use super::{
        CustodySetupObservation, PendingMutationScope, PendingQueueStore, append_to_pending_store,
        observe_custody_in_pending_store, replace_custody_publish_in_pending_store,
    };

    #[derive(Default)]
    struct MemoryPendingQueueStore(Mutex<PendingQueue>);

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl PendingQueueStore for MemoryPendingQueueStore {
        async fn load_queue(&self) -> Result<PendingQueue, crate::TonkWorkerError> {
            let queue = self.0.lock().unwrap().clone();
            tokio::task::yield_now().await;
            Ok(queue)
        }

        async fn save_queue(&self, queue: &PendingQueue) -> Result<(), crate::TonkWorkerError> {
            tokio::task::yield_now().await;
            *self.0.lock().unwrap() = queue.clone();
            Ok(())
        }
    }

    fn custody_batch(label: &str) -> [PendingWork; 2] {
        [
            PendingWork::Provision {
                consumer: format!("invalid-custody-{label}"),
                consent_hex: format!("aa{label}"),
                consumer_kind: Some("custody".to_string()),
            },
            PendingWork::PublishCustody {
                custody: format!("invalid-custody-{label}"),
                sealed_hex: format!("bb{label}"),
                invocation_hex: format!("cc{label}"),
            },
        ]
    }

    #[dialog_common::test]
    async fn production_lock_wrapper_keeps_concurrent_custody_batches_ordered() {
        let store = Arc::new(MemoryPendingQueueStore::default());
        // Distinct local mutexes model two live worker generations. Only the
        // canonical named-lock half of the production wrapper can serialize
        // them; a test-only shared mutex would hide the race under review.
        let left_scope = PendingMutationScope::new(
            b"did:key:zAccountProfile",
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let right_scope = PendingMutationScope::new(
            b"did:key:zAccountProfile",
            Arc::new(tokio::sync::Mutex::new(())),
        );
        let first = custody_batch("01");
        let second = custody_batch("02");

        let (left, right) = tokio::join!(
            append_to_pending_store(&left_scope, store.as_ref(), first.clone()),
            append_to_pending_store(&right_scope, store.as_ref(), second.clone()),
        );
        left.unwrap();
        right.unwrap();

        let queue = store.load_queue().await.unwrap();
        let entries = queue.entries();
        let first_then_second = first.into_iter().chain(second.clone()).collect::<Vec<_>>();
        let second_then_first = second
            .into_iter()
            .chain(custody_batch("01"))
            .collect::<Vec<_>>();
        assert!(
            entries == first_then_second || entries == second_then_first,
            "both ordered Provision+Publish pairs must survive either lock acquisition order"
        );
    }

    #[dialog_common::test]
    async fn production_mutator_replaces_publish_without_moving_it_ahead_of_provision() {
        let store = MemoryPendingQueueStore::default();
        let scope = PendingMutationScope::new(
            b"did:key:zAccountProfile",
            Arc::new(tokio::sync::Mutex::new(())),
        );
        append_to_pending_store(&scope, &store, custody_batch("01"))
            .await
            .unwrap();

        assert_eq!(
            replace_custody_publish_in_pending_store(
                &scope,
                &store,
                "invalid-custody-01",
                "aa01",
                "bb01",
                "cc01",
                "fresh01",
            )
            .await
            .unwrap(),
            CustodySetupObservation::Pending
        );
        // A lost response repeats the same mutation without a second entry or
        // a second receipt-worthy queue change.
        assert_eq!(
            replace_custody_publish_in_pending_store(
                &scope,
                &store,
                "invalid-custody-01",
                "aa01",
                "bb01",
                "cc01",
                "fresh01",
            )
            .await
            .unwrap(),
            CustodySetupObservation::Pending
        );
        assert_eq!(
            observe_custody_in_pending_store(
                &scope,
                &store,
                "invalid-custody-01",
                "aa01",
                "bb01",
                "fresh01",
            )
            .await
            .unwrap(),
            CustodySetupObservation::Pending
        );

        let queue = store.load_queue().await.unwrap();
        assert_eq!(queue.len(), 2);
        assert!(matches!(queue.entries()[0], PendingWork::Provision { .. }));
        assert!(matches!(
            &queue.entries()[1],
            PendingWork::PublishCustody { invocation_hex, .. }
                if invocation_hex == "fresh01"
        ));
    }
}
