use reqwest::StatusCode;
use serde::Deserialize;
use tonk_worker_api::{
    BranchConfiguration, EvaluateResponse, IdentifyResponse, JoinRequest, JoinResponse,
    ProfileInfo, QueryResponse, RemoteConfiguration, RepositoryConfiguration, RepositoryInfo,
    SyncResponse, SyncStatusResponse,
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
    web_sys::window()
        .expect("Could not access window")
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
    tonk_host::ready::wait().await;
    tonk_common::log!("Fetching repository '{}'...", name);

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

/// Fetch [`RepositoryInfo`] for the profile-as-repository via
/// `GET /api/profile/repository`. The profile lives outside the
/// named-repo namespace, so its `RepositoryInfo` has its own
/// endpoint instead of `/api/repository/{name}`.
pub async fn profile_repository() -> Result<Option<RepositoryInfo>, TonkUiError> {
    tonk_host::ready::wait().await;
    tonk_common::log!("Fetching profile repository...");
    let response = reqwest::Client::new()
        .get(format!("{}/api/profile/repository", origin()))
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
                "GET /api/profile/repository returned {}: {}",
                status, text
            )))
        }
    }
}

/// Ensures the profile has at least one space, then returns the
/// hosting document's service-worker Client ID (the `X-Tonk-Client-Id`
/// response header the worker stamps on every response).
///
/// Idempotency is keyed on profile *state*, not on a fixed repository
/// address: a repository's identity is a freshly minted `did:key`, so
/// there is no stable name to `If-None-Match` against. We `GET
/// /api/profile` (which also carries the client-id header) and only
/// create a space when the profile lists zero. The new space is created
/// without a fixed identifier — the worker mints the routing key — and
/// labeled [`DEFAULT_REPO`], a display name only.
///
/// The create body wires up an `origin` remote pointing at the UCAN
/// access service (resolved against the current window origin) and sets
/// the default branch to track `origin/{branch}`.
pub async fn init() -> Result<String, TonkUiError> {
    tonk_host::ready::wait().await;
    tonk_common::log!("Ensuring the profile has at least one space...");

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

    // The client id rides on every worker response, including this GET.
    let client_id = response
        .headers()
        .get("x-tonk-client-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            TonkUiError::ApiError(
                "GET /api/profile response missing X-Tonk-Client-Id header".to_string(),
            )
        })?;

    let profile = response
        .json::<ProfileInfo>()
        .await
        .map_err(into_api_error)?;

    // Profile already has a space → nothing to create. The user lands on
    // the Hub and picks one.
    if !profile.space.is_empty() {
        tonk_common::log!(
            "Profile already has {} space(s); skipping default create",
            profile.space.len()
        );
        return Ok(client_id);
    }

    tonk_common::log!(
        "Profile has no spaces; creating the default '{}'",
        DEFAULT_REPO
    );

    let service_url = format!("{}{}", origin(), ACCESS_SERVICE_PATH);
    // The origin remote is a UCAN-over-S3 access endpoint; the worker
    // decodes this into its real `SiteAddress`.
    let configuration = RepositoryConfiguration::default()
        .remote("origin", RemoteConfiguration::ucan(service_url))
        // The branch's standard library (the built-in concepts,
        // views, commands, rules + demo content) is seeded by the
        // service worker at repository creation: it fetches the
        // served `/library/core.yaml` asset and runs it through the
        // evaluate pipeline. The shell no longer ships it in the PUT
        // body, so the library updates without rebuilding the wasm.
        .branch(
            DEFAULT_BRANCH,
            BranchConfiguration::default().upstream("origin", DEFAULT_BRANCH),
        );

    // The path segment is only the display label; the worker mints the
    // routing key and returns it in the `RepositoryInfo`.
    let response = reqwest::Client::new()
        .put(format!("{}/api/repository/{}", origin(), DEFAULT_REPO))
        .json(&configuration)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::CREATED => Ok(client_id),
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "PUT /api/repository/{} returned {}: {}",
                DEFAULT_REPO, status, text
            )))
        }
    }
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
    tonk_host::ready::wait().await;
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
    let path = format!("/api/repository/{repo}/branch/{branch}/evaluate");
    evaluate_at(&path, body, content_type, transact).await
}

