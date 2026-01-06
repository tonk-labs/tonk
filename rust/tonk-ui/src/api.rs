use leptos::{logging::log, prelude::window};
use tonk_worker::{AuthorizeRequest, AuthorizeResponse, StatusResponse};

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

/// Authorizes the user with the Tonk service worker.
pub async fn authorize(body: AuthorizeRequest) -> Result<AuthorizeResponse, TonkUiError> {
    log!("Authorizing...");

    let response = reqwest::Client::new()
        .post(format!("{}/api/authorize", origin()))
        .json(&body)
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}
