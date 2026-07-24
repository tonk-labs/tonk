use reqwest::StatusCode;
use serde::Deserialize;
use tonk_worker_api::{
    AccountDevice, AccountLinkRequest, AccountRecoverRequest, AccountStatus, EvaluateResponse,
    IdentifyResponse, JoinRequest, JoinResponse, QueryResponse, RepositoryInfo, RevokeDeviceRequest,
    SyncResponse, SyncStatusResponse,
};

use crate::error::TonkUiError;

/// Pending native device metadata resolved through a handoff secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAccountLink {
    /// Hash the root ceremony binds into its signed completion.
    pub token_hash: String,
    /// Native profile DID that will receive the delegation.
    pub device_did: String,
    /// Native device name shown for confirmation.
    pub device_name: String,
}

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

/// Return the current profile's persisted account-link state.
pub async fn account_status() -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/account", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    response.json().await.map_err(into_api_error)
}

/// Persist a verified account-root delegation in the local profile.
///
/// `succession_hex` is `Some` only for a rotation relink: the hex-encoded
/// `oldRoot → newRoot` succession chain that authorizes the worker to
/// replace an already-linked root with `root_did` instead of rejecting the
/// request as a conflicting root.
pub async fn save_account_link(
    root_did: String,
    delegation_hex: String,
    succession_hex: Option<&str>,
) -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/link", origin()))
        .json(&AccountLinkRequest {
            root_did,
            delegation_hex,
            succession_hex: succession_hex.map(str::to_owned),
        })
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/account/link returned {status}: {text}"
        )))
    }
}

/// List the devices registered under the linked account.
pub async fn account_devices() -> Result<Vec<AccountDevice>, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/account/devices", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/account/devices returned {status}: {text}"
        )))
    }
}

/// Revoke one of the account's devices; returns the refreshed list.
pub async fn revoke_account_device(did: String) -> Result<Vec<AccountDevice>, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/devices/revoke", origin()))
        .json(&RevokeDeviceRequest { did })
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/account/devices/revoke returned {status}: {text}"
        )))
    }
}

/// Clear this browser's stored account link (local sign-out).
pub async fn unlink_account() -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .delete(format!("{}/api/account", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "DELETE /api/account returned {status}: {text}"
        )))
    }
}

/// Submit a signed rotation ceremony to the account service.
///
/// `POST {service_url}/accounts/rotate` with the old-root-signed rotation
/// container and the new-root-signed confirmation container. On success the
/// service has flipped the account onto the new root; the caller still must
/// relink this profile locally (`save_account_link` with the succession
/// chain) to pick up the new custody key.
pub async fn rotate_account(
    service_url: &str,
    rotation_hex: &str,
    confirmation_hex: &str,
) -> Result<(), TonkUiError> {
    let response = reqwest::Client::new()
        .post(format!(
            "{}/accounts/rotate",
            service_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({
            "rotation": rotation_hex,
            "confirmation": confirmation_hex,
        }))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /accounts/rotate returned {status}: {text}"
        )))
    }
}

/// Recover an account onto a fresh root under a surviving device's
/// authority, via `POST /api/account/recover`. The worker builds the
/// device-signed half of the ceremony from the stored link, submits both
/// containers to the account service, replaces the local link, and
/// converges spaces — so a single successful response means recovery is
/// fully done, with nothing left for the caller to persist separately.
pub async fn recover_account(
    request: &AccountRecoverRequest,
) -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/recover", origin()))
        .json(request)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/account/recover returned {status}: {text}"
        )))
    }
}

/// Ask the account service to email a verification code.
pub async fn request_account_code(service: &str, email: &str) -> Result<(), TonkUiError> {
    let response = reqwest::Client::new()
        .post(format!("{}/codes", service.trim_end_matches('/')))
        .json(&serde_json::json!({ "email": email }))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /codes returned {status}: {text}"
        )))
    }
}

/// Submit a signed ceremony container to the account service.
pub async fn submit_account_ceremony(
    service: &str,
    path: &str,
    invocation_hex: &str,
) -> Result<(), TonkUiError> {
    let body = hex::decode(invocation_hex)
        .map_err(|error| TonkUiError::ApiError(format!("invalid invocation bytes: {error}")))?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/{}",
            service.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/cbor")
        .body(body)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST {path} returned {status}: {text}"
        )))
    }
}

/// Resolve a pending CLI handoff using its raw fragment secret.
pub async fn resolve_account_link(
    service: &str,
    secret: &str,
) -> Result<PendingAccountLink, TonkUiError> {
    let response = reqwest::Client::new()
        .post(format!("{}/links/resolve", service.trim_end_matches('/')))
        .json(&serde_json::json!({ "secret": secret }))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /links/resolve returned {status}: {text}"
        )))
    }
}
