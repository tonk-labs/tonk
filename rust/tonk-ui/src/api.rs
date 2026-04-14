use leptos::{logging::log, prelude::window};
use tonk_worker::{IdentifyResponse, InitResponse, StatusResponse};

use crate::error::TonkUiError;

/// Default repository name used by the UI.
const DEFAULT_REPO: &str = "home";
/// Default branch name.
const DEFAULT_BRANCH: &str = "main";

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

/// Fetches the current status of the repository from the service worker.
pub async fn status() -> Result<StatusResponse, TonkUiError> {
    log!("Fetching status...");

    let response = reqwest::Client::new()
        .get(format!(
            "{}/api/repository/{}/status",
            origin(),
            DEFAULT_REPO
        ))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Initializes sync by setting up the UCAN remote for the default branch.
pub async fn init() -> Result<InitResponse, TonkUiError> {
    log!("Initializing sync...");

    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/repository/{}/branch/{}/init",
            origin(),
            DEFAULT_REPO,
            DEFAULT_BRANCH
        ))
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
