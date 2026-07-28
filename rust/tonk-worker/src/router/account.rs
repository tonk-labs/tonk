//! Attach optional provider services to the provider-neutral local root.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_effects::credential::CredentialError;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{AccountLinkRequest, AccountStatus};

use super::AppState;
use crate::TonkWorkerError;

const ACCOUNT_PROVIDER_SITE: &str = "tonk-account-provider-v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ProviderRecord {
    version: u8,
    provider: String,
    attached_at: u64,
}

async fn load_provider(
    state: &crate::worker::TonkState,
) -> Result<Option<ProviderRecord>, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => return Ok(None),
        Ok(bytes) => bytes,
        Err(CredentialError::NotFound(_)) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load account provider: {error}"
            )));
        }
    };
    let record: ProviderRecord = serde_json::from_slice(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored account provider is malformed: {error}"))
    })?;
    if record.version != 1 {
        return Err(TonkWorkerError::Internal(format!(
            "unsupported account provider version {}",
            record.version
        )));
    }
    Ok(Some(record))
}

/// Attached provider base URL, if any.
pub(crate) async fn provider(state: &crate::worker::TonkState) -> Option<String> {
    load_provider(state)
        .await
        .ok()
        .flatten()
        .map(|record| record.provider)
}

/// The stable local root grant, available to provider operations only when attached.
pub(crate) async fn account_link(
    state: &crate::worker::TonkState,
) -> Option<dialog_ucan_core::DelegationChain> {
    provider(state).await?;
    super::identity::local_root(state)
        .await
        .ok()
        .map(|root| root.delegation)
}

/// The local root DID used for every durable membership operation.
pub(crate) async fn member_did(
    state: &crate::worker::TonkState,
) -> Result<dialog_varsig::Did, TonkWorkerError> {
    super::identity::root_did(state).await
}

async fn status(state: &crate::worker::TonkState) -> Result<AccountStatus, TonkWorkerError> {
    let device_did = state.profile.did().to_string();
    let root = match super::identity::local_root(state).await {
        Ok(root) => root,
        Err(TonkWorkerError::RootRequired) => {
            return Ok(AccountStatus::RootMissing { device_did });
        }
        Err(error) => return Err(error),
    };
    match load_provider(state).await? {
        None => Ok(AccountStatus::Unregistered {
            root_did: root.root_did.to_string(),
            device_did,
        }),
        Some(provider) => Ok(AccountStatus::Registered {
            root_did: root.root_did.to_string(),
            device_did,
            provider: provider.provider,
        }),
    }
}

/// Return local-root and provider attachment state.
#[wasm_compat]
pub async fn get(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(status(&state).await?))
}

/// Validate that provider ceremony metadata exactly matches the local root,
/// then store provider metadata without changing authority.
pub(crate) async fn persist_link(
    state: &crate::worker::TonkState,
    request: &AccountLinkRequest,
) -> Result<(), TonkWorkerError> {
    if request.provider.trim().is_empty() {
        return Err(TonkWorkerError::Router(
            "provider must not be empty".to_string(),
        ));
    }
    let root = super::identity::local_root(state).await?;
    if request.root_did != root.root_did.to_string()
        || request.credential_id != root.credential_id
        || request.delegation_hex != hex::encode(&root.bytes)
    {
        return Err(TonkWorkerError::Forbidden(
            "provider ceremony does not match the persisted local root".to_string(),
        ));
    }
    if let Some(existing) = load_provider(state).await?
        && existing.provider != request.provider
    {
        return Err(TonkWorkerError::Conflict(
            "another account provider is already attached".to_string(),
        ));
    }
    let record = ProviderRecord {
        version: 1,
        provider: request.provider.trim_end_matches('/').to_string(),
        attached_at: web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let bytes = serde_json::to_vec(&record).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize account provider: {error}"))
    })?;
    state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save account provider: {error}"))
        })?;
    Ok(())
}

/// Attach provider services and start best-effort backup/restore.
#[wasm_compat]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let app_state = state.clone();
    let state = state.read().await;
    persist_link(&state, &request).await?;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_futures::spawn_local(async move {
        let state = app_state.read().await;
        crate::router::account_backup::back_up_existing_spaces(&state).await;
        crate::router::restore::restore_spaces(&state).await;
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    crate::router::restore::restore_spaces(&state).await;

    Ok(Json(status(&state).await?))
}

/// Disconnect provider services while preserving the local root and spaces.
#[wasm_compat]
pub async fn unlink(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    state
        .profile
        .credential()
        .site(ACCOUNT_PROVIDER_SITE)
        .save(Vec::<u8>::new())
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to clear account provider: {error}"))
        })?;
    Ok(Json(status(&state).await?))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn tests_request_for(
    root_seed: &[u8; 32],
    audience: dialog_varsig::Did,
) -> tonk_worker_api::AccountLinkRequest {
    use dialog_varsig::Principal;
    let root = tonk_identity::derive::derive_root_signer(root_seed)
        .await
        .unwrap();
    let root_did = root.did().to_string();
    let delegation = tonk_identity::delegation::mint_device_delegation(root, &audience)
        .await
        .unwrap();
    tonk_worker_api::AccountLinkRequest {
        provider: "https://accounts.tonk.xyz".into(),
        root_did,
        credential_id: "test-credential".into(),
        delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use crate::router::tests::{test_state, test_state_without_root};
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn matching_request(state: &crate::worker::TonkState) -> AccountLinkRequest {
        let root = super::super::identity::local_root(state).await.unwrap();
        AccountLinkRequest {
            provider: "https://accounts.tonk.xyz".into(),
            root_did: root.root_did.to_string(),
            credential_id: root.credential_id,
            delegation_hex: hex::encode(root.bytes),
        }
    }

    #[dialog_common::test]
    async fn it_reports_an_unregistered_local_root_without_an_account() {
        let state = Arc::new(RwLock::new(test_state().await));
        let Json(status) = get(State(state)).await.unwrap();
        assert!(matches!(status, AccountStatus::Unregistered { .. }));
    }

    #[dialog_common::test]
    async fn it_reports_a_missing_root_separately() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let Json(status) = get(State(state)).await.unwrap();
        assert!(matches!(status, AccountStatus::RootMissing { .. }));
    }

    #[dialog_common::test]
    async fn it_attaches_a_provider_without_replacing_the_root_grant() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = {
            let state = state.read().await;
            super::super::identity::local_root(&state)
                .await
                .unwrap()
                .bytes
        };
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        let after = {
            let state = state.read().await;
            super::super::identity::local_root(&state)
                .await
                .unwrap()
                .bytes
        };
        assert_eq!(before, after);
    }

    #[dialog_common::test]
    async fn it_detaches_a_provider_without_revoking_or_rotating_the_device() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = state.read().await.profile.did();
        let request = {
            let state = state.read().await;
            matching_request(&state).await
        };
        let _ = link(State(state.clone()), Json(request)).await.unwrap();
        let Json(status) = unlink(State(state.clone())).await.unwrap();
        assert!(matches!(status, AccountStatus::Unregistered { .. }));
        assert_eq!(state.read().await.profile.did(), before);
    }
}
