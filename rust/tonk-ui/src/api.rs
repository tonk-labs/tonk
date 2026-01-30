use leptos::{logging::log, prelude::window};
use tonk_worker::{AuthorizeResponse, DelegationsResponse, IdentifyResponse, StatusResponse};

use crate::error::TonkUiError;

fn into_api_error<T>(error: T) -> TonkUiError
where
    T: std::fmt::Display,
{
    TonkUiError::ApiError(format!("{error}"))
}

fn origin() -> String {
    window()
        .location()
        .origin()
        .expect("Could not read window location")
}

/// Fetches the current status of the space from the service worker.
pub async fn status() -> Result<StatusResponse, TonkUiError> {
    log!("Fetching status...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/status", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Enables sync by authorizing the space with the access service.
///
/// This no longer requires any input - the service worker uses its
/// internal operator and delegation.
pub async fn authorize() -> Result<AuthorizeResponse, TonkUiError> {
    log!("Enabling sync...");

    let response = reqwest::Client::new()
        .post(format!("{}/api/authorize", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Fetches the current user's identity (DID) from the service worker.
pub async fn identify() -> Result<IdentifyResponse, TonkUiError> {
    log!("Fetching identity...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/identify", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Fetches the user's delegations for the current space from the service worker.
pub async fn delegations() -> Result<DelegationsResponse, TonkUiError> {
    log!("Fetching delegations...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/delegations", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}
