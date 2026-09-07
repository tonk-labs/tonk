use serde::Deserialize;
use tonk_worker_api::{
    AccountStatus, AccountSummary, IdentifyResponse, RootStatus, SaveRootRequest,
};

use crate::error::AccountTransportKind;
use crate::error::TonkUiError;

/// Mirrors the worker's error envelope so we can decode
/// structured rejections (analyzer code + range) instead of
/// stringifying the response body.
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    kind: String,
    #[serde(default)]
    code: Option<String>,
}

fn into_api_error<T>(error: T) -> TonkUiError
where
    T: std::fmt::Display,
{
    TonkUiError::ApiError(format!("{error}"))
}

fn account_boundary_error(
    transport_kind: AccountTransportKind,
    status: Option<u16>,
    service_code: Option<String>,
    diagnostic: impl Into<String>,
) -> TonkUiError {
    TonkUiError::AccountApi {
        transport_kind,
        status,
        service_code,
        diagnostic: diagnostic.into(),
    }
}

async fn send_account(
    request: reqwest::RequestBuilder,
    method: &'static str,
    path: &'static str,
) -> Result<reqwest::Response, TonkUiError> {
    request.send().await.map_err(|error| {
        account_boundary_error(
            AccountTransportKind::Network,
            None,
            None,
            format!("{method} {path} did not receive a response: {error}"),
        )
    })
}

async fn decode_account<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    method: &'static str,
    path: &'static str,
) -> Result<T, TonkUiError> {
    let status = response.status();
    let text = response.text().await.map_err(|error| {
        account_boundary_error(
            AccountTransportKind::Decode,
            Some(status.as_u16()),
            None,
            format!("{method} {path} response body was unreadable: {error}"),
        )
    })?;
    if status.is_success() {
        return serde_json::from_str(&text).map_err(|error| {
            account_boundary_error(
                AccountTransportKind::Decode,
                Some(status.as_u16()),
                None,
                format!("{method} {path} response did not decode: {error}"),
            )
        });
    }
    let service_code = serde_json::from_str::<ErrorBody>(&text)
        .ok()
        .and_then(|body| body.error.code.or(Some(body.error.kind)))
        .map(|code| {
            serde_json::to_value(tonk_analytics::account::ServiceCode::from_wire(&code))
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned())
        });
    Err(account_boundary_error(
        AccountTransportKind::Http,
        Some(status.as_u16()),
        service_code,
        format!("{method} {path} returned {status}: {text}"),
    ))
}

/// Returns the page origin (`http://host:port`). Used by API
/// helpers to build absolute URLs against the worker's routes.
pub fn origin() -> String {
    web_sys::window()
        .expect("Could not access window")
        .location()
        .origin()
        .expect("Could not read window location")
}

/// Profile-side counterpart to [`evaluate`] — POSTs to
/// Assert a claim on the profile's main branch.
///
/// The page's way of causing an effect: a transient lands, its command
/// runs, and the outcome comes back as facts the page is subscribed to.
/// Nothing is read from the answer beyond whether the commit landed.
pub async fn transact_profile(claim: serde_json::Value) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/profile/branch/main/transact", origin()))
        .json(&claim)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/profile/branch/main/transact returned {status}: {text}"
        )))
    }
}

/// Outcome of [`join`] — invite redemption.
///
/// Distinguishes "name already taken" from other failures so the
/// `/join` form can keep the user on the page with a rename
/// prompt rather than a generic error. Note that "you already have
/// this space" is *not* an error here: the worker treats that
/// branch as success ([`JoinResponse::Renewed`]).
#[derive(Debug)]
pub enum JoinError {
    /// The chosen name is taken by an unrelated space. Recipient
    /// should retry with a different name.
    NameTaken,
    /// Any other failure — network, malformed invite, 5xx, etc.
    Other(TonkUiError),
}

impl From<TonkUiError> for JoinError {
    fn from(error: TonkUiError) -> Self {
        Self::Other(error)
    }
}

/// Fetches the current user's identity (DID) from the service worker.
pub async fn identify() -> Result<IdentifyResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    tonk_common::log!("Fetching identity...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/identify", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Return the current profile's provider-neutral local root state.
pub async fn root_status() -> Result<RootStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/identity/root", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    response.json().await.map_err(into_api_error)
}

/// Persist a verified local root ceremony result.
pub async fn save_root(
    credential_id: String,
    delegation_hex: String,
    passkey: Option<tonk_worker_api::PasskeyMetadata>,
    encryption_key: Option<String>,
) -> Result<RootStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/identity/root", origin()))
        .json(&SaveRootRequest {
            credential_id,
            delegation_hex,
            passkey,
            encryption_key,
        })
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/identity/root returned {status}: {text}"
        )))
    }
}

/// Ask the worker for a sync drain now.
///
/// The registering ceremony's activation signal is the account sweep's
/// own pull turning from refused to served, so its freshness is the
/// drain cadence. While the ceremony waits it calls this on its own
/// clock instead of the background heartbeat's; the drain coalesces
/// concurrent requests, so an extra ask costs nothing.
pub async fn kick_sync() -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/sync", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(TonkUiError::ApiError(format!(
            "POST /api/sync returned {}",
            response.status()
        )))
    }
}

/// The account's customer registration state: the access service's live
/// answer joined with the locally recorded enrollment.
pub async fn customer_state() -> Result<serde_json::Value, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new().get(format!("{}/api/customer", origin()));
    decode_account(
        send_account(response, "GET", "/api/customer").await?,
        "GET",
        "/api/customer",
    )
    .await
}

/// Return the current profile's persisted account-link state.
pub async fn account_status() -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let request = reqwest::Client::new().get(format!("{}/api/account", origin()));
    decode_account(
        send_account(request, "GET", "/api/account").await?,
        "GET",
        "/api/account",
    )
    .await
}

/// Load verified account and passkey facts for the linked account.
pub async fn account_summary() -> Result<AccountSummary, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new().get(format!("{}/api/account/summary", origin()));
    decode_account(
        send_account(response, "GET", "/api/account/summary").await?,
        "GET",
        "/api/account/summary",
    )
    .await
}
