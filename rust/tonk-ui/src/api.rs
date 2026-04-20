use leptos::{logging::log, prelude::window};
use tonk_worker::{
    ClaimInviteRequest, ClaimInviteResponse, CreateRepositoryRequest, CreateRepositoryResponse,
    IdentifyResponse, ListRepositoriesResponse,
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

/// Creates a new self-owned repo via the service worker. The request's
/// [`CreateRepositoryRequest::remote`] controls whether a sync remote is
/// configured, and which one.
pub async fn create_repository(
    req: &CreateRepositoryRequest,
) -> Result<CreateRepositoryResponse, TonkUiError> {
    log!("Creating repo…");

    let response = reqwest::Client::new()
        .post(format!("{}/api/repository/create", origin()))
        .json(req)
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Redeems an invite URL via the service worker, which parses the URL,
/// redelegates (for open invites) or verifies the audience (for scoped
/// invites), and persists the resulting delegation chain to the profile.
///
/// The full URL including any `#fragment` must be passed — the fragment
/// carries the ephemeral seed for open invites.
pub async fn claim_invite(url: &str) -> Result<ClaimInviteResponse, TonkUiError> {
    log!("Claiming invite…");

    let response = reqwest::Client::new()
        .post(format!("{}/api/invite/claim", origin()))
        .json(&ClaimInviteRequest {
            url: url.to_string(),
        })
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Lists every repo the profile has access to. Drives the sidebar and
/// the first-run-modal gating.
pub async fn list_repositories() -> Result<ListRepositoriesResponse, TonkUiError> {
    let response = reqwest::Client::new()
        .get(format!("{}/api/repositories", origin()))
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
