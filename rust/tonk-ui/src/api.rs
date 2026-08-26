use reqwest::StatusCode;
use serde::Deserialize;
use tonk_worker_api::{
    AccountDeletionPlan, AccountDeletionRequest, AccountDeletionResult, AccountDevice,
    AccountLinkRequest, AccountStatus, AccountSummary, ActivateProfileRequest, EvaluateResponse,
    HostedSpaceDeletionResult, IdentifyResponse, JoinRequest, JoinResponse, ProfilesResponse,
    QueryResponse, RepositoryInfo, RevokeDeviceAcknowledgement, RevokeDeviceRequest, RootStatus,
    SaveRootRequest, SyncResponse, SyncStatusResponse,
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

/// Ask the worker what it knows about an email address.
///
/// A command, not a request: the answer lands as the `EmailStatus`
/// overlay fact the entry screen subscribes to, which is also where the
/// share dialog's registration form reads it. One implementation of the
/// mapping, one place the states are named.
///
/// The attribute the command reads is normally filled from a form's
/// submit event; the entry screen asks while someone is still typing, so
/// it supplies the value directly.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn check_email(email: &str) -> Result<(), TonkUiError> {
    use wasm_bindgen::JsValue;

    tonk_host::ready::wait().await;
    let consumer = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .ok_or_else(|| TonkUiError::ApiError("no document to dispatch from".to_string()))?;
    let request = js_sys::JSON::parse(&check_email_claim(email).to_string())
        .map_err(|error| TonkUiError::ApiError(format!("check claim did not parse: {error:?}")))?;
    tonk_host::consumer::claim_with_route(
        &consumer,
        &request,
        None,
        Some(tonk_account::MAIN_BRANCH),
        true,
    )
    .await
    .map(|_: JsValue| ())
    .map_err(|error| TonkUiError::ApiError(format!("check was not dispatched: {error:?}")))
}

/// The `TransactRequest` body for `account/check-email`.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
fn check_email_claim(email: &str) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Ask what is registered under an email address.",
                        "with": {
                            "email": {
                                "the": "dom.event.current-target.elements.email/value",
                                "cardinality": "one",
                                "as": "Text"
                            }
                        }
                    }
                },
                "parameters": { "email": email }
            }
        }]
    })
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
/// Assert a claim on the profile's main branch.
///
/// The page's way of causing an effect: a transient lands, its command
/// runs, and the outcome comes back as facts the page is subscribed to.
/// Nothing is read from the answer beyond whether the commit landed.
pub async fn transact_profile(claim: serde_json::Value) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/profile/branch/main/transact", origin()))
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

/// Return the current profile's provider-neutral local root state.
pub async fn root_status() -> Result<RootStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/identity/root", origin()))
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
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/identity/root", origin()))
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
pub async fn enroll_customer(email: Option<&str>, deposits: &[String]) -> Result<(), TonkUiError> {
    use wasm_bindgen::JsValue;

    tonk_host::ready::wait().await;
    let consumer = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .ok_or_else(|| TonkUiError::ApiError("no document to dispatch from".to_string()))?;
    let request = js_sys::JSON::parse(&enroll_claim(email, deposits).to_string())
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

/// The `TransactRequest` body for the `tonk:enroll` command.
///
/// Natively only its tests reach it; the dispatcher above is wasm-only.
///
/// Both fields are always present because a concept resolves only when
/// every one of them is, so "unset" is the empty string: no address means
/// the account's recorded one, and no deposits mean the worker mints a
/// device-chained set. The deposits join with commas rather than riding
/// as a list, because a command's fields are scalars.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
fn enroll_claim(email: Option<&str>, deposits: &[String]) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Register this account as a customer of the access service.",
                        "with": {
                            "email": { "the": "xyz.tonk.enroll/email", "cardinality": "one", "as": "Text" },
                            "deposits": { "the": "xyz.tonk.enroll/deposits", "cardinality": "one", "as": "Text" }
                        }
                    }
                },
                "parameters": {
                    "email": email.unwrap_or_default(),
                    "deposits": deposits.join(",")
                }
            }
        }]
    })
}

/// The account's customer registration state: the access service's live
/// answer joined with the locally recorded enrollment.
pub async fn customer_state() -> Result<serde_json::Value, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/customer", origin()))
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
pub async fn save_account_link(
    provider: String,
    root_did: String,
    credential_id: String,
    delegation_hex: String,
    descriptor_hex: String,
    initialize_name: bool,
) -> Result<AccountStatus, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/attach", origin()))
        .json(&AccountLinkRequest {
            provider,
            root_did,
            credential_id,
            delegation_hex,
            descriptor_hex,
            initialize_name,
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
            "POST /api/account/attach returned {status}: {text}"
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

/// Load verified account and passkey facts for the linked account.
pub async fn account_summary() -> Result<AccountSummary, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/account/summary", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/account/summary returned {status}: {text}"
        )))
    }
}

