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

    save_customer(
        state,
        &CustomerRecord {
            customer: receipt.customer.to_string(),
            email: email.clone(),
            status: receipt.status,
            enrolled_at: Timestamp::now().to_unix(),
        },
    )
    .await?;
    // The same answer as a fact on profile main, so every device on this
    // account reads the registration state by query rather than by
    // probing the service itself.
    //
    // The sync endpoint comes from the receipt: the SERVICE decides
    // where its customers sync and says so, rather than each client
    // deriving an address from whichever origin it happened to reach.
    // An older service that answers no remote leaves whatever was
    // recorded before in place.
    if let Err(error) =
        record_customer_status(state, receipt.status, &email, receipt.provider.as_deref()).await
    {
        log!("account customer status not recorded: {error}");
    }
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
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{AccountCustomer, prelude::DidExt as _};

    let account = super::identity::root_did(state).await.ok()?;
    let branch = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
        .ok()?;
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
        .ok()?;
    rows.into_iter().next()
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
    // An absent address means "unchanged", not "no provider": a service
    // that predates the field must not blank one a previous receipt
    // already recorded.
    let provider = match provider {
        Some(provider) => provider.to_owned(),
        None => provider_address(state).await.unwrap_or_default(),
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
