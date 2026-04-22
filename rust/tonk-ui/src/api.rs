use dialog_remote_ucan_s3::UcanAddress;
use leptos::{logging::log, prelude::window};
use reqwest::StatusCode;
use tonk_worker::{
    BranchConfiguration, ClaimRequest, IdentifyResponse, ListRepositoriesResponse, QueryResponse,
    RemoteConfiguration, RepositoryConfiguration, RepositoryInfo, SyncResponse,
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
/// `PUT /api/repository/{name}` with `If-None-Match: *`.
///
/// Returns `Ok(())` whether the repo was just created (`201`) or
/// already existed (`412`) — both are fine from the UI's point of
/// view. Any other non-success status is turned into an error.
///
/// The body wires up an `origin` remote pointing at the UCAN access
/// service (resolved against the current window origin) and sets
/// the default branch to track `origin/{branch}`.
pub async fn init() -> Result<(), TonkUiError> {
    log!("Ensuring repository '{}' exists...", DEFAULT_REPO);

    let configuration = default_configuration();

    let response = reqwest::Client::new()
        .put(format!("{}/api/repository/{}", origin(), DEFAULT_REPO))
        .header("If-None-Match", "*")
        .json(&configuration)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::CREATED | StatusCode::PRECONDITION_FAILED => Ok(()),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "PUT /api/repository/{} returned {}: {}",
                DEFAULT_REPO, status, text
            )))
        }
    }
}

/// Build the default [`RepositoryConfiguration`] the UI uses when
/// creating a new self-owned repo: `origin` remote at `/ucan/`, and
/// `main` tracking `origin/main`. Shared by [`init`] and [`create`].
fn default_configuration() -> RepositoryConfiguration {
    let address = UcanAddress::new(format!("{}{}", origin(), ACCESS_SERVICE_PATH));
    RepositoryConfiguration::default()
        .remote("origin", RemoteConfiguration::new(address))
        .branch(
            DEFAULT_BRANCH,
            BranchConfiguration::default().upstream("origin", DEFAULT_BRANCH),
        )
}

/// Create a new self-owned repo at `name` via `PUT /api/repository/{name}`.
///
/// Uses the same default configuration as [`init`] — `origin` pointed
/// at the UCAN access service with `main` tracking `origin/main`.
/// Returns the created [`RepositoryInfo`] on success. A 409 / 412
/// (already exists) surfaces as an error — callers should pick a new
/// name and retry.
pub async fn create(name: &str) -> Result<RepositoryInfo, TonkUiError> {
    log!("Creating repository '{}'...", name);

    let response = reqwest::Client::new()
        .put(format!("{}/api/repository/{}", origin(), name))
        .json(&default_configuration())
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::CREATED => response.json().await.map_err(into_api_error),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "PUT /api/repository/{} returned {}: {}",
                name, status, text
            )))
        }
    }
}

/// Redeem an invite URL via `POST /api/claim`.
///
/// Pass the complete `window.location.href` — audience-open invites
/// carry the ephemeral seed in the URL fragment, and browsers never
/// send fragments on normal fetches, so the UI is responsible for
/// forwarding the full string.
pub async fn claim(url: &str) -> Result<RepositoryInfo, TonkUiError> {
    log!("Claiming invite...");

    let response = reqwest::Client::new()
        .post(format!("{}/api/claim", origin()))
        .json(&ClaimRequest {
            url: url.to_string(),
        })
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => response.json().await.map_err(into_api_error),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST /api/claim returned {}: {}",
                status, text
            )))
        }
    }
}

/// List every repo registered in the profile's home meta-index.
pub async fn list_repositories() -> Result<Vec<String>, TonkUiError> {
    let response = reqwest::Client::new()
        .get(format!("{}/api/repositories", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => {
            let body: ListRepositoriesResponse = response.json().await.map_err(into_api_error)?;
            Ok(body.repositories)
        }
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "GET /api/repositories returned {}: {}",
                status, text
            )))
        }
    }
}

/// Pull remote state into `{repo}/{branch}` via the worker's
/// sync route.
pub async fn pull(repo: &str, branch: &str) -> Result<SyncResponse, TonkUiError> {
    log!("Pulling {}/{}...", repo, branch);

    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/repository/{}/branch/{}/sync/pull",
            origin(),
            repo,
            branch
        ))
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => response.json().await.map_err(into_api_error),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST /api/repository/{}/branch/{}/sync/pull returned {}: {}",
                repo, branch, status, text
            )))
        }
    }
}

/// Query claims from `{repo}/{branch}` via the worker's select endpoint.
///
/// At least one of `the` (attribute, `namespace/name`) or `of` (entity)
/// must be non-empty — this mirrors the worker-side validation.
pub async fn select_claims(
    repo: &str,
    branch: &str,
    the: Option<&str>,
    of: Option<&str>,
) -> Result<QueryResponse, TonkUiError> {
    let base = format!(
        "{}/api/repository/{}/branch/{}/claim/select",
        origin(),
        repo,
        branch
    );
    let mut url = url::Url::parse(&base).map_err(into_api_error)?;
    {
        let mut q = url.query_pairs_mut();
        if let Some(v) = the {
            q.append_pair("the", v);
        }
        if let Some(v) = of {
            q.append_pair("of", v);
        }
    }

    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => response.json().await.map_err(into_api_error),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "GET /api/repository/{}/branch/{}/claim/select returned {}: {}",
                repo, branch, status, text
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
