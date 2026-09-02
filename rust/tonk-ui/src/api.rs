use reqwest::{Method, StatusCode};
use serde::Deserialize;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDevice,
    AccountLinkRequest, AccountStatus, AccountSummary, ActivateProfileRequest, EvaluateResponse,
    HostedSpaceDeletionResult, IdentifyResponse, JoinRequest, JoinResponse, ProfilesResponse,
    QueryResponse, RepositoryInfo, RevokeDeviceAcknowledgement, RevokeDeviceRequest, RootStatus,
    SaveRootRequest, SyncResponse, SyncStatusResponse,
};

use crate::error::AccountTransportKind;
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

fn into_api_error<T>(error: T) -> TonkUiError
where
    T: std::fmt::Display,
{
    TonkUiError::ApiError(format!("{error}"))
}

fn account_boundary_error(
    transport_kind: AccountTransportKind,
    status: Option<u16>,
    service_code: Option<String>,
    diagnostic: impl Into<String>,
) -> TonkUiError {
    TonkUiError::AccountApi {
        transport_kind,
        status,
        service_code,
        diagnostic: diagnostic.into(),
    }
}

async fn send_account(
    request: crate::worker_client::WorkerRequest,
    method: &'static str,
    path: &'static str,
) -> Result<reqwest::Response, TonkUiError> {
    request.send().await.map_err(|error| {
        account_boundary_error(
            AccountTransportKind::Network,
            None,
            None,
            format!("{method} {path} did not receive a response: {error}"),
        )
    })
}

async fn decode_account<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    method: &'static str,
    path: &'static str,
) -> Result<T, TonkUiError> {
    let status = response.status();
    let text = response.text().await.map_err(|error| {
        account_boundary_error(
            AccountTransportKind::Decode,
            Some(status.as_u16()),
            None,
            format!("{method} {path} response body was unreadable: {error}"),
        )
    })?;
    if status.is_success() {
        return serde_json::from_str(&text).map_err(|error| {
            account_boundary_error(
                AccountTransportKind::Decode,
                Some(status.as_u16()),
                None,
                format!("{method} {path} response did not decode: {error}"),
            )
        });
    }
    let service_code = serde_json::from_str::<ErrorBody>(&text)
        .ok()
        .and_then(|body| body.error.code.or(Some(body.error.kind)))
        .map(|code| {
            serde_json::to_value(tonk_analytics::account::ServiceCode::from_wire(&code))
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned())
        });
    Err(account_boundary_error(
        AccountTransportKind::Http,
        Some(status.as_u16()),
        service_code,
        format!("{method} {path} returned {status}: {text}"),
    ))
}