/// Register a freshly authorized device in the account service's
/// registry, through this browser's own membership. Answers the service's
/// JSON, which carries the issued `attachmentId`.
/// Provision a custody space under this profile's account: the page
/// ran the enrollment ceremony, the worker deposits the consent
/// through `/provider/add`.
pub async fn provision_custody(custody: &str, consent_hex: &str) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/custody/provision", origin()))
        .json(&serde_json::json!({
            "custody": custody,
            "consentHex": consent_hex,
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
            "POST /api/custody/provision returned {status}: {text}"
        )))
    }
}

/// The work queued until the account confirms its email, so the page
/// can run the parts only it can sign.
pub async fn pending_work() -> Result<tonk_account::pending::PendingQueue, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/customer/pending", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/customer/pending returned {status}: {text}"
        )))
    }
}

/// Queue a custody cell that could not be published yet, so it is
/// republished once provisioning and email activation let it through.
pub async fn queue_custody_publish(
    custody: &str,
    sealed_hex: &str,
    invocation_hex: &str,
) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/custody/queue", origin()))
        .json(&serde_json::json!({
            "custody": custody,
            "sealedHex": sealed_hex,
            "invocationHex": invocation_hex,
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
            "POST /api/custody/queue returned {status}: {text}"
        )))
    }
}

/// Register a device with the account service through the worker, so a
/// grant recipient is already listed before its delegation is delivered.
pub async fn register_account_device(
    did: &str,
    name: &str,
    delegation_hex: &str,
) -> Result<serde_json::Value, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/devices/register", origin()))
        .json(&serde_json::json!({
            "did": did,
            "name": name,
            "delegationHex": delegation_hex,
        }))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/account/devices/register returned {status}: {text}"
        )))
    }
}

/// Revoke one of the account's devices and return the canonical publication
/// acknowledgement. Refreshing the device list is a separate, best-effort
/// operation.
pub async fn revoke_account_device(
    did: String,
) -> Result<RevokeDeviceAcknowledgement, TonkUiError> {
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

/// List every profile signed in (or local) on this browser.
pub async fn list_profiles() -> Result<ProfilesResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/profiles", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/profiles returned {status}: {text}"
        )))
    }
}

/// Record an activation receipt with the worker.
///
/// The activation page posts the emailed invocation straight to the
/// service, so the receipt — and the provider address it names — never
/// passes through the worker. This hands it over, so the registration
/// fact reflects activation the moment it happens.
pub async fn report_activation(receipt: &serde_json::Value) -> Result<(), TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/customer/activated", origin()))
        .json(receipt)
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/customer/activated returned {status}: {text}"
        )))
    }
}

/// Swap the worker onto another roster profile. The caller reloads the
/// page afterwards so every surface re-renders the new profile.
pub async fn activate_profile(profile: String) -> Result<ProfilesResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/profiles/activate", origin()))
        .json(&ActivateProfileRequest { profile })
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/profiles/activate returned {status}: {text}"
        )))
    }
}

/// Rotate onto a fresh profile — the landing pad the normal sign-in
/// ceremony then runs on. The caller reloads the page afterwards.
pub async fn add_account_profile() -> Result<ProfilesResponse, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/profiles/add", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "POST /api/profiles/add returned {status}: {text}"
        )))
    }
}

/// Sign out on this device while preserving its local profile and spaces.
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

/// Load the exact, service-authoritative destructive scope for review.
pub async fn account_deletion_plan() -> Result<AccountDeletionPlan, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .get(format!("{}/api/account/deletion/plan", origin()))
        .send()
        .await
        .map_err(into_api_error)?;
    if response.status().is_success() {
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(TonkUiError::ApiError(format!(
            "GET /api/account/deletion/plan returned {status}: {text}"
        )))
    }
}

/// Execute the root-signed destructive plan in service-safe order.
pub async fn delete_account(
    request: &AccountDeletionRequest,
) -> Result<AccountDeletionResult, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/delete", origin()))
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
            "POST /api/account/delete returned {status}: {text}"
        )))
    }
}

/// Permanently delete one owned hosted space without deleting the account.
pub async fn delete_owned_space(
    request: &tonk_worker_api::AccountSpaceDeletionRequest,
) -> Result<HostedSpaceDeletionResult, TonkUiError> {
    tonk_host::ready::wait().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/account/spaces/delete", origin()))
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
            "POST /api/account/spaces/delete returned {status}: {text}"
        )))
    }
}

/// The account service's error body: `{"error":{"code":…,"message":…}}`.
/// Distinct from [`ErrorBody`], whose `kind` the account service does not
/// emit.
#[derive(Deserialize)]
struct AccountErrorBody {
    error: AccountErrorDetail,
}

#[derive(Deserialize)]
struct AccountErrorDetail {
    message: String,
}