/// Profile-side counterpart to [`evaluate`] — POSTs to
/// `/api/profile/branch/{branch}/evaluate`. Used by the profile
/// view's editor so its requests don't get mis-routed through
/// the named-repo namespace.
pub async fn evaluate_profile(
    branch: &str,
    body: String,
    content_type: &str,
    transact: bool,
) -> Result<EvaluateResponse, TonkUiError> {
    let path = format!("/api/profile/branch/{branch}/evaluate");
    evaluate_at(&path, body, content_type, transact).await
}

/// Shared body for [`evaluate`] / [`evaluate_profile`]. `path`
/// is the URL path (no query string, no origin); `transact=false`
/// appends `?transact=false`.
async fn evaluate_at(
    path: &str,
    body: String,
    content_type: &str,
    transact: bool,
) -> Result<EvaluateResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    // The worker's default is `transact=true`; only attach the
    // query string when we want to override.
    let url = if transact {
        format!("{}{}", origin(), path)
    } else {
        format!("{}{}?transact=false", origin(), path)
    };
    let response = reqwest::Client::new()
        .post(&url)
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .map_err(into_api_error)?;

    match response.status() {
        StatusCode::OK => response.json::<EvaluateResponse>().await.map_err(|e| {
            TonkUiError::ApiError(format!("POST {path}: failed to decode response body: {e}",))
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
                    "POST {path} returned 400: {text}",
                ))),
            }
        }
        status => {
            let text = response.text().await.unwrap_or_default();
            Err(TonkUiError::ApiError(format!(
                "POST {path} returned {status}: {text}",
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

/// Full sync (pull then push) of a branch against its upstream.
///
/// `POST /api/repository/{repo}/branch/{branch}/sync`. Used by the
/// background sync controller so a tick reconciles a branch in both
/// directions in one round-trip.
pub async fn sync(repo: &str, branch: &str) -> Result<SyncResponse, TonkUiError> {
    sync_op(repo, branch, "").await
}

/// Read a branch's sync state relative to its upstream.
///
/// `GET /api/repository/{repo}/branch/{branch}/sync/status`. Read
/// only — fetches the upstream head without merging — so the badge
/// can refresh without moving any data.
pub async fn sync_status(repo: &str, branch: &str) -> Result<SyncStatusResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    let path = format!("/api/repository/{repo}/branch/{branch}/sync/status");
    let response = reqwest::Client::new()
        .get(format!("{}{}", origin(), path))
        .send()
        .await
        .map_err(into_api_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(TonkUiError::ApiError(format!(
            "GET {path} returned {status}: {text}"
        )));
    }
    response
        .json::<SyncStatusResponse>()
        .await
        .map_err(into_api_error)
}

/// POST a sync route. `op` is `"pull"` / `"push"` for the
/// directional routes, or `""` for the combined `/sync` route.
async fn sync_op(repo: &str, branch: &str, op: &str) -> Result<SyncResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    tonk_common::log!("Sync ({}) repo='{}' branch='{}'", op, repo, branch);
    // `op` is a directional suffix (`/pull`, `/push`); the combined
    // sync route is the bare `/sync` path, so an empty `op` drops
    // the trailing segment entirely.
    let suffix = if op.is_empty() {
        String::new()
    } else {
        format!("/{op}")
    };
    let path = format!("/api/repository/{repo}/branch/{branch}/sync{suffix}");
    let response = reqwest::Client::new()
        .post(format!("{}{}", origin(), path))
        .send()
        .await
        .map_err(into_api_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(TonkUiError::ApiError(format!(
            "POST {path} returned {status}: {text}"
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
/// profile meta branch and broadcasts on `/api/profile`, so any
/// subscriber picks up the new tile without an explicit refetch.
pub async fn join(url: &str) -> Result<JoinResponse, JoinError> {
    tonk_host::ready::wait().await;
    tonk_common::log!("Joining invite...");

    let body = JoinRequest {
        url: url.to_string(),
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
    tonk_host::ready::wait().await;
    tonk_common::log!("Fetching identity...");

    let response = reqwest::Client::new()
        .get(format!("{}/api/identify", origin()))
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}