async fn finish_account(
    response: reqwest::Response,
    method: &'static str,
    path: &'static str,
) -> Result<(), TonkUiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.map_err(|error| {
        account_boundary_error(
            AccountTransportKind::Decode,
            Some(status.as_u16()),
            None,
            format!("{method} {path} response body was unreadable: {error}"),
        )
    })?;
    let service_code = serde_json::from_str::<ErrorBody>(&text)
        .ok()
        .and_then(|body| body.error.code.or(Some(body.error.kind)))
        .map(|code| {
            serde_json::to_value(tonk_analytics::account::ServiceCode::from_wire(&code))
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned())
        });
    Err(account_boundary_error(
        AccountTransportKind::Http,
        Some(status.as_u16()),
        service_code,
        format!("{method} {path} returned {status}: {text}"),
    ))
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
    tonk_common::log!("Fetching repository '{}'...", name);

    let response = crate::worker_client::request(Method::GET, format!("/api/repository/{name}"))
        .await?
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
    tonk_common::log!("Fetching profile repository...");
    let response = crate::worker_client::request(Method::GET, "/api/profile/repository")
        .await?
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
    let base = format!("/api/repository/{repo}/branch/{branch}/claim/select");
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(value) = the {
        query.append_pair("the", value);
    }
    if let Some(value) = of {
        query.append_pair("of", value);
    }
    let query = query.finish();
    let path = if query.is_empty() {
        base
    } else {
        format!("{base}?{query}")
    };

    let response = crate::worker_client::request(Method::GET, path)
        .await?
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
/// Assert a claim on the profile's main branch.
///
/// The page's way of causing an effect: a transient lands, its command
/// runs, and the outcome comes back as facts the page is subscribed to.
/// Nothing is read from the answer beyond whether the commit landed.
pub async fn transact_profile(claim: serde_json::Value) -> Result<(), TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/profile/branch/main/transact")
        .await?
        .json(&claim)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/profile/branch/main/transact returned {status}: {text}"
        )))
    }
}

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
    // The worker's default is `transact=true`; only attach the
    // query string when we want to override.
    let endpoint = if transact {
        path.to_owned()
    } else {
        format!("{path}?transact=false")
    };
    let response = crate::worker_client::request(Method::POST, &endpoint)
        .await?
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
    let path = format!("/api/repository/{repo}/branch/{branch}/sync/status");
    let response = crate::worker_client::request(Method::GET, &path)
        .await?
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
    let response = crate::worker_client::request(Method::POST, &path)
        .await?
        .send()
        .await
        .map_err(into_api_error)?;

    if !response.status().is_success() {
        let status = response.status();
        return match response.json::<ErrorBody>().await {
            Ok(body) if body.error.code.is_some() => Err(TonkUiError::Sync {
                code: body.error.code.expect("checked above"),
                message: body.error.message,
            }),
            _ => Err(TonkUiError::ApiError(format!(
                "POST {path} returned {status}"
            ))),
        };
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
    tonk_common::log!("Joining invite...");

    let body = JoinRequest {
        url: url.to_string(),
    };

    let response = crate::worker_client::request(Method::POST, "/api/profile/join")
        .await?
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
    tonk_common::log!("Fetching identity...");

    let response = crate::worker_client::request(Method::GET, "/api/identify")
        .await?
        .send()
        .await
        .map_err(into_api_error)?;

    response.json().await.map_err(into_api_error)
}

/// Return the current profile's provider-neutral local root state.
pub async fn root_status() -> Result<RootStatus, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/identity/root")
        .await?
        .send()
        .await
        .map_err(into_api_error)?;
    response.json().await.map_err(into_api_error)
}

/// Persist a verified local root ceremony result.
pub async fn save_root(
    credential_id: String,
    delegation_hex: String,
    passkey: Option<tonk_worker_api::PasskeyMetadata>,
    encryption_key: Option<String>,
) -> Result<RootStatus, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/identity/root")
        .await?
        .json(&SaveRootRequest {
            credential_id,
            delegation_hex,
            passkey,
            encryption_key,
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
            "POST /api/identity/root returned {status}: {text}"
        )))
    }
}

/// Enroll this profile's account as a customer of the access service,
/// sending the activation link to `email`, or to the account's recorded
/// address when none is given. Idempotent: re-enrolling while registered
/// resends the link, and an already-active customer answers as active.
///
/// A command, not a request, so it answers nothing. What enrollment
/// produces is the `AccountCustomer` fact, which the registration row
/// subscribes to; a caller that wants to know how it went watches that
/// rather than this call's return. `Ok` here means the transient was
/// committed and the handler will run, not that the service replied.
///
/// Dispatched as a `tonk-claim` on the document, which is where
/// `tonk_host::install` puts its listener. The account page deliberately
/// has no `window.tonk` — the top page must not look like a portal guest
/// — so the FAB's `window.tonk.transact` path is unavailable here.
///
/// Routed at profile main explicitly. A routeless claim resolves against
/// the nearest `with` ancestor, and this page has none, so it would land
/// on the bare endpoint rather than the branch the command's handler and
/// its `AccountCustomer` outcome both live on. The FAB gets away with a
/// routeless dispatch because a portal pins its context; nothing pins
/// one here.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn enroll_customer(email: Option<&str>) -> Result<(), TonkUiError> {
    use wasm_bindgen::JsValue;

    tonk_host::ready::wait().await;
    let consumer = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .ok_or_else(|| TonkUiError::ApiError("no document to dispatch from".to_string()))?;
    let request = js_sys::JSON::parse(&enroll_claim(email).to_string())
        .map_err(|error| TonkUiError::ApiError(format!("enroll claim did not parse: {error:?}")))?;
    tonk_host::consumer::claim_with_route(
        &consumer,
        &request,
        None,
        Some(tonk_account::MAIN_BRANCH),
        true,
    )
    .await
    .map(|_: JsValue| ())
    .map_err(|error| TonkUiError::ApiError(format!("enrollment was not dispatched: {error:?}")))
}

