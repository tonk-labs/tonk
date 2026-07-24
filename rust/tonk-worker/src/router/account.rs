//! Persist and inspect the current profile's account-root delegation.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_effects::credential::CredentialError;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
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

async fn validate_succession(
    succession_hex: &str,
    old_root: &dialog_varsig::Did,
    new_root: &dialog_varsig::Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let bytes = hex::decode(succession_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid succession hex: {error}")))?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("invalid succession chain: {error}")))?;
    if chain.proof_cids().len() != 1 {
        return Err(TonkWorkerError::Router(
            "succession must contain exactly one proof".to_string(),
        ));
    }
    if chain.issuer() != old_root {
        return Err(TonkWorkerError::Forbidden(
            "succession issuer is not the linked account root".to_string(),
        ));
    }
    if chain.audience() != new_root {
        return Err(TonkWorkerError::Forbidden(
            "succession audience is not the new account root".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(TonkWorkerError::Router(
            "succession must be subject-open".to_string(),
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
            TonkWorkerError::Forbidden(format!("invalid succession signature: {error}"))
        })?;
    Ok(chain)
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
        Ok(bytes) => {
            // An empty value is the unlink tombstone: the credential
            // store has no delete, so signing out writes empty bytes.
            if bytes.is_empty() {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
        Err(CredentialError::NotFound(_)) => Ok(None),
        Err(error) => Err(TonkWorkerError::Internal(format!(
            "failed to load local account link: {error}"
        ))),
    }
}

/// The stored `root → device` delegation for this profile, or `None`
/// when the profile is unlinked or the stored link is unreadable.
///
/// Fail-safe: an unreadable or malformed link resolves to `None`, so the
/// device behaves exactly as an unlinked one and keeps working.
pub(crate) async fn account_link(tonk: &crate::worker::TonkState) -> Option<DelegationChain> {
    let bytes = match load_link(tonk).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(error) => {
            log!("account link unreadable; treating profile as unlinked: {error}");
            return None;
        }
    };
    match DelegationChain::try_from(bytes.as_slice()) {
        Ok(chain) => Some(chain),
        Err(error) => {
            log!("account link malformed; treating profile as unlinked: {error}");
            None
        }
    }
}

/// The account root DID for this profile, or `None` when unlinked. A
/// linked device knows its root without holding the root key: the
/// `root → device` delegation names the root as issuer.
pub(crate) async fn account_root_did(
    tonk: &crate::worker::TonkState,
) -> Option<dialog_varsig::Did> {
    account_link(tonk).await.map(|chain| chain.issuer().clone())
}

/// The DID roster writes and invite claims key on: the account root when
/// linked, otherwise this device's own DID.
pub(crate) async fn member_did(tonk: &crate::worker::TonkState) -> dialog_varsig::Did {
    match account_root_did(tonk).await {
        Some(root) => root,
        None => tonk.profile.did(),
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

/// Validate and store a `root → current profile` delegation.
///
/// This is the persistence half of [`link`], split out so it can be used
/// without the handler's post-link convergence dispatch. Tests that only
/// need a linked profile call this directly: going through [`link`] would
/// also fire the background sweep, which races whatever the test does
/// next.
pub(crate) async fn persist_link(
    state: &crate::worker::TonkState,
    request: &AccountLinkRequest,
) -> Result<(), TonkWorkerError> {
    let device_did = state.profile.did();
    let (chain, bytes) = validate_link(request, &device_did).await?;

    if let Some(existing) = load_link(state).await? {
        let existing = DelegationChain::try_from(existing.as_slice()).map_err(|error| {
            TonkWorkerError::Internal(format!("stored account delegation is invalid: {error}"))
        })?;
        if existing.issuer() != chain.issuer() {
            let Some(succession_hex) = &request.succession_hex else {
                return Err(TonkWorkerError::Conflict(
                    "profile is already linked to another account root".to_string(),
                ));
            };
            let succession =
                validate_succession(succession_hex, existing.issuer(), chain.issuer()).await?;
            state
                .profile
                .access()
                .save(UcanDelegation(succession))
                .perform(&state.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to save succession delegation: {error}"
                    ))
                })?;
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
    Ok(())
}

/// Validate and persist a `root → current profile` delegation, then
/// converge this device's spaces onto the account in the background.
#[wasm_compat]
pub async fn link(
    State(state): State<AppState>,
    Json(request): Json<AccountLinkRequest>,
) -> Result<Json<AccountStatus>, TonkWorkerError> {
    // Keep the cloneable `AppState` handle around: `state` is about to be
    // shadowed by a read guard for the body of this handler, but the
    // fire-and-forget convergence dispatch below needs its own independent
    // lock inside a detached task. Native awaits restore inline using
    // the guard already held below, so it never needs this clone.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let app_state = state.clone();
    let state = state.read().await;
    let device_did = state.profile.did();
    // Only the wasm dispatch below acts on a root change; capturing it
    // unconditionally would leave the native build with an unused binding.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let previous_root = account_root_did(&state).await;
    persist_link(&state, &request).await?;

    // A freshly linked device converges its existing spaces and pulls the
    // account's backed-up ones in the background — a slow/hung account
    // service must never stall the link response. On native there's no UI
    // to stall, so it awaits inline using the guard already held here.
    //
    // Migration runs under the WRITE lock. It is a purely local storage
    // sweep (its backup requests are themselves dispatched detached), and
    // handlers hold read locks while writing, so a read lock here would let
    // the sweep's transactions run concurrently with a handler's against
    // the same stores — which the storage layer rejects. The write lock
    // makes the sweep mutually exclusive with request handling instead.
    // It cannot deadlock: `spawn_local` defers the task, so this handler's
    // read guard is dropped before the task asks for the lock.
    //
    // Restore deliberately stays on a read lock: it awaits account-service
    // round trips, and holding the write lock across those would stall
    // every handler on that latency — the one thing this dispatch exists
    // to avoid. The lock is therefore released between the two.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let new_root_did = request.root_did.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // Migrate this device's existing spaces onto the root first:
            // restore only mounts subjects this device doesn't already
            // have, so running migration first means restore won't
            // re-touch a space migration just re-keyed.
            {
                let tonk = app_state.write().await;
                if let Some(old_root) = previous_root.filter(|old| old.to_string() != new_root_did)
                {
                    crate::router::migrate::converge_after_rotation(&tonk, &old_root).await;
                }
                crate::router::migrate::migrate_rosters(&tonk).await;
            }
            let tonk = app_state.read().await;
            crate::router::restore::restore_spaces(&tonk).await;
        });
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        crate::router::restore::restore_spaces(&state).await;
    }

    Ok(Json(AccountStatus::Linked {
        root_did: request.root_did,
        device_did: device_did.to_string(),
    }))
}

/// Clear the stored account link for this profile — local sign-out.
///
/// Writes an empty tombstone over the stored link (the credential store
/// has no delete effect). The `root → device` delegation saved into the
/// access store at link time has no removal API and stays behind: a
/// signed-out device that is not also *revoked* still holds a usable
/// delegation. Revocation, not unlink, is the security boundary.
#[wasm_compat]
pub async fn unlink(State(state): State<AppState>) -> Result<Json<AccountStatus>, TonkWorkerError> {
    let state = state.read().await;
    state
        .profile
        .credential()
        .site(ACCOUNT_LINK_SITE)
        .save(Vec::new())
        .perform(&state.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to clear local account link: {error}"))
        })?;
    Ok(Json(AccountStatus::Unlinked {
        device_did: state.profile.did().to_string(),
    }))
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
        root_did,
        delegation_hex: hex::encode(delegation.to_bytes().unwrap()),
        succession_hex: None,
    }
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
        tests_request_for(root_seed, audience).await
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

    #[dialog_common::test]
    async fn it_replaces_the_root_when_a_succession_authorizes_it() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let _ = link(State(state.clone()), Json(first.clone()))
            .await
            .unwrap();

        let old_root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&[8u8; 32])
            .await
            .unwrap();
        let succession = tonk_identity::delegation::mint_root_succession(old_root, &new_root.did())
            .await
            .unwrap();
        let mut second = request_for(&[8u8; 32], device_did).await;
        second.succession_hex = Some(hex::encode(succession.to_bytes().unwrap()));

        let Json(status) = link(State(state.clone()), Json(second.clone()))
            .await
            .unwrap();
        match status {
            AccountStatus::Linked { root_did, .. } => assert_eq!(root_did, second.root_did),
            AccountStatus::Unlinked { .. } => panic!("relink did not persist"),
        }
    }

    #[dialog_common::test]
    async fn it_rejects_a_succession_from_the_wrong_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let _ = link(State(state.clone()), Json(first)).await.unwrap();

        // Succession issued by an unrelated key, not the linked root.
        let stranger = tonk_identity::derive::derive_root_signer(&[13u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&[8u8; 32])
            .await
            .unwrap();
        let succession = tonk_identity::delegation::mint_root_succession(stranger, &new_root.did())
            .await
            .unwrap();
        let mut second = request_for(&[8u8; 32], device_did).await;
        second.succession_hex = Some(hex::encode(succession.to_bytes().unwrap()));

        assert!(matches!(
            link(State(state), Json(second)).await,
            Err(TonkWorkerError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_succession_minted_for_a_different_root() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let first = request_for(&[7u8; 32], device_did.clone()).await;
        let _ = link(State(state.clone()), Json(first)).await.unwrap();

        // Succession correctly issued by the linked root, but audienced to
        // a third DID — not the root this relink actually claims.
        let old_root = tonk_identity::derive::derive_root_signer(&[7u8; 32])
            .await
            .unwrap();
        let third_root = tonk_identity::derive::derive_root_signer(&[42u8; 32])
            .await
            .unwrap();
        let succession =
            tonk_identity::delegation::mint_root_succession(old_root, &third_root.did())
                .await
                .unwrap();
        let mut second = request_for(&[8u8; 32], device_did).await;
        second.succession_hex = Some(hex::encode(succession.to_bytes().unwrap()));

        assert!(matches!(
            link(State(state), Json(second)).await,
            Err(TonkWorkerError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_resolves_the_member_did_to_the_root_when_linked() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        let expected_root = request.root_did.clone();
        let _ = link(State(state.clone()), Json(request)).await.unwrap();

        let tonk = state.read().await;
        assert_eq!(member_did(&tonk).await.to_string(), expected_root);
        assert_eq!(
            account_root_did(&tonk).await.map(|did| did.to_string()),
            Some(expected_root),
        );
    }

    #[dialog_common::test]
    async fn it_resolves_the_member_did_to_the_device_when_unlinked() {
        let state = Arc::new(RwLock::new(test_state().await));
        let tonk = state.read().await;
        let device_did = tonk.profile.did();
        assert_eq!(member_did(&tonk).await, device_did);
        assert!(account_root_did(&tonk).await.is_none());
    }

    #[dialog_common::test]
    async fn it_unlinks_and_returns_to_the_unlinked_state() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }

        let Json(after) = unlink(State(state.clone())).await.unwrap();
        assert_eq!(
            after,
            AccountStatus::Unlinked {
                device_did: device_did.to_string()
            }
        );
        let Json(loaded) = get(State(state.clone())).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Unlinked { .. }));

        // The tombstone must read as "no link", not as a malformed link.
        let tonk = state.read().await;
        assert!(account_link(&tonk).await.is_none());
    }

    #[dialog_common::test]
    async fn it_relinks_the_same_root_after_an_unlink() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }
        let _ = unlink(State(state.clone())).await.unwrap();
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }
        let Json(loaded) = get(State(state)).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Linked { .. }));
    }
}
