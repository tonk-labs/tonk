//! Sync routes for syncing changes with the remote.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Response for sync operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether the sync operation succeeded.
    pub success: bool,
    /// Whether any changes were pulled.
    pub pulled: bool,
    /// Whether any changes were pushed.
    pub pushed: bool,
    /// Error message if sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response for pull operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullResponse {
    /// Whether the pull operation succeeded.
    pub success: bool,
    /// Whether any changes were pulled.
    pub updated: bool,
    /// Error message if pull failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Response for push operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PushResponse {
    /// Whether the push operation succeeded.
    pub success: bool,
    /// Whether any changes were pushed.
    pub updated: bool,
    /// Error message if push failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Handles sync requests - performs both pull and push.
///
/// POST /api/sync
#[wasm_compat]
pub async fn sync(State(state): State<AppState>) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!("Full sync with upstream...");

    let mut space = state.write().await;

    // Check if upstream is configured
    if !space.has_upstream().await {
        return Ok(Json(SyncResponse {
            success: false,
            pulled: false,
            pushed: false,
            error: Some("No upstream configured".to_string()),
        }));
    }

    let mut pulled = false;
    let mut pushed = false;
    let mut error = None;

    // Perform pull first
    match space.pull().await {
        Ok(old_revision) => {
            pulled = old_revision.is_some();
            if pulled {
                log!("Pull successful, changes received");
            } else {
                log!("Pull successful, already up to date");
            }
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            error = Some(format!("Pull failed: {}", e));
        }
    }

    // Then push (even if pull failed, try to push local changes)
    if error.is_none() {
        match space.push().await {
            Ok(old_revision) => {
                pushed = old_revision.is_some();
                if pushed {
                    log!("Push successful, changes sent");
                } else {
                    log!("Push successful, already up to date");
                }
            }
            Err(e) => {
                log!("Push failed: {:?}", e);
                error = Some(format!("Push failed: {}", e));
            }
        }
    }

    Ok(Json(SyncResponse {
        success: error.is_none(),
        pulled,
        pushed,
        error,
    }))
}

/// Handles pull requests - pulls changes from the upstream remote.
///
/// POST /api/sync/pull
#[wasm_compat]
pub async fn pull(State(state): State<AppState>) -> Result<Json<PullResponse>, TonkWorkerError> {
    log!("Pulling from upstream...");

    let mut space = state.write().await;

    // Check if upstream is configured
    if !space.has_upstream().await {
        return Ok(Json(PullResponse {
            success: false,
            updated: false,
            error: Some("No upstream configured".to_string()),
        }));
    }

    // Perform pull
    match space.pull().await {
        Ok(old_revision) => {
            let updated = old_revision.is_some();
            if updated {
                log!("Pull successful, changes received");
            } else {
                log!("Pull successful, already up to date");
            }
            Ok(Json(PullResponse {
                success: true,
                updated,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            Ok(Json(PullResponse {
                success: false,
                updated: false,
                error: Some(format!("Pull failed: {}", e)),
            }))
        }
    }
}

/// Handles push requests - pushes changes to the upstream remote.
///
/// POST /api/sync/push
#[wasm_compat]
pub async fn push(State(state): State<AppState>) -> Result<Json<PushResponse>, TonkWorkerError> {
    log!("Pushing to upstream...");

    let mut space = state.write().await;

    // Check if upstream is configured
    if !space.has_upstream().await {
        log!("No upstream configured, skipping push");
        return Ok(Json(PushResponse {
            success: true,
            updated: false,
            error: None,
        }));
    }

    // Perform push
    match space.push().await {
        Ok(old_revision) => {
            let updated = old_revision.is_some();
            if updated {
                log!("Push successful, changes sent");
            } else {
                log!("Push successful, already up to date");
            }
            Ok(Json(PushResponse {
                success: true,
                updated,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {:?}", e);
            Ok(Json(PushResponse {
                success: false,
                updated: false,
                error: Some(format!("Push failed: {}", e)),
            }))
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_space;
    use super::*;
    use crate::api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn sync_returns_error_without_upstream() {
        let space = test_space().await;
        let app = api_router(space);

        let request = Request::builder()
            .uri("/api/sync")
            .method("POST")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let sync_response: SyncResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(!sync_response.success);
        assert!(sync_response.error.is_some());
    }

    #[dialog_common::test]
    async fn pull_returns_error_without_upstream() {
        let space = test_space().await;
        let app = api_router(space);

        let request = Request::builder()
            .uri("/api/sync/pull")
            .method("POST")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let pull_response: PullResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(!pull_response.success);
        assert!(pull_response.error.is_some());
    }

    #[dialog_common::test]
    async fn push_succeeds_without_upstream() {
        let space = test_space().await;
        let app = api_router(space);

        let request = Request::builder()
            .uri("/api/sync/push")
            .method("POST")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let push_response: PushResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        // Push without upstream should succeed but not update
        assert!(push_response.success);
        assert!(!push_response.updated);
    }
}