/// The custody material an enrollment carries, as the ceremony hands it
/// back.
///
/// Four values the worker cannot produce for itself: two of them are
/// signatures by the custody key, which exists only inside a live
/// passkey assertion in this page. They ride the command so the worker
/// can present them to the access service, which writes the cell.
/// The `TransactRequest` body for the `tonk:enroll` command.
///
/// Both fields are always present because a concept resolves only when
/// every one of them is, so "unset" is the empty string: no address means
/// the account's recorded one.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn enroll_claim(email: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Register this account as a customer of the access service.",
                        "with": {
                            "email": { "the": "xyz.tonk.enroll/email", "cardinality": "one", "as": "Text" }
                        }
                    }
                },
                "parameters": {
                    "email": email.unwrap_or_default()
                }
            }
        }]
    })
}

/// The `account/resend-activation` claim: ask the worker to have the
/// service mail the activation link again. No address and no ceremony —
/// the enrollment's rows stand, so the worker signs the resend
/// invocation with its own device key and nothing prompts for a passkey.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn resend_activation_claim(at: f64) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Send this account's activation link again.",
                        "with": {
                            "at": { "the": "xyz.tonk.resend-activation/at", "cardinality": "one", "as": "UnsignedInteger" }
                        }
                    }
                },
                "parameters": {
                    "at": at as u64
                }
            }
        }]
    })
}

/// Dispatch the resend-activation command.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn resend_activation() -> Result<(), TonkUiError> {
    use wasm_bindgen::JsValue;

    tonk_host::ready::wait().await;
    let consumer = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .ok_or_else(|| TonkUiError::ApiError("no document to dispatch from".to_string()))?;
    let request = js_sys::JSON::parse(&resend_activation_claim(js_sys::Date::now()).to_string())
        .map_err(|error| TonkUiError::ApiError(format!("resend claim did not parse: {error:?}")))?;
    tonk_host::consumer::claim_with_route(
        &consumer,
        &request,
        None,
        Some(tonk_account::MAIN_BRANCH),
        true,
    )
    .await
    .map(|_: JsValue| ())
    .map_err(|error| TonkUiError::ApiError(format!("resend was not dispatched: {error:?}")))
}

/// Ask the worker for a sync drain now.
///
/// The registering ceremony's activation signal is the account sweep's
/// own pull turning from refused to served, so its freshness is the
/// drain cadence. While the ceremony waits it calls this on its own
/// clock instead of the background heartbeat's; the drain coalesces
/// concurrent requests, so an extra ask costs nothing.
pub async fn kick_sync() -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/sync", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(TonkUiError::ApiError(format!(
            "POST /api/sync returned {}",
            response.status()
        )))
    }
}

/// The account's customer registration state: the access service's live
/// answer joined with the locally recorded enrollment.
pub async fn customer_state() -> Result<serde_json::Value, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/customer").await?;
    decode_account(
        send_account(response, "GET", "/api/customer").await?,
        "GET",
        "/api/customer",
    )
    .await
}

/// Return the current profile's persisted account-link state.
pub async fn account_status() -> Result<AccountStatus, TonkUiError> {
    let request = crate::worker_client::request(Method::GET, "/api/account").await?;
    decode_account(
        send_account(request, "GET", "/api/account").await?,
        "GET",
        "/api/account",
    )
    .await
}

