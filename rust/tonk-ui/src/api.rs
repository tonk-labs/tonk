use dialog_remote_ucan_s3::UcanAddress;
use leptos::{logging::log, prelude::window};
use reqwest::StatusCode;
use tonk_worker::{
    BranchConfiguration, IdentifyResponse, ProfileInfo, RemoteConfiguration,
    RepositoryConfiguration, RepositoryInfo,
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

/// Outcome of [`create_space`].
///
/// Distinguishes "name already taken" from other failures so the
/// dialog can surface a field-specific message instead of a
/// generic error. 409/412 both mean "already exists" — the 412
/// case just signals that the caller used `If-None-Match: *`,
/// which we always do here.
#[derive(Debug)]
pub enum CreateSpaceError {
    /// A repository with this name is already registered.
    AlreadyExists,
    /// Any other failure — network, 5xx, serialization, etc.
    Other(TonkUiError),
}

impl From<TonkUiError> for CreateSpaceError {
    fn from(error: TonkUiError) -> Self {
        Self::Other(error)
    }
}

/// Creates a new repository with the given name.
///
/// Sends `PUT /api/repository/{name}` with `If-None-Match: *`
/// and a body that defines a single `main` branch with no
/// upstream and no remotes. On success the worker registers a
/// replica for this repository in the profile repo, which means
/// the next `GET /api/profile` will include it in the space
/// list — callers should refetch the shared `ProfileResource` so
/// the sidebar picks up the new tile.
pub async fn create_space(name: &str) -> Result<RepositoryInfo, CreateSpaceError> {
    log!("Creating space '{}'...", name);

    let configuration =
        RepositoryConfiguration::default().branch(DEFAULT_BRANCH, BranchConfiguration::default());

    let response = reqwest::Client::new()
        .put(format!("{}/api/repository/{}", origin(), name))
        .header("If-None-Match", "*")
        .json(&configuration)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::CREATED => response
            .json::<RepositoryInfo>()
            .await
            .map_err(into_api_error)
            .map_err(CreateSpaceError::Other),
        StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT => {
            Err(CreateSpaceError::AlreadyExists)
        }
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "PUT /api/repository/{} returned {}: {}",
                name, status, text
            ))
            .into())
        }
    }
}

/// Fetches the profile record at `GET /api/profile`.
///
/// Returns the profile's `RepositoryInfo` and a `{ name -> subject }`
/// map of every space this profile owns. The sidebar uses this to
/// render a tile per space without fetching each repository
/// individually.
pub async fn profile() -> Result<ProfileInfo, TonkUiError> {
    log!("Fetching profile...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/profile", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(TonkUiError::ApiError(format!(
            "GET /api/profile returned {}: {}",
            status, text
        )));
    }

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
