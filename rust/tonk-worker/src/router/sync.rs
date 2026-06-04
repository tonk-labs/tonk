//! Sync routes for push/pull operations with upstream.
//!
//! These endpoints allow synchronizing a branch with its upstream remote.
//! Each response carries the local branch revision before and after the
//! operation so the UI can render a diff (or detect "nothing changed").
//!
//! Pull/push run through the reactor's chain so a successful pull
//! re-polls every subscription on the branch (push doesn't change
//! local state, so it doesn't re-poll).

use ::axum::{Json, extract::Path, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{SyncState, classify};

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for sync endpoints.
#[derive(Debug, Deserialize)]
pub struct SyncPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response for sync operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether the sync operation succeeded.
    pub success: bool,
    /// Local branch revision *before* the sync ran. `None` when
    /// the branch had no commits at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Revision>,
    /// Local branch revision *after* the sync. `None` when the
    /// branch still has no commits, or when the operation failed
    /// before producing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Revision>,
    /// Error message if sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pull changes from the upstream remote.
#[wasm_compat]
pub async fn pull(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pulling from upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .pull()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(after) => {
            log!("Pull succeeded");
            Ok(Json(SyncResponse {
                success: true,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {e:?}");
            Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Push local changes to the upstream remote.
#[wasm_compat]
pub async fn push(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pushing to upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .push()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(_) => {
            log!("Push succeeded");
            Ok(Json(SyncResponse {
                success: true,
                before: before.clone(),
                after: session.handle().revision(),
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {e:?}");
            Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Sync-state of a branch relative to its upstream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    /// How the local head relates to the upstream head.
    pub state: SyncState,
    /// Local branch revision, or `null` if it has no commits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<Revision>,
    /// Upstream branch revision as last fetched, or `null` if the
    /// upstream has no commits (or none is configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Revision>,
}

/// Classify a branch against its upstream without mutating local
/// state.
///
/// Reads the local head, fetches the upstream head read-only (no
/// merge, no push), and runs the shared classifier. A branch with
/// no upstream returns [`SyncState::NoUpstream`] rather than an
/// error, so the indicator has something to render for every
/// branch.
#[wasm_compat]
pub async fn sync_status(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncStatusResponse>, TonkWorkerError> {
    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let handle = session.handle();
    let local = handle.revision();

    if handle.upstream().is_none() {
        return Ok(Json(SyncStatusResponse {
            state: SyncState::NoUpstream,
            local,
            remote: None,
        }));
    }

    let remote = handle
        .fetch()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(e.to_string()))?;

    let sync_state = classify(local.as_ref(), remote.as_ref());
    Ok(Json(SyncStatusResponse {
        state: sync_state,
        local,
        remote,
    }))
}

/// Full sync: pull then push.
#[wasm_compat]
pub async fn sync(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Syncing with upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    let after_pull = match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .pull()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(after) => {
            log!("Pull succeeded");
            after
        }
        Err(e) => {
            log!("Pull failed: {e:?}");
            return Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(format!("Pull failed: {e}")),
            }));
        }
    };

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .push()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(_) => {
            log!("Push succeeded");
            Ok(Json(SyncResponse {
                success: true,
                before,
                after: session.handle().revision(),
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {e:?}");
            Ok(Json(SyncResponse {
                success: false,
                before,
                after: after_pull,
                error: Some(format!("Push failed: {e}")),
            }))
        }
    }
}
