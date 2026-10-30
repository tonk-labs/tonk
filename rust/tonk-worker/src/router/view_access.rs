//! `POST /api/view/access`: a framed view's peer takes the delegation an
//! MCP host's `tonk_view_access` signed to it (plan #1200 stage 7, route
//! A).
//!
//! In a host that frames the provider's origin, the view's peer is this
//! worker, in the frame's own storage partition: its profile key is the
//! view's key, and the relay asks the host to delegate the shown space to
//! the profile's DID ([`identify`](super::identify)). The frame hands the
//! chains here; each must end at this profile, so a chain for anyone else
//! is refused rather than saved. They are saved as the profile's own
//! authority, and the space is mounted as a replica of `remote`, as a
//! join mounts one. Asking again, with a renewed delegation, saves the
//! new chains over the same replica.

use ::axum::{Json, extract::State};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{Container, DelegationChain};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_handler_macros::wasm_compat;
use tonk_schema::prelude::DidExt as _;

use super::AppState;
use crate::TonkWorkerError;

/// Body of `POST /api/view/access`.
#[derive(Debug, Deserialize)]
pub struct ViewAccessRequest {
    /// The shown space.
    pub space: String,
    /// The provider's `/ucan/`, which the space syncs with.
    pub remote: String,
    /// The chains `tonk_view_access` answered, base64url containers.
    pub delegations: Vec<String>,
}

/// Answer of `POST /api/view/access`.
#[derive(Debug, Serialize)]
pub struct ViewAccessResponse {
    /// The space's repository key.
    pub repo: String,
}

#[wasm_compat]
pub async fn accept(
    State(state): State<AppState>,
    Json(body): Json<ViewAccessRequest>,
) -> Result<Json<ViewAccessResponse>, TonkWorkerError> {
    let bad = |error: &dyn std::fmt::Display| TonkWorkerError::Router(error.to_string());
    let space: Did = body.space.parse().map_err(|e| bad(&e))?;
    let remote = url::Url::parse(&body.remote).map_err(|e| bad(&e))?;
    if body.delegations.is_empty() {
        return Err(bad(&"no delegation to accept"));
    }
    let tonk = state.read().await;
    let profile = tonk.profile.did();
    let mut chains = Vec::new();
    for encoded in &body.delegations {
        let container = Container::decode(encoded.trim().as_bytes()).map_err(|e| bad(&e))?;
        let chain = DelegationChain::try_from(container).map_err(|e| bad(&e))?;
        if chain.audience() != &profile || chain.subject().is_some_and(|subject| subject != &space)
        {
            return Err(TonkWorkerError::Forbidden(
                "a delegation for another key or space".into(),
            ));
        }
        chains.push(chain);
    }
    for chain in chains {
        tonk.profile
            .access()
            .save(UcanDelegation(chain))
            .perform(&tonk.operator)
            .await
            .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    }
    super::join::mount_replica(&tonk, &space, Some(remote.as_str()), None).await?;
    super::repository::record_initialized_replica_in_profile(&tonk, &space)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    Ok(Json(ViewAccessResponse {
        repo: space.repo_key().to_owned(),
    }))
}
