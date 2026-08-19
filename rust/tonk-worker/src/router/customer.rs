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
use tonk_account::CUSTOMER_CREDENTIAL_SITE;
use tonk_account::customer::{CustomerStatus, Receipt};
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

/// Provision `consumer` with the same-origin access service under this
/// profile's account, depositing `consent` — the space's powerline to
/// the account. A consumer another customer already provides is not an
/// error here: the space exists and works locally either way, and the
/// caller treats this whole call as best effort.
pub(crate) async fn provision_consumer(
    state: &crate::worker::TonkState,
    consumer: &dialog_varsig::Did,
    consent: &dialog_ucan_core::DelegationChain,
    deletion_grant: Option<&dialog_ucan_core::DelegationChain>,
) -> Result<(), TonkWorkerError> {
    use tonk_identity::request::build_provider_add_invocation_with_deletion;

    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let device = state.profile.signer().signer().clone();
    let body = build_provider_add_invocation_with_deletion(
        device,
        &link,
        consumer,
        consent,
        deletion_grant,
    )
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
