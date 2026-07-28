//! Persist and inspect the current profile's account-root delegation.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_effects::credential::CredentialError;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_worker_api::{AccountLinkRequest, AccountStatus, SignOutResponse};

use super::AppState;
use crate::TonkWorkerError;

const ACCOUNT_LINK_SITE: &str = "tonk-account-link-v1";

async fn validate_link(
    request: &AccountLinkRequest,
    device_did: &dialog_varsig::Did,
) -> Result<(DelegationChain, Vec<u8>), TonkWorkerError> {
    let bytes = hex::decode(&request.delegation_hex)
        .map_err(|error| TonkWorkerError::Router(format!("invalid delegation hex: {error}")))?;
    let chain = super::identity::validate_grant(bytes.clone(), device_did).await?;
    if chain.issuer().to_string() != request.root_did {
        return Err(TonkWorkerError::Forbidden(
            "delegation issuer does not match rootDid".to_string(),
        ));
    }
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
        wasm_bindgen_futures::spawn_local(async move {
            // Migrate this device's existing spaces onto the root first:
            // restore only mounts subjects this device doesn't already
            // have, so running migration first means restore won't
            // re-touch a space migration just re-keyed.
            {
                let tonk = app_state.write().await;
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

/// Sign out on this device: revoke it in the registry, then rotate onto
/// a fresh key.
///
/// Clearing the local link alone would leave a signed-out device still
/// holding a usable `root → device` delegation — the access store has no
/// removal API — so sign-out revokes instead. That makes the registry
/// agree with what the user just asked for, and it is the only half of
/// this that a stolen device cannot undo.
///
/// Revocation is permanent for a key, and the access-service screen
/// matches a revoked device DID against every chain that DID issues a
/// hop in, unscoped by account. Keeping the same profile afterwards
/// would therefore refuse *all* presigns from this browser, local spaces
/// included, forever. So the device rotates onto a new profile in the
/// same movement, and the old key is simply left behind.
///
/// Nothing outside the device is keyed to the device DID — a linked
/// profile's roster entries name the account root, and its spaces are
/// escrowed under that root — so logging back in restores them. The
/// costs are a passkey prompt to link again, and any space that was
/// never sync-enabled, which was never escrowed and does not come back.
#[wasm_compat]
pub async fn unlink(
    State(state): State<AppState>,
) -> Result<Json<SignOutResponse>, TonkWorkerError> {
    // Revoke first, while this device's key is still the one the
    // registry knows. Best-effort: an unreachable account service must
    // not strand the user on a device they asked to sign out of, and
    // the local half is what they can see. Failure is carried into the
    // response rather than aborting: the user asked to leave this
    // device, and they can finish the revocation from another one.
    let (revoked, warning) = match revoke_this_device(&state).await {
        SelfRevocation::Recorded => (true, None),
        SelfRevocation::NothingToRecord => (false, None),
        SelfRevocation::Failed(cause) => {
            log!("sign-out revoked nothing in the registry: {cause}");
            (false, Some(cause))
        }
    };

    let storage = { state.read().await.storage.clone() };
    let (profile_name, profile) = crate::device::rotate(&storage).await?;
    let session = crate::session::open(&profile, &storage).await?;
    let device_did = profile.did().to_string();

    {
        let mut tonk = state.write().await;
        // The reactor caches repositories and branches opened as the
        // old profile. They are not this device's to serve any more, so
        // it is replaced rather than flushed.
        tonk.reactor = crate::Reactor::new(profile.clone());
        tonk.profile = profile;
        tonk.operator = session.operator;
        tonk.session_expires_at = session.expires_at;
        tonk.profile_name = profile_name;
        tonk.sync_queue = Default::default();
    }

    // Lay down the replacement profile's meta branch. Not fatal: by
    // here the device is revoked and the key rotated, so reporting a
    // failure would name an action that did happen and leave nothing to
    // retry. Boot bootstraps the profile too, so a miss here heals on
    // the next worker start.
    {
        let tonk = state.read().await;
        if let Err(error) = crate::router::repository::bootstrap_profile(&tonk).await {
            log!("replacement profile not bootstrapped, deferring to next boot: {error}");
        }
    }

    Ok(Json(SignOutResponse {
        device_did,
        revoked,
        warning,
    }))
}

/// What became of the registry half of a sign-out.
enum SelfRevocation {
    /// The registry accepted this device's self-revocation.
    Recorded,
    /// There was no registry to tell: the profile was never linked, or
    /// this deployment has no account service.
    NothingToRecord,
    /// The registry should have been told and was not. The device still
    /// signs out locally; the cause travels to the user, who can revoke
    /// from another device.
    Failed(String),
}

/// Tell the account registry this device is out, signing the revocation
/// with the device's own key.
///
/// A self-revoke is the one revocation a device can always make: it
/// needs no passkey and no root, because the only thing it cuts off is
/// itself. Nothing here is fatal — a profile that was never linked has
/// nothing to revoke, and a failed call leaves a device the registry
/// still believes in, which is the state it was already in.
async fn revoke_this_device(state: &AppState) -> SelfRevocation {
    use dialog_ucan_core::promise::Promised;

    // Read what the call needs and let the lock go: the rest of this
    // signs and then waits on the network, and nothing else may touch
    // the state while a guard is held across those awaits.
    let (link, device, device_did) = {
        let tonk = state.read().await;
        let Some(link) = account_link(&tonk).await else {
            return SelfRevocation::NothingToRecord; // never linked
        };
        (
            link,
            tonk.profile.signer().signer().clone(),
            tonk.profile.did().to_string(),
        )
    };
    let Some(service) = crate::router::account_backup::account_service_url() else {
        return SelfRevocation::NothingToRecord;
    };
    let target = link.proof_cids()[0];

    let revocation =
        match tonk_identity::revocation::mint_self_revocation(device.clone(), &link, &target).await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                return SelfRevocation::Failed(format!(
                    "could not sign a revocation for this device: {error}"
                ));
            }
        };

    let arguments = [
        ("did".to_owned(), Promised::String(device_did)),
        (
            "revocation".to_owned(),
            Promised::String(hex::encode(revocation)),
        ),
    ]
    .into_iter()
    .collect();

    let body = match tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    {
        Ok(body) => body,
        Err(error) => {
            return SelfRevocation::Failed(format!(
                "could not build the revoke invocation: {error}"
            ));
        }
    };

    let endpoint = format!("{}/devices/revoke", service.trim_end_matches('/'));
    match crate::router::account_backup::post_for_bytes(&endpoint, body).await {
        Ok(_) => SelfRevocation::Recorded,
        Err(error) => SelfRevocation::Failed(format!(
            "could not reach the account service to revoke this device: {error}"
        )),
    }
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
        // The test scope has no account service, so there was no
        // registry to tell — which is not a failure and must not warn.
        assert!(!after.revoked);
        assert!(
            after.warning.is_none(),
            "nothing-to-record is not a failure worth warning about"
        );
        let Json(loaded) = get(State(state.clone())).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Unlinked { .. }));

        let tonk = state.read().await;
        assert!(account_link(&tonk).await.is_none());
    }

    /// The device this browser presents afterwards must not be the one
    /// sign-out just revoked. Revocation is permanent for a key and the
    /// access-service screen matches the device DID unscoped by account,
    /// so coming back as the same profile would refuse every presign
    /// this browser ever makes again.
    #[dialog_common::test]
    async fn it_signs_out_onto_a_key_it_did_not_just_revoke() {
        let state = Arc::new(RwLock::new(test_state().await));
        let revoked_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], revoked_did.clone()).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }

        let Json(after) = unlink(State(state.clone())).await.unwrap();

        let device_did = after.device_did;
        assert_ne!(device_did, revoked_did.to_string());
        assert_eq!(
            state.read().await.profile.did().to_string(),
            device_did,
            "the reported device has to be the one the worker actually signs as"
        );
    }

    /// Sign-out replaces the operator too, not just the profile. An
    /// operator still delegated from the retired profile would sign
    /// presigns that prove nothing about the key this device now has.
    #[dialog_common::test]
    async fn it_re_keys_the_signing_session_on_sign_out() {
        let state = Arc::new(RwLock::new(test_state().await));
        let before = state.read().await.operator.did();

        let _ = unlink(State(state.clone())).await.unwrap();

        let tonk = state.read().await;
        assert_ne!(before, tonk.operator.did());
        assert_eq!(
            tonk.operator.profile_did(),
            tonk.profile.did(),
            "the session has to descend from the profile this device now signs as"
        );
    }

    #[dialog_common::test]
    async fn it_relinks_the_same_root_after_a_sign_out() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request = request_for(&[7u8; 32], device_did).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &request).await.unwrap();
        }
        let _ = unlink(State(state.clone())).await.unwrap();

        // The same account root, but delegated to the replacement key —
        // the old delegation names a device this browser no longer is.
        let rotated_did = state.read().await.profile.did();
        let relink = request_for(&[7u8; 32], rotated_did).await;
        {
            let tonk = state.read().await;
            persist_link(&tonk, &relink).await.unwrap();
        }

        let Json(loaded) = get(State(state)).await.unwrap();
        assert!(matches!(loaded, AccountStatus::Linked { .. }));
    }
}
