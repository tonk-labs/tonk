//! Persist and inspect the current profile's account-root delegation.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_effects::credential::CredentialError;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{AccountLinkRequest, AccountStatus};

use super::AppState;
use crate::TonkWorkerError;

const ACCOUNT_LINK_SITE: &str = "tonk-account-link-v1";

async fn validate_link(
    request: &AccountLinkRequest,
    device_did: &dialog_varsig::Did,
) -> Result<(DelegationChain, Vec<u8>), TonkWorkerError> {
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("invalid account delegation: {error}")))?;

    if chain.proof_cids().len() != 1 {
        return Err(TonkWorkerError::Router(
            "account delegation must contain exactly one proof".to_string(),
        ));
    }
    if chain.issuer().to_string() != request.root_did {
        return Err(TonkWorkerError::Forbidden(
            "delegation issuer does not match rootDid".to_string(),
        ));
    }
    if chain.audience() != device_did {
        return Err(TonkWorkerError::Forbidden(
            "delegation audience is not the current profile".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(TonkWorkerError::Router(
            "account delegation must be subject-open".to_string(),
        ));
    }

    let proof = chain
        .proofs()
        .next()
        .expect("a one-proof chain contains one proof");
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid account delegation signature: {error}"))
        })?;

    Ok((chain, bytes))
}

async fn load_link(state: &crate::worker::TonkState) -> Result<Option<Vec<u8>>, TonkWorkerError> {
    match state
        .profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) => Ok(Some(bytes)),
        Err(CredentialError::NotFound(_)) => Ok(None),
        Err(error) => Err(TonkWorkerError::Internal(format!(
            "failed to load local account link: {error}"
        ))),
    }
}

/// Return the current profile's local account-link state.
#[wasm_compat]
pub async fn get(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    let device_did = state.profile.did();
    let Some(bytes) = load_link(&state).await? else {
        return Ok(Json(AccountStatus::Unlinked {
            device_did: device_did.to_string(),
        }));
    };
    let chain = DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        TonkWorkerError::Internal(format!("stored account delegation is invalid: {error}"))
    })?;
    if chain.audience() != &device_did {
        return Err(TonkWorkerError::Internal(
            "stored account delegation targets another profile".to_string(),
        ));
    }

    Ok(Json(AccountStatus::Linked {
        root_did: chain.issuer().to_string(),
        device_did: device_did.to_string(),
    }))
}

/// Validate and persist a `root → current profile` delegation.
#[wasm_compat]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    let device_did = state.profile.did();
    let (chain, bytes) = validate_link(&request, &device_did).await?;

    if let Some(existing) = load_link(&state).await? {
        let existing = DelegationChain::try_from(existing.as_slice()).map_err(|error| {
            TonkWorkerError::Internal(format!("stored account delegation is invalid: {error}"))
        })?;
        if existing.issuer() != chain.issuer() {
            return Err(TonkWorkerError::Conflict(
                "profile is already linked to another account root".to_string(),
            ));
        }
    }

    state
        .profile
        .access()
        .save(UcanDelegation(chain))
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save account delegation: {error}"))
        })?;
    state
        .profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(bytes)
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save local account link: {error}"))
        })?;

    Ok(Json(AccountStatus::Linked {
        root_did: request.root_did,
        device_did: device_did.to_string(),
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use std::sync::Arc;

    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    use crate::router::tests::test_state;

    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn request_for(root_seed: &[u8; 32], audience: dialog_varsig::Did) -> AccountLinkRequest {
        let root = tonk_identity::derive::derive_root_signer(root_seed)
            .await
            .unwrap();
        let root_did = root.did().to_string();
        let delegation = tonk_identity::delegation::mint_device_delegation(root, &audience)
            .await
            .unwrap();
        AccountLinkRequest {
            root_did,
            delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        }
    }

    #[dialog_common::test]
    async fn it_round_trips_an_account_link_idempotently() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        let expected_bytes = hex::decode(&request.delegation_hex).unwrap();

        let Json(first) = link(State(state.clone()), Json(request.clone()))
            .await
            .unwrap();
        let Json(second) = link(State(state.clone()), Json(request)).await.unwrap();
        let Json(loaded) = get(State(state.clone())).await.unwrap();

        for status in [first, second, loaded] {
            match status {
                AccountStatus::Linked {
                    root_did,
                    device_did: linked_device,
                } => {
                    assert!(root_did.starts_with("did:key:z6Mk"));
                    assert_eq!(linked_device, device_did.to_string());
                }
                AccountStatus::Unlinked { .. } => panic!("account link was not persisted"),
            }
        }

        let stored = {
            let state = state.read().await;
            state
                .profile
                .credential()
                .site(ACCOUNT_LINK_SITE)
                .load::<Vec<u8>>()
                .perform(&state.operator)
                .await
                .unwrap()
        };
        assert_eq!(stored, expected_bytes);
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegation_for_another_profile() {
        let state = Arc::new(RwLock::new(test_state().await));
        let other = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let request = request_for(&[7u8; 32], other.did()).await;
        let Json(before) = get(State(state.clone())).await.unwrap();

        assert!(matches!(
            link(State(state.clone()), Json(request)).await,
            Err(TonkWorkerError::Forbidden(_))
        ));
        let Json(after) = get(State(state)).await.unwrap();
        assert_eq!(after, before);
    }

    #[dialog_common::test]
    async fn it_does_not_replace_an_existing_account_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let second = request_for(&[8u8; 32], device_did).await;

        let _ = link(State(state.clone()), Json(first)).await.unwrap();
        assert!(matches!(
            link(State(state), Json(second)).await,
            Err(TonkWorkerError::Conflict(_))
        ));
    }
}
