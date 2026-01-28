//! Branch inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::super::AppState;
use crate::TonkWorkerError;

/// Response for branch status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchStatusResponse {
    /// The branch name.
    pub name: String,
    /// Current revision period.
    pub period: usize,
    /// Current revision moment.
    pub moment: usize,
    /// Upstream info if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<UpstreamStatusResponse>,
}

/// Upstream status info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamStatusResponse {
    /// The site name (None for local upstream).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// The branch name on the upstream.
    pub branch: String,
    /// The upstream revision period if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<usize>,
    /// The upstream revision moment if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moment: Option<usize>,
}

/// Returns the status of a specific branch.
#[wasm_compat]
pub async fn branch(
    State(state): State<AppState>,
    Path(branch_name): Path<String>,
) -> Result<Json<BranchStatusResponse>, TonkWorkerError> {
    log!("Querying branch status for: {}", branch_name);
    let tonk_state = state.read().await;

    match tonk_state.space.branch_info(&branch_name).await {
        Ok(branch_info) => {
            let upstream = branch_info.upstream.map(|u| {
                let (period, moment) = u
                    .revision
                    .map(|r| (Some(r.period), Some(r.moment)))
                    .unwrap_or((None, None));
                UpstreamStatusResponse {
                    site: u.site,
                    branch: u.branch,
                    period,
                    moment,
                }
            });

            Ok(Json(BranchStatusResponse {
                name: branch_info.name,
                period: branch_info.revision.period,
                moment: branch_info.revision.moment,
                upstream,
            }))
        }
        Err(e) => {
            log!("Error getting branch info: {:?}", e);
            Err(TonkWorkerError::Internal(format!(
                "Failed to get branch info: {}",
                e
            )))
        }
    }
}
