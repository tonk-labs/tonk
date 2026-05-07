use dialog_remote_ucan_s3::UcanAddress;
use leptos::{logging::log, prelude::window};
use reqwest::StatusCode;
use serde::Deserialize;
use tonk_worker::{
    BranchConfiguration, CreateInviteRequest, CreateInviteResponse, EvaluateResponse,
    IdentifyResponse, JoinRequest, JoinResponse, ProfileInfo, QueryResponse, RemoteConfiguration,
    RepositoryConfiguration, RepositoryInfo, SyncResponse,
};

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
    message: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    range: Option<lsp_types::Range>,
}

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

/// Returns the page origin (`http://host:port`). Used by API
/// helpers to build absolute URLs against the worker's routes.
pub fn origin() -> String {
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
/// returns the hosting document's service-worker Client ID as
/// reported by the worker in the `X-Tonk-Client-Id` response
/// header.
///
/// Succeeds whether the repo was just created (`201`) or
/// already existed (`412`) — header access works on both, so
/// the client id is available without touching the response
/// body.
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

    match response.status() {
        StatusCode::CREATED | StatusCode::PRECONDITION_FAILED => response
            .headers()
            .get("x-tonk-client-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                TonkUiError::ApiError(
                    "PUT /api/repository response missing X-Tonk-Client-Id header".to_string(),
                )
            }),
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
/// Sends `PUT /api/repository/{name}` with `If-None-Match: *` and
/// a body that defines a single `main` branch with no upstream and
/// no remotes. On success the worker registers a replica for this
/// repository in the profile repo and broadcasts on `/api/profile`,
/// so anything subscribed to that channel (notably the shell's
/// `ProfileResource`) can refresh.
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

/// Query claims on a branch via `GET /api/repository/{repo}/branch/{branch}/claim/select`.
///
/// At least one of `the` (attribute, namespace/name form) or `of`
/// (entity ID) must be supplied; the worker rejects unconstrained
/// queries.
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

/// Submit an asserted-notation document to a branch via
/// `POST /api/repository/{repo}/branch/{branch}/evaluate`.
///
/// The body may contain any mix of queries and mutations. The
/// worker analyzes, runs the unified query, then plans + commits
/// every mutation against each match frame in a single
/// transaction. Returns matches plus a commit summary.
///
/// `transact` controls the worker's commit step. Pass `false`
/// to project query results without applying mutations — used
/// by the editor's auto-evaluate (on idle) so a half-typed
/// transaction doesn't actually land.
pub async fn evaluate(
    repo: &str,
    branch: &str,
    body: String,
    content_type: &str,
    transact: bool,
) -> Result<EvaluateResponse, TonkUiError> {
    // The worker's default is `transact=true`; only attach the
    // query string when we want to override.
    let url = if transact {
        format!(
            "{}/api/repository/{}/branch/{}/evaluate",
            origin(),
            repo,
            branch
        )
    } else {
        format!(
            "{}/api/repository/{}/branch/{}/evaluate?transact=false",
            origin(),
            repo,
            branch
        )
    };
    let response = reqwest::Client::new()
        .post(url)
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => response.json::<EvaluateResponse>().await.map_err(|e| {
            TonkUiError::ApiError(format!(
                "POST /api/repository/{}/branch/{}/evaluate: failed to decode response body: {e}",
                repo, branch
            ))
        }),
        StatusCode::BAD_REQUEST => {
            let text = response.text().await.unwrap_or_default();
            // The worker emits a structured error body for
            // analyzer rejections (`{"error":{"kind":"analyze",
            // "code":"E_…","message":"…","range":{…}}}`). Try
            // to decode it so the editor can route it as a
            // diagnostic with proper position.
            match serde_json::from_str::<ErrorBody>(&text) {
                Ok(ErrorBody {
                    error:
                        ErrorDetail {
                            kind,
                            code: Some(code),
                            message,
                            range,
                        },
                }) if kind == "analyze" => Err(TonkUiError::Analyze {
                    code,
                    message,
                    range,
                }),
                _ => Err(TonkUiError::ApiError(format!(
                    "POST /api/repository/{}/branch/{}/evaluate returned 400: {}",
                    repo, branch, text
                ))),
            }
        }
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST /api/repository/{}/branch/{}/evaluate returned {}: {}",
                repo, branch, status, text
            )))
        }
    }
}