/// Persist a verified account-root delegation in the local profile.
pub async fn save_account_link(
    provider: String,
    root_did: String,
    credential_id: String,
    delegation_hex: String,
    remote: String,
    initialize_name: bool,
) -> Result<AccountStatus, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/attach")
        .await?
        .json(&AccountLinkRequest {
            provider,
            root_did,
            credential_id,
            delegation_hex,
            remote,
            initialize_name,
        });
    decode_account(
        send_account(response, "POST", "/api/account/attach").await?,
        "POST",
        "/api/account/attach",
    )
    .await
}

/// List the devices registered under the linked account.
pub async fn account_devices() -> Result<Vec<AccountDevice>, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/account/devices").await?;
    decode_account(
        send_account(response, "GET", "/api/account/devices").await?,
        "GET",
        "/api/account/devices",
    )
    .await
}

/// Load verified account and passkey facts for the linked account.
pub async fn account_summary() -> Result<AccountSummary, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/account/summary").await?;
    decode_account(
        send_account(response, "GET", "/api/account/summary").await?,
        "GET",
        "/api/account/summary",
    )
    .await
}

/// Commit the authoritative display name for the active account/profile.
pub async fn set_account_display_name(name: &str) -> Result<String, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/display-name")
        .await?
        .json(&tonk_worker_api::AccountDisplayNameRequest {
            name: name.to_owned(),
        });
    let response = send_account(response, "POST", "/api/account/display-name").await?;
    decode_account::<tonk_worker_api::AccountDisplayNameResponse>(
        response,
        "POST",
        "/api/account/display-name",
    )
    .await
    .map(|response| response.name)
}

/// Register a freshly authorized device in the account service's
/// registry, through this browser's own membership. Answers the service's
/// JSON, which carries the issued `attachmentId`.
/// Provision a custody space under this profile's account: the page
/// ran the enrollment ceremony, the worker deposits the consent
/// through `/provider/add`.
pub async fn provision_custody(custody: &str, consent_hex: &str) -> Result<(), TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/custody/provision")
        .await?
        .json(&serde_json::json!({
            "custody": custody,
            "consentHex": consent_hex,
        }));
    finish_account(
        send_account(response, "POST", "/api/custody/provision").await?,
        "POST",
        "/api/custody/provision",
    )
    .await
}

/// The work queued until the account confirms its email, so the page
/// can run the parts only it can sign.
pub async fn pending_work() -> Result<tonk_account::pending::PendingQueue, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/customer/pending").await?;
    decode_account(
        send_account(response, "GET", "/api/customer/pending").await?,
        "GET",
        "/api/customer/pending",
    )
    .await
}

/// Record the complete custody handoff. The worker queues provisioning before
/// any deferred publish in one durable write, then drains both in order.
pub async fn queue_custody_publish(
    custody: &str,
    consent_hex: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> Result<(), TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/custody/queue")
        .await?
        .json(&serde_json::json!({
            "custody": custody,
            "consentHex": consent_hex,
            "sealedHex": sealed_hex,
            "invocationHex": invocation_hex,
        }));
    finish_account(
        send_account(response, "POST", "/api/custody/queue").await?,
        "POST",
        "/api/custody/queue",
    )
    .await
}

/// Register a device with the account service through the worker, so a
/// grant recipient is already listed before its delegation is delivered.
pub async fn register_account_device(
    did: &str,
    name: &str,
    delegation_hex: &str,
) -> Result<serde_json::Value, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/devices/register")
        .await?
        .json(&serde_json::json!({
            "did": did,
            "name": name,
            "delegationHex": delegation_hex,
        }));
    decode_account(
        send_account(response, "POST", "/api/account/devices/register").await?,
        "POST",
        "/api/account/devices/register",
    )
    .await
}

/// Revoke one of the account's devices and return the canonical publication
/// acknowledgement. Refreshing the device list is a separate, best-effort
/// operation.
pub async fn revoke_account_device(
    did: String,
) -> Result<RevokeDeviceAcknowledgement, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/devices/revoke")
        .await?
        .json(&RevokeDeviceRequest { did });
    decode_account(
        send_account(response, "POST", "/api/account/devices/revoke").await?,
        "POST",
        "/api/account/devices/revoke",
    )
    .await
}

