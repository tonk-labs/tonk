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
use tonk_identity::request::build_enroll_invocation;
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
    /// The provider serving this account, from the registration fact on
    /// profile main; absent until enrollment records it.
    pub provider: Option<String>,
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
    custody: &tonk_identity::request::CustodyMaterial<'_>,
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

    let body = build_enroll_invocation(device, &link, &email, custody)
        .await
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
                // Synthesized locally, so it names neither a
                // service-provided provider nor a ledger space;
                // whatever was recorded before stands.
                provider: None,
                ledger: None,
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
    retain_ledger(state, &receipt).await;
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
/// What a `tonk:enroll` transient carries: the address and deposits, and
/// the custody material every enrollment must present.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct Enrollment {
    email: Option<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_enrollment(facts: &crate::reactor::EntityFacts) -> Option<Enrollment> {
    use crate::reactor::Decode as _;
    let command = facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::EnrollCustomer::decode(entity, facts))?;
    let email = (!command.email.0.trim().is_empty()).then(|| command.email.0.trim().to_owned());
    Some(Enrollment { email })
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
            let Some(enrollment) = decoded else {
                log!("tonk:enroll: unparseable command; skipping");
                return;
            };
            // Enrollment must present custody material, and minting it
            // needs a passkey the worker cannot prompt for. So the page
            // is asked to mediate: it runs one assertion and posts the
            // derivation handles back, and the handoff does the rest —
            // including this enrollment, which travels with it.
            let Some(client) = env.client().cloned() else {
                log!("tonk:enroll: no originating client to mediate a passkey; skipping");
                return;
            };
            super::custody::request_mediation(&client, enrollment.email).await;
        })
    }
}

