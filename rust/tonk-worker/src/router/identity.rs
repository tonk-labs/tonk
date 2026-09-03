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
use crate::worker::{DefaultOperator, TonkState};
use dialog_operator::Profile;

const LOCAL_ROOT_SITE: &str = "tonk-local-root-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalRootRecord {
    version: u8,
    credential_id: String,
    delegation: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passkey: Option<PasskeyMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encryption_key: Option<String>,
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
    /// The account's X25519 recipient, when the ceremony that wrote this
    /// record held the secret. Published to the account space by
    /// `seed_sealed_inbox`.
    pub encryption_key: Option<dialog_varsig::Did>,
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
        .verify_signature(&dialog_credentials::DidKeyResolver)
        .await
        .map_err(|error| {
            TonkWorkerError::Forbidden(format!("invalid root delegation signature: {error}"))
        })?;
    Ok(chain)
}

pub(crate) async fn load_record(
    state: &TonkState,
) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
    load_record_from(&state.profile, &state.operator).await
}

/// Load and validate the serialized root record belonging to an explicit
/// profile. Account routing uses this without constructing a full TonkState
/// for every inactive roster entry.
async fn load_record_from(
    profile: &Profile,
    operator: &DefaultOperator,
) -> Result<Option<LocalRootRecord>, TonkWorkerError> {
    let bytes = match profile
        .credential()
        .site(LOCAL_ROOT_SITE)
        .load::<Vec<u8>>()
        .perform(operator)
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

/// Return the verified historical account root for an explicit profile.
/// A missing record is a rootless profile; a malformed or misaddressed grant
/// is an unreadable profile and is never treated as a match.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn historical_root_did(
    profile: &Profile,
    operator: &DefaultOperator,
) -> Result<Option<dialog_varsig::Did>, TonkWorkerError> {
    let Some(record) = load_record_from(profile, operator).await? else {
        return Ok(None);
    };
    let delegation = validate_grant(record.delegation, &profile.did()).await?;
    Ok(Some(delegation.issuer().clone()))
}

