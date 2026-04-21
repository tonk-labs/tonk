//! Branch initialization route for setting up UCAN remote and upstream.
//!
//! The init endpoint creates a UCAN-based remote ("origin") and sets it as
//! the upstream for the given branch. This replaces the old authorize endpoint.

use ::axum::{Json, extract::Path, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Access service path (will be resolved to absolute URL at runtime).
const ACCESS_SERVICE_PATH: &str = "/ucan/";

/// Get the absolute URL for the access service.
///
/// In WASM (service worker), we resolve against the current origin.
/// In native, we return the path as-is (for testing).
fn get_access_service_url() -> String {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;
        use web_sys::ServiceWorkerGlobalScope;

        let global = js_sys::global()
            .dyn_into::<ServiceWorkerGlobalScope>()
            .expect("Expected ServiceWorkerGlobalScope");

        let origin = global.location().origin();
        format!("{}{}", origin, ACCESS_SERVICE_PATH)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        ACCESS_SERVICE_PATH.to_string()
    }
}

/// Path parameters for init endpoint.
#[derive(Debug, Deserialize)]
pub struct InitPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response for init operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitResponse {
    /// Whether the initialization succeeded.
    pub success: bool,
    /// Error message if initialization failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Initializes a branch with a UCAN remote and upstream configuration.
///
/// 1. Opens (or creates) the repository by name
/// 2. Delegates repo access to the profile if needed
/// 3. Creates a UCAN remote ("origin") pointing to the access service
/// 4. Opens the branch
/// 5. Sets the remote branch as upstream
#[wasm_compat]
pub async fn init(
    State(state): State<AppState>,
    Path(params): Path<InitPath>,
) -> Result<Json<InitResponse>, TonkWorkerError> {
    log!(
        "Initializing branch: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

    // 1. Open (create-or-load) repository
    let repo = tonk_state
        .profile
        .repository(&params.repo)
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "Failed to open repository '{}': {}",
                params.repo, e
            ))
        })?;
    log!("Repository DID: {}", repo.did());

    // 2. Delegate repo access to profile if possible
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
                    log!("Warning: failed to save repo delegation: {}", e);
                }
            }
            Err(e) => {
                log!("Warning: failed to delegate repo to profile: {}", e);
            }
        }
    }

    // 3. Create UCAN remote ("origin")
    let service_url = get_access_service_url();
    log!("Setting up UCAN remote with URL: {}", service_url);

    let ucan_address = UcanAddress::new(&service_url);
    let site_address = SiteAddress::from(ucan_address);

    match repo
        .remote("origin")
        .create(site_address)
        .perform(&tonk_state.operator)
        .await
    {
        Ok(remote) => {
            log!("Remote 'origin' created: {}", remote.site().name());
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("RemoteAlreadyExists") {
                log!("Remote 'origin' already configured, skipping create");
            } else {
                return Err(TonkWorkerError::Internal(format!(
                    "Failed to create remote: {}",
                    e
                )));
            }
        }
    }

    // 4. Open branch
    let branch = repo
        .branch(params.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", params.branch, e))
        })?;

    // 5. Set upstream if not already configured
    if branch.upstream().is_none() {
        log!(
            "Setting upstream for branch '{}' to origin/{}",
            params.branch,
            params.branch
        );

        // Load the remote and get a remote branch handle
        let remote_repo = repo
            .remote("origin")
            .load()
            .perform(&tonk_state.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("Failed to load remote 'origin': {}", e))
            })?;

        let remote_branch = remote_repo
            .branch(params.branch.as_str())
            .open()
            .perform(&tonk_state.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "Failed to open remote branch '{}': {}",
                    params.branch, e
                ))
            })?;

        branch
            .set_upstream(&remote_branch)
            .perform(&tonk_state.operator)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("Failed to set upstream: {}", e)))?;

        log!("Upstream set successfully");
    } else {
        log!("Upstream already configured, skipping");
    }

    Ok(Json(InitResponse {
        success: true,
        error: None,
    }))
}
