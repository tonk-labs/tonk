//! Sync routes for push/pull operations with upstream.
//!
//! These endpoints allow synchronizing the local space with the upstream remote.

use ::axum::{Json, extract::{Path, State}};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

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
pub async fn pull(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Pulling from upstream...");

    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let mut session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    let result = match session.space_mut().pull().await {
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
    };

    // Update the cached session
    tonk_state.update_session(session).await;

    result
}

/// Push local changes to the upstream remote.
///
/// Sends local changes to the remote.
#[wasm_compat]
pub async fn push(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Pushing to upstream...");

    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let mut session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    let result = match session.space_mut().push().await {
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
    };

    // Update the cached session
    tonk_state.update_session(session).await;

    result
}

/// Full sync: pull then push.
///
/// First pulls changes from upstream, then pushes local changes.
#[wasm_compat]
pub async fn sync(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Syncing with upstream (pull + push)...");

    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let mut session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    // First pull
    let pull_changed = match session.space_mut().pull().await {
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
            tonk_state.update_session(session).await;
            return Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(format!("Pull failed: {}", e)),
            }));
        }
    };

    // Then push
    let push_changed = match session.space_mut().push().await {
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
            tonk_state.update_session(session).await;
            return Ok(Json(SyncResponse {
                success: false,
                changed: pull_changed,
                error: Some(format!("Push failed: {}", e)),
            }));
        }
    };

    // Update the cached session
    tonk_state.update_session(session).await;

    Ok(Json(SyncResponse {
        success: true,
        changed: pull_changed || push_changed,
        error: None,
    }))
}