/// Load and validate the local root, failing when it is missing.
pub(crate) async fn local_root(state: &TonkState) -> Result<LocalRoot, TonkWorkerError> {
    let record = load_record(state)
        .await?
        .ok_or(TonkWorkerError::RootRequired)?;
    let device_did = state.profile.did();
    let delegation = validate_grant(record.delegation.clone(), &device_did).await?;
    let encryption_key = record
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    Ok(LocalRoot {
        root_did: delegation.issuer().clone(),
        device_did,
        credential_id: record.credential_id,
        delegation,
        bytes: record.delegation,
        passkey: record.passkey,
        encryption_key,
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
        encryption_key: root.encryption_key.map(|did| did.to_string()),
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

/// Rewrite the local root record without its recipient: the shape of a
/// device linked before the encryption key existed, for tests of what
/// such a device does when it needs one.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn forget_encryption_key(state: &TonkState) -> Result<(), TonkWorkerError> {
    let Some(mut record) = load_record(state).await? else {
        return Err(TonkWorkerError::RootRequired);
    };
    record.encryption_key = None;
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
        .map_err(|error| TonkWorkerError::Internal(format!("failed to save local root: {error}")))
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
    request
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = validate_grant(bytes.clone(), &state.profile.did()).await?;
    let mut record = LocalRootRecord {
        version: 1,
        credential_id: request.credential_id,
        delegation: bytes.clone(),
        passkey,
        encryption_key: request.encryption_key,
    };
    if let Some(existing) = load_record(state).await? {
        let stored_root = DelegationChain::try_from(existing.delegation.as_slice())
            .ok()
            .map(|stored| stored.issuer().clone());
        // Only a ceremony that held the secret records the recipient, so
        // a re-save from one that did not (a link, a re-minted grant)
        // keeps the one already recorded for this root.
        if stored_root.as_ref() == Some(chain.issuer()) && record.encryption_key.is_none() {
            record.encryption_key = existing.encryption_key.clone();
        }
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
        //
        // A root is replaceable only when this profile has NO account
        // attachment history at all: a creation ceremony binds the
        // root before registration can still fail (an email already
        // taken, a service outage), and that half-created record must
        // not wedge the profile — a retry may replace it. A stored
        // attachment OR the sign-out tombstone both refuse: signed out
        // or signed in, this profile's spaces and delegations hang off
        // the stored root, and a different account arrives through
        // add-account, never by rebinding this profile.
        if stored_root.as_ref() != Some(chain.issuer()) {
            if super::account::has_attachment_history(state).await {
                return Err(TonkWorkerError::Conflict(
                    "a different account is already signed in on this profile; \
                     use \"Add account\" to sign in with another account"
                        .to_string(),
                ));
            }
            tonk_common::log!(
                "replacing a dangling account root (no attachment was ever made): {} -> {}",
                stored_root
                    .map(|did| did.to_string())
                    .unwrap_or_else(|| "unreadable".to_string()),
                chain.issuer()
            );
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

    let encryption_key = record
        .encryption_key
        .as_deref()
        .map(parse_encryption_key)
        .transpose()?;
    // An operation may be waiting on exactly this: a page answered the
    // worker's request for a passkey assertion by saving the key.
    if let Some(recipient) = &encryption_key {
        super::custody::notify_encryption_key(recipient);
    }
    Ok(status(LocalRoot {
        root_did: chain.issuer().clone(),
        device_did: state.profile.did(),
        credential_id: record.credential_id,
        delegation: chain,
        bytes,
        passkey: record.passkey,
        encryption_key,
    }))
}

/// Parse a ceremony-supplied recipient, refusing anything that is not
/// an X25519 `did:key`: a record holding an Ed25519 DID here would seal
/// every seed to a key nothing can open.
fn parse_encryption_key(did: &str) -> Result<dialog_varsig::Did, TonkWorkerError> {
    let did: dialog_varsig::Did = did
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid encryptionKey: {error}")))?;
    tonk_identity::sealed::RecipientKey::try_from(&did)
        .map_err(|error| TonkWorkerError::Router(format!("invalid encryptionKey: {error}")))?;
    Ok(did)
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
    // signup or at a later custody enrollment. The passkey's own row is
    // written by the ceremony, which is the only place that holds both
    // the custody DID it keys on and the creation label. Seed the
    // sealed-inbox address now rather than on the next sweep, so the
    // dashboard reflects the enrollment immediately.
    if super::account_state::seed_sealed_inbox(&state).await {
        tonk_common::log!("published the account's encryption key in the account space");
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

    use crate::router::tests::{test_state, test_state_without_account, test_state_without_root};
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
                encryption_key: None,
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

    fn recipient_did(byte: u8) -> dialog_varsig::Did {
        tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([byte; 32]))
            .secret()
            .did()
    }

    #[dialog_common::test]
    async fn it_keeps_the_encryption_key_with_the_local_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let recipient = recipient_did(1);
        let (mut request, _) = request_for(1, &device).await;
        request.encryption_key = Some(recipient.to_string());
        let _ = save(State(state.clone()), Json(request)).await.unwrap();
        assert_eq!(
            local_root(&*state.read().await)
                .await
                .unwrap()
                .encryption_key,
            Some(recipient.clone())
        );

        // A later ceremony on the same root that did not hold the secret
        // (a re-minted grant) leaves the recorded recipient in place.
        let (again, _) = request_for(1, &device).await;
        let _ = save(State(state.clone()), Json(again)).await.unwrap();
        assert_eq!(
            local_root(&*state.read().await)
                .await
                .unwrap()
                .encryption_key,
            Some(recipient)
        );
    }

    #[dialog_common::test]
    async fn it_rejects_an_encryption_key_that_is_not_x25519() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        let device = state.read().await.profile.did();
        let (mut request, grant) = request_for(1, &device).await;
        request.encryption_key = Some(grant.issuer().to_string());
        assert!(matches!(
            save(State(state), Json(request)).await,
            Err(TonkWorkerError::Router(_))
        ));
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

    /// The dangling case `has_attachment_history` exists for: a
    /// creation ceremony saved the root, registration failed, and no
    /// attachment (nor sign-out tombstone) was ever written. A retry
    /// with a fresh passkey must replace the leftover root instead of
    /// wedging the profile.
    #[dialog_common::test]
    async fn it_replaces_a_dangling_root_when_no_attachment_was_ever_made() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let device = state.read().await.profile.did();
        let (replacement, _) = request_for(2, &device).await;
        let replacement_credential = replacement.credential_id.clone();

        let status = save(State(state.clone()), Json(replacement)).await.unwrap();
        assert!(
            matches!(
                status.0,
                RootStatus::Ready { credential_id, .. } if credential_id == replacement_credential
            ),
            "the retry's credential must be the persisted one"
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
            encryption_key: None,
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
