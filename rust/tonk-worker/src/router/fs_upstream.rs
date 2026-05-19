//! Configure a branch to track a `dialog-remote-fs` directory as its
//! upstream.
//!
//! `POST /api/repository/{repo}/branch/{branch}/upstream/fs/{vault_id}`
//!
//! Idempotent: if the `"fs"` remote already exists with the same vault
//! id, the call just re-points the branch's upstream at it. If it
//! exists with a different vault id, returns `409 Conflict` — switching
//! a repo's FS-remote target needs an explicit reset.
//!
//! The handle for `vault_id` must already have been registered via
//! [`TonkServiceWorker::register_fs_handle`](crate::TonkServiceWorker::register_fs_handle)
//! before any subsequent `sync` invocation; this route only wires the
//! metadata, it doesn't touch the disk.

use ::axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use axum_wasm_macros::wasm_compat;
use dialog_remote_fs::FsAddress;
use dialog_repository::{LoadRemoteError, RepositoryExt as _};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Conventional name for the FS-remote on a repository. There's only
/// ever one — switching the underlying vault id rewrites this entry.
const FS_REMOTE_NAME: &str = "fs";

/// Path parameters: which repo/branch/vault to wire up.
#[derive(Debug, Deserialize)]
pub struct FsUpstreamPath {
    /// Repository name (local).
    pub repo: String,
    /// Branch name (local) — also the branch name on the FS remote.
    pub branch: String,
    /// Opaque vault id — must match what was registered with
    /// `registerFsHandle`.
    pub vault_id: String,
}

/// Response shape.
#[derive(Debug, Serialize, Deserialize)]
pub struct FsUpstreamResponse {
    /// `true` when the upstream points at the requested vault id.
    pub success: bool,
    /// The remote's name (always `"fs"` today).
    pub remote: String,
    /// The vault id the remote points at.
    pub vault_id: String,
}

#[wasm_compat]
pub async fn set_fs_upstream(
    State(state): State<AppState>,
    Path(params): Path<FsUpstreamPath>,
) -> Result<(StatusCode, Json<FsUpstreamResponse>), TonkWorkerError> {
    log!(
        "Setting FS upstream: repo={} branch={} vault_id={}",
        params.repo,
        params.branch,
        params.vault_id
    );

    let tonk = state.write().await;

    let repo = tonk
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("repository '{}': {e}", params.repo)))?;

    let address = FsAddress::new(&params.vault_id);

    // Idempotency: if "fs" remote already exists, only proceed when it
    // points at the same vault id. Replacing it with a different vault
    // would silently re-target a user's branch to unfamiliar data —
    // require an explicit reset (deferred to a future route).
    let remote = match repo.remote(FS_REMOTE_NAME).load().perform(&tonk.operator).await {
        Ok(existing) => {
            let existing_address = existing.address();
            let proposed_site = address.clone().into();
            if existing_address.site() != &proposed_site {
                return Err(TonkWorkerError::Conflict(
                    "FS remote already configured with a different vault id".to_string(),
                ));
            }
            existing
        }
        Err(LoadRemoteError::NotFound { .. }) => repo
            .remote(FS_REMOTE_NAME)
            .create(address)
            .perform(&tonk.operator)
            .await
            .map_err(|e| TonkWorkerError::Router(format!("create FS remote: {e}")))?,
        Err(e) => {
            return Err(TonkWorkerError::Router(format!(
                "load FS remote: {e}",
            )));
        }
    };

    let remote_branch = remote
        .branch(&params.branch)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("open remote branch: {e}")))?;

    let local_branch = repo
        .branch(&params.branch)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(format!("branch '{}': {e}", params.branch)))?;

    local_branch
        .set_upstream(&remote_branch)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("set upstream: {e}")))?;

    Ok((
        StatusCode::OK,
        Json(FsUpstreamResponse {
            success: true,
            remote: FS_REMOTE_NAME.to_string(),
            vault_id: params.vault_id,
        }),
    ))
}
