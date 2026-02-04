use leptos::{logging::log, prelude::window};
use tonk_worker::{
    AuthorizeResponse, DelegationsResponse, IdentifyResponse, ListSpacesResponse, StatusResponse,
};

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

/// Build a space-aware API URL.
///
/// The multikey is the `z6Mk...` portion of a DID.
fn space_api_url(multikey: &str, path: &str) -> String {
    format!("{}/api/{}{}", origin(), multikey, path)
}

/// Lists all spaces the user has access to.
///
/// This is a global endpoint that doesn't require a space context.
pub async fn list_spaces() -> Result<ListSpacesResponse, TonkUiError> {
    log!("Fetching space list...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/space/list", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Fetches the current status of the space from the service worker.
pub async fn status(multikey: &str) -> Result<StatusResponse, TonkUiError> {
    log!("Fetching status for space {}...", multikey);

    let response = reqwest::Client::new()
        .get(space_api_url(multikey, "/status"))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Enables sync by authorizing the space with the access service.
///
/// This no longer requires any input - the service worker uses its
/// internal operator and delegation.
pub async fn authorize(multikey: &str) -> Result<AuthorizeResponse, TonkUiError> {
    log!("Enabling sync for space {}...", multikey);

    let response = reqwest::Client::new()
        .post(space_api_url(multikey, "/authorize"))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Fetches the current user's identity (DID) from the service worker.
///
/// This is a global endpoint that doesn't require a space context.
pub async fn identify() -> Result<IdentifyResponse, TonkUiError> {
    log!("Fetching identity...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/identify", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Fetches the user's delegations for the given space from the service worker.
pub async fn delegations(multikey: &str) -> Result<DelegationsResponse, TonkUiError> {
    log!("Fetching delegations for space {}...", multikey);

    let response = reqwest::Client::new()
        .get(space_api_url(multikey, "/delegations"))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}