/// Turn a failed account-service response into an error the account
/// panel can show verbatim.
///
/// The service already curates these messages for display ("an account
/// already exists for this email address"), so the message alone is what
/// belongs in front of someone — not the JSON envelope, and not the HTTP
/// status. An unparseable body falls back to the raw text, which is only
/// reachable if the service returned something other than its own error
/// shape.
fn account_service_error(path: &str, status: reqwest::StatusCode, text: &str) -> TonkUiError {
    match serde_json::from_str::<AccountErrorBody>(text) {
        Ok(body) => TonkUiError::Account(body.error.message),
        Err(_) => TonkUiError::ApiError(format!("POST {path} returned {status}: {text}")),
    }
}

/// Submit a signed ceremony container to the account service.
pub async fn submit_account_ceremony(
    service: &str,
    path: &str,
    invocation_hex: &str,
) -> Result<serde_json::Value, TonkUiError> {
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
        response.json().await.map_err(into_api_error)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(account_service_error(path, status, &text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account panel shows whatever string comes back, so the
    /// service's curated sentence has to survive on its own — without the
    /// JSON envelope, the HTTP status, or the "local API" label that
    /// [`TonkUiError::ApiError`] adds.
    #[test]
    fn it_shows_only_the_services_message() {
        let error = account_service_error(
            "/accounts",
            reqwest::StatusCode::CONFLICT,
            r#"{"error":{"code":"CONFLICT","message":"an account already exists for this email address"}}"#,
        );
        assert_eq!(
            error.to_string(),
            "an account already exists for this email address"
        );
    }

    /// A body that isn't the service's error shape keeps the diagnostic
    /// context, since there is no curated message to show instead.
    #[test]
    fn it_falls_back_to_the_raw_body_for_an_unknown_shape() {
        let error = account_service_error(
            "/accounts",
            reqwest::StatusCode::BAD_GATEWAY,
            "<html>upstream is down</html>",
        );
        assert_eq!(
            error.to_string(),
            "Error from local API: POST /accounts returned 502 Bad Gateway: <html>upstream is down</html>"
        );
    }
}

#[cfg(test)]
mod check_claim {
    use super::check_email_claim;

    /// The command reads the address off a form's submit event. The
    /// entry asks while someone is still typing, so it supplies the
    /// value under the same attribute rather than through the DOM.
    #[dialog_common::test]
    fn it_carries_the_address_under_the_form_attribute() {
        let claim = check_email_claim("a@example.com");
        assert_eq!(
            claim["claims"][0]["application"]["parameters"]["email"],
            "a@example.com"
        );
        assert_eq!(
            claim["claims"][0]["application"]["predicate"]["concept"]["with"]["email"]["the"],
            "dom.event.current-target.elements.email/value"
        );
    }

    /// Transient: an answer about a half-typed address has no business
    /// in replicated history.
    #[dialog_common::test]
    fn it_asks_transiently() {
        assert_eq!(
            check_email_claim("a@example.com")["claims"][0]["application"]["predicate"]["kind"],
            "transient"
        );
    }
}

#[cfg(test)]
mod enrollment_claim {
    use super::enroll_claim;

    /// Both fields ride even when unset, because a concept resolves
    /// only when every field is present: a claim that omitted `email`
    /// would never decode, and the command would silently not run.
    #[dialog_common::test]
    fn it_sends_both_fields_even_when_neither_was_given() {
        let claim = enroll_claim(None, &[]);
        let parameters = &claim["claims"][0]["application"]["parameters"];
        assert_eq!(parameters["email"], "");
        assert_eq!(parameters["deposits"], "");
    }

    /// Empty means "the account's recorded address", which is what the
    /// login and resend paths want, so it must be distinguishable from
    /// an address that was given.
    #[dialog_common::test]
    fn it_carries_an_address_when_one_was_given() {
        let claim = enroll_claim(Some("a@example.com"), &[]);
        assert_eq!(
            claim["claims"][0]["application"]["parameters"]["email"],
            "a@example.com"
        );
    }

    /// Deposits join with commas because a command's fields are
    /// scalars, and the worker splits them back apart.
    #[dialog_common::test]
    fn it_joins_ceremony_deposits_into_one_field() {
        let claim = enroll_claim(None, &["ab".to_string(), "cd".to_string()]);
        assert_eq!(
            claim["claims"][0]["application"]["parameters"]["deposits"],
            "ab,cd"
        );
    }

    /// The command is transient: it exists to trigger a handler and is
    /// swept at the commit, never persisted.
    #[dialog_common::test]
    fn it_asserts_a_transient() {
        let claim = enroll_claim(None, &[]);
        assert_eq!(claim["claims"][0]["op"], "assert");
        assert_eq!(
            claim["claims"][0]["application"]["predicate"]["kind"],
            "transient"
        );
    }
}
