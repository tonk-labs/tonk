//! Sync routes for push/pull operations with upstream.
//!
//! These endpoints allow synchronizing the local space with the upstream remote.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Response for sync operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether the sync operation succeeded.
    pub success: bool,
    /// Whether any changes were synced.
    pub changed: bool,
    /// Error message if sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pull changes from the upstream remote.
///
/// Fetches changes from the remote and merges them into the local branch.
#[wasm_compat]
pub async fn pull(State(state): State<AppState>) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Pulling from upstream...");

    let mut tonk_state = state.write().await;

    match tonk_state.workspace.space_mut().pull().await {
        Ok(Some(_old_revision)) => {
            log!("Pull succeeded, changes applied");
            Ok(Json(SyncResponse {
                success: true,
                changed: true,
                error: None,
            }))
        }
        Ok(None) => {
            log!("Pull succeeded, already in sync");
            Ok(Json(SyncResponse {
                success: true,
                changed: false,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Push local changes to the upstream remote.
///
/// Sends local changes to the remote.
#[wasm_compat]
pub async fn push(State(state): State<AppState>) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Pushing to upstream...");

    let mut tonk_state = state.write().await;

    match tonk_state.workspace.space_mut().push().await {
        Ok(Some(_old_revision)) => {
            log!("Push succeeded, changes sent");
            Ok(Json(SyncResponse {
                success: true,
                changed: true,
                error: None,
            }))
        }
        Ok(None) => {
            log!("Push succeeded, already in sync");
            Ok(Json(SyncResponse {
                success: true,
                changed: false,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {:?}", e);
            Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Full sync: pull then push.
///
/// First pulls changes from upstream, then pushes local changes.
#[wasm_compat]
pub async fn sync(State(state): State<AppState>) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Syncing with upstream (pull + push)...");

    let mut tonk_state = state.write().await;

    // First pull
    let pull_changed = match tonk_state.workspace.space_mut().pull().await {
        Ok(Some(_)) => {
            log!("Pull succeeded, changes applied");
            true
        }
        Ok(None) => {
            log!("Pull succeeded, already in sync");
            false
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            return Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(format!("Pull failed: {}", e)),
            }));
        }
    };

    // Then push
    let push_changed = match tonk_state.workspace.space_mut().push().await {
        Ok(Some(_)) => {
            log!("Push succeeded, changes sent");
            true
        }
        Ok(None) => {
            log!("Push succeeded, already in sync");
            false
        }
        Err(e) => {
            log!("Push failed: {:?}", e);
            return Ok(Json(SyncResponse {
                success: false,
                changed: pull_changed,
                error: Some(format!("Push failed: {}", e)),
            }));
        }
    };

    Ok(Json(SyncResponse {
        success: true,
        changed: pull_changed || push_changed,
        error: None,
    }))
}