/// Mint an invite URL for a locally-owned space.
///
/// `POST /api/repository/{repo}/invite` with a JSON body —
/// `base_url` defaults the recipient's `/join` to the inviter's
/// own origin (so dev/localhost links open against dev), and
/// `audience` (when present) constrains the invite to a specific
/// recipient DID. Returns a tagged [`CreateInviteResponse`] so
/// the caller can branch on `Open` vs `Scoped` without re-parsing
/// the URL.
pub async fn create_invite(
    repo: &str,
    audience: Option<&str>,
) -> Result<CreateInviteResponse, TonkUiError> {
    log!("Minting invite for '{}' (audience={:?})...", repo, audience);

    let base_url = url::Url::parse(&format!("{}/join", origin()))
        .map_err(|e| TonkUiError::ApiError(format!("invalid window.origin: {e}")))?;
    let body = CreateInviteRequest {
        base_url: Some(base_url),
        audience: audience
            .map(|s| s.parse())
            .transpose()
            .map_err(|e| TonkUiError::ApiError(format!("invalid audience DID: {e}")))?,
    };

    let response = reqwest::Client::new()
        .post(format!("{}/api/repository/{}/invite", origin(), repo))
        .json(&body)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        // Tag decode failures separately so schema drift between
        // worker and UI surfaces distinctly from network errors.
        StatusCode::OK => response.json::<CreateInviteResponse>().await.map_err(|e| {
            TonkUiError::ApiError(format!(
                "POST /api/repository/{}/invite: failed to decode response body: {e}",
                repo
            ))
        }),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST /api/repository/{}/invite returned {}: {}",
                repo, status, text
            )))
        }
    }
}

/// Pull changes from upstream into a local branch.
///
/// `POST /api/repository/{repo}/branch/{branch}/sync/pull`. The
/// response carries the local revision before and after the pull,
/// which the UI uses to render a diff chip.
pub async fn pull(repo: &str, branch: &str) -> Result<SyncResponse, TonkUiError> {
    sync_op(repo, branch, "pull").await
}

/// Push local changes on a branch to upstream.
///
/// `POST /api/repository/{repo}/branch/{branch}/sync/push`.
pub async fn push(repo: &str, branch: &str) -> Result<SyncResponse, TonkUiError> {
    sync_op(repo, branch, "push").await
}

async fn sync_op(repo: &str, branch: &str, op: &str) -> Result<SyncResponse, TonkUiError> {
    log!("Sync ({}) repo='{}' branch='{}'", op, repo, branch);
    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/repository/{}/branch/{}/sync/{}",
            origin(),
            repo,
            branch,
            op,
        ))
        .send()
        .await
        .map_err(into_api_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(TonkUiError::ApiError(format!(
            "POST /api/repository/{}/branch/{}/sync/{} returned {}: {}",
            repo, branch, op, status, text
        )));
    }
    response
        .json::<SyncResponse>()
        .await
        .map_err(into_api_error)
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

/// Redeems an invite URL via `POST /api/profile/join`.
///
/// `url` must be the full invite URL including any `#fragment` (the
/// fragment carries the ephemeral seed for audience-open invites and
/// browsers don't transmit it with `fetch`, so the caller must read
/// `window.location.href` and pass the whole string). `name` is the
/// local label the recipient picked, used only when a fresh replica
/// is created — if the recipient already has this space, the
/// existing name is returned and `name` is ignored.
///
/// On success the worker registers (or reuses) the replica in the
/// profile meta branch and broadcasts on `/api/profile`, so the
/// shared [`ProfileResource`] picks up any new tile without an
/// explicit refetch.
///
/// [`ProfileResource`]: crate::components::ProfileResource
pub async fn join(url: &str, name: &str) -> Result<JoinResponse, JoinError> {
    log!("Joining invite as '{}'...", name);

    let body = JoinRequest {
        url: url.to_string(),
        name: name.to_string(),
    };

    let response = reqwest::Client::new()
        .post(format!("{}/api/profile/join", origin()))
        .json(&body)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK | StatusCode::CREATED => response
            .json::<JoinResponse>()
            .await
            .map_err(into_api_error)
            .map_err(JoinError::Other),
        StatusCode::CONFLICT => Err(JoinError::NameTaken),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST /api/profile/join returned {}: {}",
                status, text
            ))
            .into())
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
