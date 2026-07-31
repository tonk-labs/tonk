//! Persist and validate the provider-neutral local passkey root.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{RootStatus, SaveRootRequest};

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

const LOCAL_ROOT_SITE: &str = "tonk-local-root-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LocalRootRecord {
    version: u8,
    credential_id: String,
    delegation: Vec<u8>,
}

/// A validated local root record.
#[derive(Clone)]
pub(crate) struct LocalRoot {
    /// Passkey-derived root DID.
    pub root_did: dialog_varsig::Did,
    /// Current device DID.
    pub device_did: dialog_varsig::Did,
    /// Opaque passkey credential ID.
    pub credential_id: String,
    /// Exact root → device delegation.
    pub delegation: DelegationChain,
    /// Exact serialized delegation bytes.
    pub bytes: Vec<u8>,
}

pub(crate) async fn validate_grant(
    bytes: Vec<u8>,
    device_did: &dialog_varsig::Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let chain = DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("invalid root delegation: {error}")))?;
    if chain.proof_cids().len() != 1 {
        return Err(TonkWorkerError::Router(
            "root delegation must contain exactly one proof".to_string(),
        ));
    }
    if chain.audience() != device_did {
        return Err(TonkWorkerError::Forbidden(
            "root delegation audience is not the current profile".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(TonkWorkerError::Router(
            "root delegation must be subject-open".to_string(),
        ));
    }
    let proof = chain.proofs().next().expect("one-proof chain");
    if !proof.command().0.is_empty() {
        return Err(TonkWorkerError::Router(
            "root delegation must be command-open".to_string(),
        ));
    }
    proof
        .verify_signature(&dialog_credentials::Ed25519KeyResolver)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid root delegation signature: {error}"))
        })?;
    Ok(chain)
}

async fn load_record(state: &TonkState) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
    let bytes = match state
        .profile
        .credential()
        .site(LOCAL_ROOT_SITE)
        .load::<Vec<u8>>()
        .perform(&state.operator)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) if crate::credential::is_missing(&error) => return Ok(None),
        Err(error) => {
            return Err(TonkWorkerError::Internal(format!(
                "failed to load local root: {error}"
            )));
        }
    };
    let record: LocalRootRecord = serde_json::from_slice(&bytes).map_err(|error| {
        TonkWorkerError::Internal(format!("stored local root is malformed: {error}"))
    })?;
    if record.version != 1 {
        return Err(TonkWorkerError::Internal(format!(
            "unsupported local root version {}",
            record.version
        )));
    }
    Ok(Some(record))
}

/// Load and validate the local root, failing when it is missing.
pub(crate) async fn local_root(state: &TonkState) -> Result<LocalRoot, TonkWorkerError> {
    let record = load_record(state)
        .await?
        .ok_or(TonkWorkerError::RootRequired)?;
    let device_did = state.profile.did();
    let delegation = validate_grant(record.delegation.clone(), &device_did).await?;
    Ok(LocalRoot {
        root_did: delegation.issuer().clone(),
        device_did,
        credential_id: record.credential_id,
        delegation,
        bytes: record.delegation,
    })
}

/// Return the verified local root DID.
#[allow(dead_code)]
pub(crate) async fn root_did(state: &TonkState) -> Result<dialog_varsig::Did, TonkWorkerError> {
    Ok(local_root(state).await?.root_did)
}

fn status(root: LocalRoot) -> RootStatus {
    RootStatus::Ready {
        root_did: root.root_did.to_string(),
        device_did: root.device_did.to_string(),
        credential_id: root.credential_id,
        delegation_cid: root.delegation.proof_cids()[0].to_string(),
        delegation_hex: hex::encode(root.bytes),
    }
}

/// `GET /api/identity/root`.
#[wasm_compat]
pub async fn get(State(state): State<AppState>) -> Result<Json<RootStatus>, TonkWorkerError> {
    let state = state.read().await;
    match load_record(&state).await? {
        None => Ok(Json(RootStatus::Missing {
            device_did: state.profile.did().to_string(),
        })),
        Some(_) => Ok(Json(status(local_root(&state).await?))),
    }
}

pub(crate) async fn persist_root(
    state: &TonkState,
    request: SaveRootRequest,
) -> Result<RootStatus, TonkWorkerError> {
    if request.credential_id.is_empty() {
        return Err(TonkWorkerError::Router(
            "credentialId must not be empty".to_string(),
        ));
    }
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = validate_grant(bytes.clone(), &state.profile.did()).await?;
    let record = LocalRootRecord {
        version: 1,
        credential_id: request.credential_id,
        delegation: bytes.clone(),
    };
    if let Some(existing) = load_record(state).await? {
        if existing != record {
            return Err(TonkWorkerError::Conflict(
                "a different local root is already persisted".to_string(),
            ));
        }
        return Ok(status(local_root(state).await?));
    }

    state
        .profile
        .access()
        .save(UcanDelegation(chain.clone()))
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save root delegation: {error}"))
        })?;
    let encoded = serde_json::to_vec(&record).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to serialize local root: {error}"))
    })?;
    state
        .profile
        .credential()
        .site(LOCAL_ROOT_SITE)
        .save(encoded)
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save local root: {error}"))
        })?;

    Ok(status(LocalRoot {
        root_did: chain.issuer().clone(),
        device_did: state.profile.did(),
        credential_id: record.credential_id,
        delegation: chain,
        bytes,
    }))
}

/// `POST /api/identity/root`.
#[wasm_compat]
pub async fn save(
    State(state): State<AppState>,
    Json(request): Json<SaveRootRequest>,
) -> Result<Json<RootStatus>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(persist_root(&state, request).await?))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use crate::router::tests::test_state_without_root;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    async fn request_for(
        root_seed: u8,
        device: &dialog_varsig::Did,
    ) -> (SaveRootRequest, DelegationChain) {
        let root = Ed25519Signer::import(&[root_seed; 32]).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root, device)
            .await
            .unwrap();
        (
            SaveRootRequest {
                credential_id: format!("credential-{root_seed}"),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
            },
            grant,
        )
    }

    #[dialog_common::test]
    async fn it_reports_a_missing_local_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let Json(result) = get(State(state)).await.unwrap();
        assert!(matches!(result, RootStatus::Missing { .. }));
    }

    #[dialog_common::test]
    async fn it_persists_and_reloads_a_local_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let (request, grant) = request_for(1, &device).await;

        let _ = save(State(state.clone()), Json(request)).await.unwrap();
        let Json(result) = get(State(state)).await.unwrap();
        assert!(matches!(result, RootStatus::Ready { delegation_cid, .. }
            if delegation_cid == grant.proof_cids()[0].to_string()));
    }

    #[dialog_common::test]
    async fn it_rejects_a_grant_for_another_device() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let other = Ed25519Signer::import(&[9u8; 32]).await.unwrap().did();
        let (request, _) = request_for(1, &other).await;

        assert!(matches!(
            save(State(state), Json(request)).await,
            Err(TonkWorkerError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_replacing_a_ready_root_with_another_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let (first, _) = request_for(1, &device).await;
        let (second, _) = request_for(2, &device).await;
        let _ = save(State(state.clone()), Json(first)).await.unwrap();

        assert!(matches!(
            save(State(state), Json(second)).await,
            Err(TonkWorkerError::Conflict(_))
        ));
    }

    #[dialog_common::test]
    async fn it_accepts_an_idempotent_repeat_of_the_same_record() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let (request, _) = request_for(1, &device).await;

        let _ = save(State(state.clone()), Json(request.clone()))
            .await
            .unwrap();
        let _ = save(State(state), Json(request)).await.unwrap();
    }
}
