use leptos::{logging::log, prelude::window};
use tonk_worker::{AuthorizeRequest, AuthorizeResponse};

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
