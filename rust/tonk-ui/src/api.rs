use dialog_remote_ucan_s3::UcanAddress;
use leptos::{logging::log, prelude::window};
use reqwest::StatusCode;
use tonk_worker::{
    BranchConfiguration, IdentifyResponse, RemoteConfiguration, RepositoryConfiguration,
    RepositoryInfo,
};

use crate::error::TonkUiError;

/// Default repository name used by the UI.
pub const DEFAULT_REPO: &str = "home";
/// Default branch name.
const DEFAULT_BRANCH: &str = "main";
/// Path of the UCAN access service, resolved against the window origin.
const ACCESS_SERVICE_PATH: &str = "/ucan/";

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

/// Fetches the repository record at `GET /api/repository/{name}`.
///
/// `Ok(Some(...))` on 200, `Ok(None)` on 404, `Err(...)` for any
/// other failure. Modelling 404 as an absence rather than an error
/// lets the UI use `ErrorBoundary` for genuine failures while
/// rendering a "not found" view through the normal value path.
pub async fn repository(name: &str) -> Result<Option<RepositoryInfo>, TonkUiError> {
    log!("Fetching repository '{}'...", name);

    let response = reqwest::Client::new()
        .get(format!("{}/api/repository/{}", origin(), name))
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => {
            let info = response
                .json::<RepositoryInfo>()
                .await
                .map_err(into_api_error)?;
            Ok(Some(info))
        }
        StatusCode::NOT_FOUND => Ok(None),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "GET /api/repository/{} returned {}: {}",
                name, status, text
            )))
        }
    }
}

/// Ensures the default repository exists via
/// `PUT /api/repository/{name}` with `If-None-Match: *`, and
/// returns the current document's service-worker Client ID as
/// reported by the worker in the `X-Tonk-Client-Id` response
/// header.
///
/// Succeeds whether the repo was just created (`201`) or already
/// existed (`412`) — both are fine from the UI's point of view.
/// Any other non-success status, or a missing client-id header,
/// is turned into an error.
///
/// The body wires up an `origin` remote pointing at the UCAN access
/// service (resolved against the current window origin) and sets
/// the default branch to track `origin/{branch}`.
pub async fn init() -> Result<String, TonkUiError> {
    log!("Ensuring repository '{}' exists...", DEFAULT_REPO);

    let service_url = format!("{}{}", origin(), ACCESS_SERVICE_PATH);
    // `RemoteConfiguration::new` accepts anything that converts
    // into `SiteAddress`, and `UcanAddress` does via `NetworkAddress`.
    let address = UcanAddress::new(&service_url);

    let configuration = RepositoryConfiguration::default()
        .remote("origin", RemoteConfiguration::new(address))
        .branch(
            DEFAULT_BRANCH,
            BranchConfiguration::default().upstream("origin", DEFAULT_BRANCH),
        );

    let response = reqwest::Client::new()
        .put(format!("{}/api/repository/{}", origin(), DEFAULT_REPO))
        .header("If-None-Match", "*")
        .json(&configuration)
        .send()
        .await
        .map_err(into_api_error)?;

    // The server returns a `RepositoryInfo` body on both `201
    // Created` (fresh repo) and `412 Precondition Failed` (repo
    // already existed). We don't use the body here — it's the
    // `X-Tonk-Client-Id` header we're after — but leaving a
    // JSON body on both status codes keeps `reqwest`'s wasm
    // client happy: it otherwise surfaces a spurious "error
    // decoding response body" when it finds an empty body on
    // the `412` response path.
    match response.status() {
        StatusCode::OK | StatusCode::CREATED | StatusCode::PRECONDITION_FAILED => {
            let host_id = response
                .headers()
                .get("x-tonk-client-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    TonkUiError::ApiError(
                        "PUT /api/repository response missing X-Tonk-Client-Id header"
                            .to_string(),
                    )
                })?;
            Ok(host_id)
        }
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "PUT /api/repository/{} returned {}: {}",
                DEFAULT_REPO, status, text
            )))
        }
    }
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
