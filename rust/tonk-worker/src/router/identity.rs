//! Persist and validate the provider-neutral local passkey root.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{PasskeyMetadata, RootStatus, SaveRootRequest};

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

const LOCAL_ROOT_SITE: &str = "tonk-local-root-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalRootRecord {
    version: u8,
    credential_id: String,
    delegation: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passkey: Option<PasskeyMetadata>,
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
    /// Informational creation details recorded by the creating browser.
    pub passkey: Option<PasskeyMetadata>,
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

pub(crate) async fn load_record(
    state: &TonkState,
) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
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
        passkey: record.passkey,
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
        passkey: root.passkey,
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
    let passkey = request
        .passkey
        .map(|mut metadata| {
            metadata.created_on = metadata.created_on.trim().to_string();
            if metadata.created_at == 0 {
                return Err(TonkWorkerError::Router(
                    "passkey createdAt must be greater than zero".to_string(),
                ));
            }
            if metadata.created_on.is_empty()
                || metadata.created_on.chars().count() > 120
                || metadata.created_on.chars().any(char::is_control)
            {
                return Err(TonkWorkerError::Router(
                    "passkey createdOn must be a readable device label".to_string(),
                ));
            }
            Ok(metadata)
        })
        .transpose()?;
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = validate_grant(bytes.clone(), &state.profile.did()).await?;
    let record = LocalRootRecord {
        version: 1,
        credential_id: request.credential_id,
        delegation: bytes.clone(),
        passkey,
    };
    if let Some(existing) = load_record(state).await? {
        if existing == record {
            return Ok(status(local_root(state).await?));
        }
        // Same root, different record — a re-minted grant or refreshed
        // metadata — updates in place; that covers signing back in after
        // sign-out. A DIFFERENT root is another account's passkey: each
        // account keeps its own profile, so it lands through add-account
        // and never overwrites the root this profile's spaces hang off.
        // An unreadable stored delegation refuses too — the roots can't
        // be proven equal.
        let stored_root = DelegationChain::try_from(existing.delegation.as_slice())
            .ok()
            .map(|stored| stored.issuer().clone());
        if stored_root.as_ref() != Some(chain.issuer()) {
            return Err(TonkWorkerError::Conflict(
                "a different account is already signed in on this profile; \
                 use \"Add account\" to sign in with another account"
                    .to_string(),
            ));
        }
    }

    // Saving the new grant below leaves earlier access certificates in
    // the profile store intact.
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
        passkey: record.passkey,
    }))
}

/// `POST /api/identity/root`.
#[wasm_compat]
pub async fn save(
    State(state): State<AppState>,
    Json(request): Json<SaveRootRequest>,
) -> Result<Json<RootStatus>, TonkWorkerError> {
    let state = state.read().await;
    let status = persist_root(&state, request).await?;
    // A save that carries creation metadata is a passkey arriving — at
    // signup or at a later custody enrollment. Seed the account facts
    // now rather than on the next sweep, so the dashboard reflects the
    // enrollment immediately. Idempotent and best-effort.
    if super::account_state::seed_passkey_facts(&state).await {
        tonk_common::log!("recorded this device's passkey creation facts in the account space");
    }
    Ok(Json(status))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use axum::extract::State;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use crate::router::tests::{test_state, test_state_without_root};
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
                passkey: None,
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
    async fn it_preserves_passkey_creation_metadata_with_the_local_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let (request, _) = request_for(1, &device).await;
        let mut request = serde_json::to_value(request).unwrap();
        request["passkey"] = serde_json::json!({
            "createdAt": 1_754_380_800,
            "createdOn": "Chrome on macOS",
        });
        let request = serde_json::from_value(request).unwrap();

        let _ = save(State(state.clone()), Json(request)).await.unwrap();
        let Json(result) = get(State(state)).await.unwrap();
        let result = serde_json::to_value(result).unwrap();
        assert_eq!(result["passkey"]["createdAt"], 1_754_380_800u64);
        assert_eq!(result["passkey"]["createdOn"], "Chrome on macOS");
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
        let state = Arc::new(RwLock::new(test_state().await));
        let device = state.read().await.profile.did();
        let (second, _) = request_for(2, &device).await;

        assert!(matches!(
            save(State(state), Json(second)).await,
            Err(TonkWorkerError::Conflict(_))
        ));
    }

    /// Sign-out keeps the profile's root, data, and certificates; a
    /// later ceremony that resolves a DIFFERENT passkey is another
    /// account arriving, and each account gets its own profile via
    /// add-account. Persisting it here would silently rebind this
    /// profile's spaces to a root that never owned them.
    #[dialog_common::test]
    async fn it_rejects_a_different_root_on_a_previously_linked_profile() {
        let state = Arc::new(RwLock::new(test_state().await));
        let previous_root = {
            let state = state.read().await;
            local_root(&state).await.unwrap().root_did
        };
        let device = state.read().await.profile.did();
        let (replacement, _) = request_for(2, &device).await;

        let _ = super::super::account::unlink(State(state.clone()))
            .await
            .unwrap();
        let error = save(State(state.clone()), Json(replacement))
            .await
            .unwrap_err();

        assert!(matches!(error, TonkWorkerError::Conflict(_)));
        let current_root = {
            let state = state.read().await;
            local_root(&state).await.unwrap().root_did
        };
        assert_eq!(
            current_root, previous_root,
            "the refused ceremony must leave the persisted root untouched"
        );
    }

    /// The same root signing back in after sign-out updates in place —
    /// tightening the different-root path must not break re-sign-in.
    #[dialog_common::test]
    async fn it_accepts_the_same_root_again_after_signing_out() {
        let state = Arc::new(RwLock::new(test_state().await));
        let (device, profile_name) = {
            let state = state.read().await;
            (state.profile.did(), state.profile_name.clone())
        };
        // Re-derive the fixture's own root and mint a FRESH grant for it:
        // same root, different record bytes — what a real re-sign-in
        // ceremony produces.
        let root = Ed25519Signer::import(&crate::router::tests::test_root_seed(&profile_name))
            .await
            .unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root, &device)
            .await
            .unwrap();
        let request = SaveRootRequest {
            credential_id: "renewed-credential".to_string(),
            delegation_hex: hex::encode(grant.to_bytes().unwrap()),
            passkey: None,
        };

        let _ = super::super::account::unlink(State(state.clone()))
            .await
            .unwrap();
        let Json(status) = save(State(state.clone()), Json(request)).await.unwrap();

        assert!(matches!(
            status,
            RootStatus::Ready { root_did, .. } if root_did == grant.issuer().to_string()
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