/// List every profile signed in (or local) on this browser.
pub async fn list_profiles() -> Result<ProfilesResponse, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/profiles").await?;
    decode_account(
        send_account(response, "GET", "/api/profiles").await?,
        "GET",
        "/api/profiles",
    )
    .await
}

/// Swap the worker onto another roster profile. The caller reloads the
/// page afterwards so every surface re-renders the new profile.
pub async fn activate_profile(profile: String) -> Result<ProfilesResponse, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/profiles/activate")
        .await?
        .json(&ActivateProfileRequest { profile });
    decode_account(
        send_account(response, "POST", "/api/profiles/activate").await?,
        "POST",
        "/api/profiles/activate",
    )
    .await
}

/// Rotate onto a fresh profile — the landing pad the account ceremony then
/// runs on. Call this only when Create or Log in is actually submitted;
/// opening the account choice must remain reversible navigation.
pub async fn add_account_profile() -> Result<ProfilesResponse, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/profiles/add").await?;
    decode_account(
        send_account(response, "POST", "/api/profiles/add").await?,
        "POST",
        "/api/profiles/add",
    )
    .await
}

/// Sign out on this device while preserving its local profile and spaces.
pub async fn unlink_account() -> Result<AccountStatus, TonkUiError> {
    let response = crate::worker_client::request(Method::DELETE, "/api/account").await?;
    decode_account(
        send_account(response, "DELETE", "/api/account").await?,
        "DELETE",
        "/api/account",
    )
    .await
}

/// Load the exact, service-authoritative destructive scope for review.
pub async fn account_deletion_plan() -> Result<AccountDeletionPlan, TonkUiError> {
    let response = crate::worker_client::request(Method::GET, "/api/account/deletion/plan").await?;
    decode_account(
        send_account(response, "GET", "/api/account/deletion/plan").await?,
        "GET",
        "/api/account/deletion/plan",
    )
    .await
}

/// Execute the root-signed destructive plan in service-safe order.
pub async fn delete_account(
    request: &AccountDeletionRequest,
) -> Result<AccountDeletionResult, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/delete")
        .await?
        .json(request);
    decode_account(
        send_account(response, "POST", "/api/account/delete").await?,
        "POST",
        "/api/account/delete",
    )
    .await
}

/// Permanently delete one owned hosted space without deleting the account.
pub async fn delete_owned_space(
    request: &tonk_worker_api::AccountSpaceDeletionRequest,
) -> Result<HostedSpaceDeletionResult, TonkUiError> {
    let response = crate::worker_client::request(Method::POST, "/api/account/spaces/delete")
        .await?
        .json(request);
    decode_account(
        send_account(response, "POST", "/api/account/spaces/delete").await?,
        "POST",
        "/api/account/spaces/delete",
    )
    .await
}

#[cfg(test)]
mod enrollment_claim {
    use super::enroll_claim;

    /// The field rides even when unset, because a concept resolves
    /// only when every field is present: a claim that omitted `email`
    /// would never decode, and the command would silently not run.
    #[dialog_common::test]
    fn it_sends_the_field_even_when_no_address_was_given() {
        let claim = enroll_claim(None);
        assert_eq!(claim["claims"][0]["application"]["parameters"]["email"], "");
    }

    /// Empty means "the account's recorded address", which is what the
    /// login and resend paths want, so it must be distinguishable from
    /// an address that was given.
    #[dialog_common::test]
    fn it_carries_an_address_when_one_was_given() {
        let claim = enroll_claim(Some("a@example.com"));
        assert_eq!(
            claim["claims"][0]["application"]["parameters"]["email"],
            "a@example.com"
        );
    }

    /// The command is transient: it exists to trigger a handler and is
    /// swept at the commit, never persisted.
    #[dialog_common::test]
    fn it_asserts_a_transient() {
        let claim = enroll_claim(None);
        assert_eq!(claim["claims"][0]["op"], "assert");
        assert_eq!(
            claim["claims"][0]["application"]["predicate"]["kind"],
            "transient"
        );
    }
}