/// Runs the `account/resend-activation` command: sign the self-subjected
/// `/customer/resend` invocation and post it.
///
/// No ceremony and no custody material — the enrollment's rows stand at
/// the service, and re-enrolling to get a mail re-runs a passkey prompt
/// the person waiting on their inbox never asked for. Outcome is the
/// mail itself; failures are logged, and the rate limit means a silent
/// round is the ordinary answer to an impatient second press.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct ResendActivationHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl ResendActivationHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::ResendActivation::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_resend(facts: &crate::reactor::EntityFacts) -> bool {
    use crate::reactor::Decode as _;
    facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::ResendActivation::decode(entity, facts))
        .is_some()
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for ResendActivationHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_resend(facts)
    }

    fn run(
        &self,
        _facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let env = env.clone();
        Box::pin(async move {
            let state = env.state().read().await;
            let account = match super::identity::root_did(&state).await {
                Ok(account) => account,
                Err(error) => {
                    log!("resend-activation: no account root: {error}");
                    return;
                }
            };
            let device = state.profile.signer().signer().clone();
            let body = match tonk_identity::request::build_resend_invocation(device, &account).await
            {
                Ok(body) => body,
                Err(error) => {
                    log!("resend-activation: invocation did not build: {error:#}");
                    return;
                }
            };
            let origin = match service_origin() {
                Ok(origin) => origin,
                Err(error) => {
                    log!("resend-activation: no service origin: {error}");
                    return;
                }
            };
            let endpoint = match ucan_endpoint(&origin) {
                Ok(endpoint) => endpoint,
                Err(error) => {
                    log!("resend-activation: {error}");
                    return;
                }
            };
            if let Err(error) = post_cbor(&endpoint, &body).await {
                log!("resend-activation: the service refused: {error}");
            }
        })
    }
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
            // The probe is also where a device that never enrolled
            // first sees the ledger grant, so it retains here too.
            retain_ledger(&state, &receipt).await;
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
    // Read after the probe: the probe is where a device that never
    // enrolled first records the provider, so this read sees it.
    let provider = provider_address(&state).await;
    Ok(Json(CustomerState {
        customer: root.to_string(),
        status,
        email: record.map(|record| record.email),
        provider,
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

fn decode_custody_consent(
    consent_hex: &str,
) -> Result<dialog_ucan_core::DelegationChain, TonkWorkerError> {
    let bytes = hex::decode(consent_hex)
        .map_err(|error| TonkWorkerError::Router(format!("consent is not hex: {error}")))?;
    dialog_ucan_core::DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("consent does not decode: {error}")))
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
    let consent = decode_custody_consent(&request.consent_hex)?;
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
    /// Consent for provisioning this custody space. Optional only so a page
    /// loaded before this worker version can finish its already-started flow.
    #[serde(default)]
    pub consent_hex: Option<String>,
    /// Hex-encoded sealed envelope to publish.
    pub sealed_hex: String,
    /// Hex-encoded pre-signed publish invocation, minted by the
    /// ceremony that sealed the envelope.
    pub invocation_hex: String,
}

fn custody_pending_batch(request: &QueueCustodyRequest) -> Vec<PendingWork> {
    let mut work = Vec::with_capacity(2);
    if let Some(consent_hex) = &request.consent_hex {
        work.push(PendingWork::Provision {
            consumer: request.custody.clone(),
            consent_hex: consent_hex.clone(),
            consumer_kind: Some("custody".to_owned()),
        });
    }
    if !request.invocation_hex.is_empty() {
        work.push(PendingWork::PublishCustody {
            custody: request.custody.clone(),
            sealed_hex: request.sealed_hex.clone(),
            invocation_hex: request.invocation_hex.clone(),
        });
    }
    work
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
    if let Some(consent_hex) = &request.consent_hex {
        // Reject malformed recovery material before it can poison the durable
        // queue. The access service still verifies its authority on replay.
        decode_custody_consent(consent_hex)?;
    }
    // No passkey facts on this path: the queue route carries a cell that
    // could not be published, not a ceremony's creation metadata. The
    // ceremony records its own row where it has both.
    record_custody_cell(
        &state,
        &request.custody,
        &request.sealed_hex,
        None,
        "",
        None,
    )
    .await?;
    let work = custody_pending_batch(&request);
    if !work.is_empty() {
        // Provision and publish are one durable ordered batch. Replaying this
        // request also restores a missing provision ahead of a surviving
        // publish from an interrupted earlier attempt.
        defer_all(&state, work).await?;
    }
    Ok(Json(()))
}

/// Record `custody`'s sealed cell on profile main, so the account's own
/// sync carries the recovery envelope to every device that holds the
/// profile.
pub(crate) async fn record_custody_cell(
    state: &crate::worker::TonkState,
    custody: &str,
    sealed_hex: &str,
    passkey: Option<tonk_worker_api::PasskeyMetadata>,
    credential_id: &str,
    // What the ceremony named the credential: the address, where one was
    // given. A passkey manager lists the entry under this, so it is what a
    // person recognises their own passkey by.
    name: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let custody: dialog_varsig::Did = custody
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("custody DID is invalid: {error:?}")))?;
    let cell = hex::decode(sealed_hex)
        .map_err(|error| TonkWorkerError::Router(format!("sealed cell is not hex: {error}")))?;
    let account = super::identity::root_did(state).await?;
    let mut transaction = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        // A message sealed to the passkey's custody DID: only a fresh
        // assertion of that passkey opens it. The account is a real
        // sender here — this envelope carries its own secret — so it is
        // one of the few places `sealed_by` earns its keep.
        .assert(tonk_schema::SecretMessage::new(&custody, cell).sealed_by(&account));
    // The passkey's own row, on the same entity the envelope is
    // addressed to. Written here because this is where the custody DID
    // and the creation metadata are both in hand: only the browser that
    // ran `credentials.create()` has the label, and only this call knows
    // which passkey it belongs to.
    if let Some(passkey) = passkey {
        let row = tonk_schema::RecoveryPasskey::new(
            &custody,
            credential_id,
            passkey.created_at,
            passkey.created_on,
        );
        let row = match name.filter(|name| !name.trim().is_empty()) {
            Some(name) => row.named(name, name),
            None => row,
        };
        transaction = transaction.assert(row);
    }
    transaction
        .commit()
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to record the sealed account secret: {error}"
            ))
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
/// activation.
///
/// `CustomerInactive` is not the only answer a replay resolves, and
/// treating it as the only one is what stranded accounts. Enrollment is
/// dispatched as a command and completes asynchronously, so a
/// provisioning call raced ahead of it reaches a service with no
/// customer row at all and is refused `UnknownCustomer`; a phone on a
/// flaky connection gets a timeout or a transport failure. Each of those
/// is a moment in time, not a verdict, and each used to drop the
/// consent — which for a custody space is unrecoverable, since only a
/// live passkey assertion can mint it.
///
/// So the queue is the default and only a refusal about the request
/// itself is terminal: a malformed consent, or a space another customer
/// already provides. Those do not become true by waiting.
pub(crate) async fn provision_or_defer(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
    consent: &dialog_ucan_core::DelegationChain,
    kind: Option<&str>,
) -> Result<(), TonkWorkerError> {
    match provision_consumer(state, consumer, consent, kind).await {
        Err(error) if is_retryable(&error) => {
            log!("{consumer} queued after a retryable refusal: {error}");
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

/// Whether a failed provisioning is worth replaying later.
///
/// Terminal are the refusals about the request itself — a consent that
/// does not verify, a consumer someone else provides, an argument that
/// does not parse. Everything else is about *when* the call happened:
/// the customer row not written yet, an unconfirmed email, a service
/// that was unreachable. Those clear on their own, and the drain is
/// idempotent, so queuing a call that turns out to be unnecessary costs
/// one round trip while dropping one costs the account.
pub(crate) fn is_retryable(error: &TonkWorkerError) -> bool {
    match error {
        TonkWorkerError::Upstream { code, status, .. } => match code.as_deref() {
            // The service's own state, not this request: the customer row
            // is not written yet, or the email is unconfirmed. Both clear
            // without anyone changing the call. Named rather than inferred
            // from the status, because both answer 4xx — which is what
            // made the status alone the wrong test.
            Some("UnknownCustomer" | "CustomerInactive") => true,
            // About the request, and no truer later: a consent that does
            // not verify, a consumer someone else provides, an argument
            // that does not parse, a suspension only an operator lifts.
            Some(
                "Forbidden" | "Unauthorized" | "Invalid" | "ConsumerProvided" | "CustomerSuspended"
                | "CustomerActive" | "UnknownConsumer",
            ) => false,
            // A refusal this client does not know: retry only when the
            // status says the service, not the request, was the problem.
            Some(_) | None => *status >= 500 || *status == 408 || *status == 429,
        },
        _ => false,
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
        Ok(_) => {
            // Only a SPACE lands in the directory. A custody namespace
            // is provisioned through the same command, but it is not a
            // directory row, and stamping one would hang provider facts
            // on an entity nothing lists.
            if matches!(kind, None | Some("space")) {
                record_space_provider(state, consumer).await;
            }
            Ok(())
        }
        Err(HttpError::Upstream(failure))
            if failure.code.as_deref() == Some("ConsumerProvided") =>
        {
            // Another customer provides it — the space is served, but
            // not under this account, so there is nothing to record.
            log!("consumer {consumer} already has a provider; leaving it");
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Record that this account provides `consumer`, on the space's
/// directory entity in the account db. The write side of the fact
/// [`space_provider_recorded`] reads: its presence is what lets a share
/// skip the `/provider/add` ceremony. Best-effort — provisioning stands
/// in D1 whether or not the fact records it, and the next share heals a
/// missing row by provisioning again.
pub(crate) async fn record_space_provider(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
) {
    use tonk_schema::SpaceProvider;

    let Ok(account) = super::identity::root_did(state).await else {
        return;
    };
    if let Err(error) = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(SpaceProvider::new(consumer, &account))
        .commit()
        .perform(&state.operator)
        .await
    {
        log!("space provider for {consumer} not recorded: {error}");
    }
}

/// Drop the record that this account provides `consumer` — the space is
/// back to local-only. Called when deprovisioning succeeds and when the
/// sync engine observes the gate refusing the subject with a denial
/// retrying cannot clear. Best-effort, like the record.
pub(crate) async fn retract_space_provider(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
) {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{SpaceProvider, prelude::DidExt as _};

    let branch = match state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("space provider for {consumer} not read: {error}");
            return;
        }
    };
    let recorded: Vec<SpaceProvider> = match branch
        .handle()
        .query()
        .select(Query::<SpaceProvider> {
            this: Term::from(consumer.this()),
            provider: Term::var("provider"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            log!("space provider for {consumer} not read: {error}");
            return;
        }
    };
    for row in recorded {
        if let Err(error) = branch
            .handle()
            .transaction()
            .retract(row)
            .commit()
            .perform(&state.operator)
            .await
        {
            log!("space provider for {consumer} not retracted: {error}");
        }
    }
}

/// Whether the account db records a provider for `consumer` — the read
/// a share consults instead of running `/provider/add` per click.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn space_provider_recorded(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
) -> bool {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{SpaceProvider, prelude::DidExt as _};

    let branch = match state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("space provider for {consumer} not read: {error}");
            return false;
        }
    };
    match branch
        .handle()
        .query()
        .select(Query::<SpaceProvider> {
            this: Term::from(consumer.this()),
            provider: Term::var("provider"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
    {
        Ok(rows) => {
            let rows: Vec<SpaceProvider> = rows;
            !rows.is_empty()
        }
        Err(error) => {
            log!("space provider for {consumer} not read: {error}");
            false
        }
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
    retract_space_provider(state, consumer).await;
    Ok(())
}

/// The worker's own origin, which the access service serves. Known only
/// inside a service-worker scope; callers outside one (native tests)
/// carry an origin of their own through `RequestOrigin` instead.
pub(crate) fn service_origin() -> Result<Url, TonkWorkerError> {
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

/// The same-origin `/ucan/` endpoint.
pub(crate) fn ucan_endpoint(origin: &Url) -> Result<Url, TonkWorkerError> {
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

/// Retain the read authority a receipt's ledger carries.
///
/// The service mints a `ledger -> account` `/use/get` chain and names
/// it in the receipt. Retaining it is how it becomes usable: dialog
/// stores a delegation content-addressed and decomposes issuer,
/// audience, subject and command onto its entity, so the grant is a
/// queryable fact on the account branch and reaches every other device
/// through sync. Nothing else stores it -- a hex string in a blob
/// beside the proof would be a second copy of an authority the proof
/// already carries.
///
/// Best-effort, like the space retain it mirrors: the ledger is
/// metering the account reads, not authority it needs to operate, so a
/// failure here must not fail the enrollment that carried it.
pub(crate) async fn retain_ledger(state: &crate::worker::TonkState, receipt: &Receipt) {
    let Some(ledger) = &receipt.ledger else {
        return;
    };
    let bytes = match hex::decode(&ledger.read_hex) {
        Ok(bytes) => bytes,
        Err(error) => {
            log!("ledger read grant is not hex: {error}");
            return;
        }
    };
    let chain = match dialog_ucan_core::DelegationChain::try_from(bytes.as_slice()) {
        Ok(chain) => chain,
        Err(error) => {
            log!("ledger read grant did not decode: {error}");
            return;
        }
    };
    super::account_state::retain_space_delegation(state, &chain).await;
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
    account_registration(state).await.provider
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

/// Record that this account is served, because the gate said so.
///
/// The remote is attached from enrollment and the provisioning gate
/// refuses an unconfirmed customer, so a sync that succeeds IS the
/// activation — no status endpoint is asked, and no emailed link has to
/// be opened on THIS device for it to learn.
///
/// Only an account still reading `registered` is watched. Activation is a
/// one-way transition, so once the fact is there nothing is waiting on it
/// and there is nothing left to observe: this returns before touching the
/// database. That is what keeps the sweep from re-asserting a
/// cardinality-one row on every heartbeat for the life of the account,
/// each one a transaction, a branch head and a push to record something
/// that did not change.
///
/// This is also why only the device holding the account remote writes it.
/// The others have no sync to learn from and do not guess: they wait on
/// the fact arriving, or on their own custody read clearing.
///
/// Best-effort — a device is served whether or not the fact records it,
/// and failing the sweep over a bookkeeping write would be worse than the
/// missing row.
pub(crate) async fn record_activation(state: &crate::worker::TonkState) {
    use tonk_schema::{AccountActive, prelude::DidExt as _};

    let Ok(account) = super::identity::root_did(state).await else {
        return;
    };
    // Registered and not yet active is the only state with a transition
    // to observe.
    let facts = account_registration(state).await;
    if facts.activated || facts.email.is_none() {
        return;
    }
    let at = Timestamp::now().to_unix();
    if let Err(error) = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountActive::new(account.this(), at))
        .commit()
        .perform(&state.operator)
        .await
    {
        log!("account activation not recorded: {error}");
    }
    // Activation is what the deferred work was waiting on: the custody
    // publish the ceremony pre-signed, and any space provisioned while
    // the gate still refused. Nothing polls a status endpoint any more,
    // so the sweep that noticed is the one that replays it.
    drain_pending(state).await;
}

/// Read how far this account got through registering.
///
/// Three independent facts answer it directly: suspension wins, then
/// activation, then registration alone. No status string to interpret.
/// The provider rides the REGISTRATION fact — it is known at enrollment —
/// so its presence says where the account will sync, not that anything
/// serves it yet: only the activation fact says that. Reading the
/// provider alone as served reported every freshly-enrolled account as
/// active, which wired remotes to a gate that refuses them and made the
/// awaiting state unreachable.
pub(crate) async fn registration(state: &crate::worker::TonkState) -> Registration {
    let facts = account_registration(state).await;
    if facts.suspended {
        return Registration::Suspended;
    }
    if facts.activated
        && let Some(provider) = facts.provider
    {
        return Registration::Served { provider };
    }
    match facts.email {
        Some(email) if !email.is_empty() => Registration::AwaitingActivation { email },
        _ => Registration::Unregistered,
    }
}

/// What the account's registration facts say, read in one pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AccountRegistrationFacts {
    /// The address enrollment named, when the account registered.
    pub email: Option<String>,
    /// Where the account syncs, named at enrollment.
    pub provider: Option<String>,
    /// Whether activation has been observed.
    pub activated: bool,
    /// Whether the service withdrew.
    pub suspended: bool,
}

/// This account's registration facts, all absent when nothing recorded any.
pub(crate) async fn account_registration(
    state: &crate::worker::TonkState,
) -> AccountRegistrationFacts {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{AccountActive, AccountRegistered, AccountSuspended, prelude::DidExt as _};

    let mut facts = AccountRegistrationFacts::default();
    let Ok(account) = super::identity::root_did(state).await else {
        return facts;
    };
    let Ok(branch) = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&state.operator)
        .await
    else {
        return facts;
    };
    let this = account.this();

    if let Ok(rows) = branch
        .handle()
        .query()
        .select(Query::<AccountRegistered> {
            this: Term::from(this.clone()),
            registered_at: Term::var("registered_at"),
            email: Term::var("email"),
            provider: Term::var("provider"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
    {
        let rows: Vec<AccountRegistered> = rows;
        if let Some(row) = rows.into_iter().next() {
            facts.email = Some(row.email.0);
            facts.provider = Some(row.provider.0).filter(|address| !address.is_empty());
        }
    }
    if let Ok(rows) = branch
        .handle()
        .query()
        .select(Query::<AccountActive> {
            this: Term::from(this.clone()),
            activated_at: Term::var("activated_at"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
    {
        let rows: Vec<AccountActive> = rows;
        facts.activated = !rows.is_empty();
    }
    if let Ok(rows) = branch
        .handle()
        .query()
        .select(Query::<AccountSuspended> {
            this: Term::from(this),
            suspended_at: Term::var("suspended_at"),
            reason: Term::var("reason"),
        })
        .perform(&state.operator)
        .try_vec()
        .await
    {
        let rows: Vec<AccountSuspended> = rows;
        facts.suspended = !rows.is_empty();
    }
    facts
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
    use tonk_schema::{AccountActive, AccountRegistered, AccountSuspended, prelude::DidExt as _};

    let account = super::identity::root_did(state).await?;
    // An absent address means "unchanged", not "no provider": a receipt
    // that names none — every enrollment receipt does — must not blank
    // one a later activation already recorded.
    let provider = match provider {
        Some(provider) => Some(provider.to_owned()),
        None => provider_address(state).await,
    };
    let at = Timestamp::now().to_unix();

    // Three independent facts, each written by the act that proves it and
    // never rewritten. Enrollment records the registration; activation adds
    // its own row rather than overwriting one, so two devices learning the
    // service's answer at once cannot race for a single status slot.
    let mut transaction = state
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountRegistered::new(
            account.this(),
            email.to_owned(),
            provider.unwrap_or_default(),
            at,
        ));
    // Activation is a timestamp and nothing else: where the account syncs
    // is on the registration, since that is where it was known.
    if status == CustomerStatus::Active {
        transaction = transaction.assert(AccountActive::new(account.this(), at));
    }
    if status == CustomerStatus::Suspended {
        transaction = transaction.assert(AccountSuspended::new(account.this(), String::new(), at));
    }
    transaction
        .commit()
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("commit account registration facts: {error}"))
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
    defer_all(state, vec![work]).await
}

async fn defer_all(
    state: &crate::worker::TonkState,
    work: Vec<PendingWork>,
) -> Result<(), TonkWorkerError> {
    let mut queue = load_pending(state).await?;
    queue.push_all(work);
    save_pending(state, &queue).await?;
    drain_pending(state).await;
    Ok(())
}

/// The next independently removable prefix. A custody provision immediately
/// followed by its publish stays one replay unit: if publishing fails after
/// provisioning succeeds, retaining both preserves the consent needed to try
/// the complete handoff again.
fn pending_replay_batch(work: &[PendingWork], start: usize) -> &[PendingWork] {
    let Some(first) = work.get(start) else {
        return &work[work.len()..];
    };
    let width = match (first, work.get(start + 1)) {
        (
            PendingWork::Provision { consumer, .. },
            Some(PendingWork::PublishCustody { custody, .. }),
        ) if consumer == custody => 2,
        _ => 1,
    };
    &work[start..start + width]
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
    'queue: while completed < queue.len() {
        let batch = pending_replay_batch(queue.entries(), completed);
        for work in batch {
            if let Err(error) = run_pending(state, work).await {
                log!("pending work for {} still waiting: {error}", work.subject());
                break 'queue;
            }
        }
        completed += batch.len();
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
            let consent = decode_custody_consent(consent_hex)
                .map_err(|error| TonkWorkerError::Router(format!("queued custody {error}")))?;
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
    use super::*;

    fn refusal(status: u16, code: Option<&str>) -> TonkWorkerError {
        TonkWorkerError::Upstream {
            status,
            code: code.map(str::to_owned),
            message: "refused".to_string(),
        }
    }

    /// The refusal that stranded accounts: enrollment is dispatched as a
    /// command and lands asynchronously, so a provisioning call that
    /// overtakes it meets a service with no customer row and is refused
    /// `UnknownCustomer`. Dropping the consent there is unrecoverable for
    /// a custody space, so it must queue exactly like the inactive case.
    #[test]
    fn it_queues_a_provisioning_that_raced_ahead_of_enrollment() {
        assert!(is_retryable(&refusal(404, Some("UnknownCustomer"))));
        assert!(is_retryable(&refusal(409, Some("CustomerInactive"))));
    }

    /// A phone on a flaky connection must not lose its consent either.
    /// Timeouts and transport failures arrive as `Upstream` with the
    /// codes `From<HttpError>` assigns them.
    #[test]
    fn it_queues_a_provisioning_the_network_refused() {
        assert!(is_retryable(&refusal(504, Some("UPSTREAM_TIMEOUT"))));
        assert!(is_retryable(&refusal(503, Some("UPSTREAM_UNAVAILABLE"))));
    }

    /// Refusals about the request itself do not become true by waiting,
    /// so they propagate rather than filling the queue with work that can
    /// never complete — and, ahead of a custody publish, block it.
    #[test]
    fn it_propagates_a_refusal_waiting_cannot_resolve() {
        assert!(!is_retryable(&refusal(403, Some("Forbidden"))));
        assert!(!is_retryable(&refusal(409, Some("ConsumerProvided"))));
        assert!(!is_retryable(&refusal(400, Some("Invalid"))));
    }

    /// The two retryable refusals both answer 4xx, so a status-only test
    /// would drop exactly the consents this fix exists to keep. Pinned
    /// because that is the mistake the first version of `is_retryable`
    /// made.
    #[test]
    fn it_does_not_judge_the_retryable_refusals_by_status_alone() {
        for (status, code) in [(404, "UnknownCustomer"), (409, "CustomerInactive")] {
            assert!(
                is_retryable(&refusal(status, Some(code))),
                "{code} answers {status} and must still queue"
            );
            assert!(
                !is_retryable(&refusal(status, None)),
                "the same status without the code is not retryable"
            );
        }
    }

    /// An unrecognized code is judged by its status: the service failing
    /// is worth retrying, the request being rejected is not.
    #[test]
    fn it_judges_an_unknown_refusal_by_its_status() {
        assert!(is_retryable(&refusal(500, Some("SomethingNew"))));
        assert!(is_retryable(&refusal(429, None)));
        assert!(!is_retryable(&refusal(400, None)));
        assert!(!is_retryable(&refusal(403, None)));
    }

    /// Only upstream refusals are provisioning outcomes at all; a local
    /// failure is this worker's own bug and must surface.
    #[test]
    fn it_does_not_queue_a_local_failure() {
        assert!(!is_retryable(&TonkWorkerError::Internal(
            "consent does not encode".to_string()
        )));
    }

    #[test]
    fn custody_queue_carries_an_ordered_repair_batch_and_accepts_legacy_requests() {
        let request = QueueCustodyRequest {
            custody: "did:key:zCustody".to_owned(),
            consent_hex: Some("aa".to_owned()),
            sealed_hex: "bb".to_owned(),
            invocation_hex: "c0de".to_owned(),
        };
        assert_eq!(
            custody_pending_batch(&request),
            vec![
                PendingWork::Provision {
                    consumer: "did:key:zCustody".to_owned(),
                    consent_hex: "aa".to_owned(),
                    consumer_kind: Some("custody".to_owned()),
                },
                PendingWork::PublishCustody {
                    custody: "did:key:zCustody".to_owned(),
                    sealed_hex: "bb".to_owned(),
                    invocation_hex: "c0de".to_owned(),
                },
            ]
        );

        let legacy: QueueCustodyRequest = serde_json::from_value(serde_json::json!({
            "custody": "did:key:zCustody",
            "sealedHex": "bb",
            "invocationHex": "c0de"
        }))
        .unwrap();
        assert_eq!(
            custody_pending_batch(&legacy),
            vec![PendingWork::PublishCustody {
                custody: "did:key:zCustody".to_owned(),
                sealed_hex: "bb".to_owned(),
                invocation_hex: "c0de".to_owned(),
            }]
        );
    }

    #[test]
    fn custody_replay_keeps_a_matching_provision_and_publish_in_one_batch() {
        let matching = QueueCustodyRequest {
            custody: "did:key:zCustody".to_owned(),
            consent_hex: Some("aa".to_owned()),
            sealed_hex: "bb".to_owned(),
            invocation_hex: "c0de".to_owned(),
        };
        let matching = custody_pending_batch(&matching);
        assert_eq!(pending_replay_batch(&matching, 0), matching.as_slice());

        let unrelated = vec![
            PendingWork::Provision {
                consumer: "did:key:zOne".to_owned(),
                consent_hex: "aa".to_owned(),
                consumer_kind: Some("custody".to_owned()),
            },
            PendingWork::PublishCustody {
                custody: "did:key:zTwo".to_owned(),
                sealed_hex: "bb".to_owned(),
                invocation_hex: "c0de".to_owned(),
            },
        ];
        assert_eq!(pending_replay_batch(&unrelated, 0), &unrelated[..1]);
    }
}
