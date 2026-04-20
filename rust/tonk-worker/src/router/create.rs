//! Create-repo endpoint.
//!
//! Opens a new self-owned repo, delegates access to the profile, and
//! optionally configures a UCAN sync remote. The response shape is kept
//! symmetric with [`crate::router::ClaimInviteResponse`] so a future
//! `/api/repositories` list endpoint can reuse the per-entry layout
//! without reshaping.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Access service path used when [`RemoteConfig::Default`] is chosen.
/// Mirrors the resolution performed by `router::init`.
const DEFAULT_ACCESS_SERVICE_PATH: &str = "/ucan/";

/// Resolve the absolute default access service URL at runtime.
///
/// In WASM (service worker), resolved against the current origin. In
/// native (tests), returned as a relative path — the test harness does
/// not actually contact the URL, it just round-trips it in the response.
fn default_access_service_url() -> String {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;
        use web_sys::ServiceWorkerGlobalScope;
        let global = js_sys::global()
            .dyn_into::<ServiceWorkerGlobalScope>()
            .expect("Expected ServiceWorkerGlobalScope");
        format!("{}{}", global.location().origin(), DEFAULT_ACCESS_SERVICE_PATH)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        DEFAULT_ACCESS_SERVICE_PATH.to_string()
    }
}

/// Generate a unique local repo name. Used when the caller does not
/// provide one. Collisions are implausible under realistic use because
/// nanosecond timestamps are combined with a monotonic counter.
fn generate_local_name() -> String {
    use dialog_common::time;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = time::now()
        .duration_since(time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("repo-{ts}-{seq}")
}

/// How to configure the sync remote for a newly-created repo.
///
/// Serialized with an explicit `kind` tag so the UI can round-trip the
/// three-way choice ("default" / "url" / "none") without ambiguity.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RemoteConfig {
    /// Use the service worker's built-in default access service.
    #[default]
    Default,
    /// Use a caller-supplied access service URL.
    Url {
        /// Access service URL.
        url: String,
    },
    /// Do not configure any remote — local-only repo.
    None,
}

/// Body for `POST /api/repository/create`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CreateRepositoryRequest {
    /// Local repo name. If absent, an auto-generated name is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Remote configuration. Defaults to the built-in access service.
    #[serde(default)]
    pub remote: RemoteConfig,
}

/// Response from `POST /api/repository/create`. Fields mirror
/// [`crate::router::ClaimInviteResponse`] so the two flows feed the
/// same sidebar row shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateRepositoryResponse {
    /// Whether the repo was successfully created.
    pub success: bool,
    /// Local repo name (storage key, URL path segment, API path segment).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_repo: Option<String>,
    /// Subject DID the repo tracks. For self-owned repos this is the
    /// local repo's own DID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Sync remote URL if one was configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// Whether the default branch has an upstream configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_upstream: Option<bool>,
    /// Error message on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Create a new self-owned repo and optionally wire it up to a remote.
#[wasm_compat]
pub async fn create_repository(
    State(state): State<AppState>,
    Json(body): Json<CreateRepositoryRequest>,
) -> Result<Json<CreateRepositoryResponse>, TonkWorkerError> {
    let local_name = body.name.unwrap_or_else(generate_local_name);
    log!("Creating repo '{}'", local_name);

    let tonk_state = state.write().await;

    let repo = tonk_state
        .profile
        .repository(&local_name)
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to open repo '{local_name}': {e}"))
        })?;
    let subject = repo.did().to_string();

    // Self-delegate so the profile can act on this repo.
    if let Some(access) = repo.try_access() {
        match access
            .claim(&repo)
            .delegate(tonk_state.profile.did())
            .perform(&tonk_state.operator)
            .await
        {
            Ok(chain) => {
                if let Err(e) = tonk_state
                    .profile
                    .access()
                    .save(chain)
                    .perform(&tonk_state.operator)
                    .await
                {
                    log!("Warning: failed to save self-delegation: {e}");
                }
            }
            Err(e) => log!("Warning: failed to self-delegate: {e}"),
        }
    }

    let (remote_url, has_upstream) = match body.remote {
        RemoteConfig::None => (None, Some(false)),
        RemoteConfig::Default | RemoteConfig::Url { .. } => {
            let url = match &body.remote {
                RemoteConfig::Url { url } => url.clone(),
                _ => default_access_service_url(),
            };
            let address = SiteAddress::from(UcanAddress::new(&url));

            match repo
                .remote("origin")
                .create(address)
                .perform(&tonk_state.operator)
                .await
            {
                Ok(_) => {}
                Err(e) if format!("{e:?}").contains("RemoteAlreadyExists") => {}
                Err(e) => {
                    return Err(TonkWorkerError::Internal(format!(
                        "failed to create remote for '{local_name}': {e}"
                    )));
                }
            }

            let branch = repo
                .branch("main")
                .open()
                .perform(&tonk_state.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!(
                        "failed to open main branch on '{local_name}': {e}"
                    ))
                })?;

            let has_upstream = if branch.upstream().is_none() {
                let remote_repo = repo
                    .remote("origin")
                    .load()
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(|e| {
                        TonkWorkerError::Internal(format!("failed to load remote 'origin': {e}"))
                    })?;
                let remote_branch = remote_repo
                    .branch("main")
                    .open()
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(|e| {
                        TonkWorkerError::Internal(format!("failed to open remote main: {e}"))
                    })?;
                branch
                    .set_upstream(&remote_branch)
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(|e| {
                        TonkWorkerError::Internal(format!("failed to set upstream: {e}"))
                    })?;
                true
            } else {
                true
            };

            (Some(url), Some(has_upstream))
        }
    };

    Ok(Json(CreateRepositoryResponse {
        success: true,
        local_repo: Some(local_name),
        subject: Some(subject),
        remote_url,
        has_upstream,
        error: None,
    }))
}
