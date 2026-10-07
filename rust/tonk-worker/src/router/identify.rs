//! Identity endpoint for retrieving the user's DID.

use ::axum::{Extension, Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::{AppState, ClientId};
use crate::TonkWorkerError;

/// Response containing the user's DID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentifyResponse {
    /// The user's decentralized identifier (DID).
    pub did: String,
}

/// Returns the current user's profile DID.
///
/// This endpoint allows the UI to retrieve the user's persistent identity.
/// The DID is generated on first use and persists across sessions.
#[wasm_compat]
pub async fn identify(
    State(state): State<AppState>,
    client: Option<Extension<ClientId>>,
) -> Result<Json<IdentifyResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;
    // A page asks who it acts as once, as it loads: when that is a profile
    // linked without the account's encryption key, the page is told, to
    // count how many such links are in use.
    if super::identity::linked_without_key(&tonk_state).await {
        super::navigate::notify_analytics(
            client.as_ref().map(|Extension(client)| client),
            tonk_worker_api::AnalyticsEvent::AccountWithoutKey,
        );
    }

    Ok(Json(IdentifyResponse {
        did: tonk_state.profile.did().to_string(),
    }))
}
