//! The worker's signing session.
//!
//! Every presign invocation is signed by the *operator* key, and its
//! proofs are assembled by a certificate-store walk that starts at the
//! operator and ends at the subject. The `profile → operator` delegation
//! is therefore the last hop of every chain this worker presents, which
//! makes it the one place a time bound reaches all of them.
//!
//! Bounding it is what gives revocation something to withhold. The
//! `root → device` grant is unexpiring, so withdrawing it can only ever
//! be a registry lookup; a session that lapses on its own costs a
//! stolen device at most one session lifetime even if the registry is
//! unreachable.
//!
//! Renewal rotates the operator key rather than re-minting under the
//! same audience. Certificates are content-addressed, the store has no
//! delete, and its chain walk never consults the clock — it filters
//! candidates against the *requested* time range, which the presign path
//! leaves unbounded, and an unbounded requirement is satisfied by every
//! range including a lapsed one. So a stale certificate left beside its
//! replacement under one audience would be chosen about half the time.
//! A rotated key gets its own audience, and the retired one is simply
//! never proved to again.
//!
//! Renewal rides the sync drain — the regular beat this worker has —
//! rather than chasing every presign path. The gap that leaves: a
//! worker alive past the TTL whose next presign is not preceded by a
//! drain presents a lapsed chain and takes one 401, which the next
//! drain's rotation heals. Service-worker lifetimes make that window
//! rare; revisit only if it is ever observed.

use dialog_capability::access::{Prove, Retain};
use dialog_capability::{Provider, Subject};
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::space::SpaceProvider;
use dialog_storage::provider::storage::Storage;
use dialog_ucan::Ucan;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime};

use crate::TonkWorkerError;
use crate::worker::DefaultSpace;

/// How long a session delegation is good for.
///
/// Hours rather than minutes: a session has to survive a stretch offline
/// and a closed laptop, or renewal failure becomes the common path
/// instead of the exceptional one.
pub use tonk_identity::session::SESSION_TTL_SECONDS;

/// How long before expiry a session is rotated.
///
/// Wide enough that rotation lands during ordinary sync activity rather
/// than at the cliff: renewal is local (a key derivation and a
/// self-signed delegation, no network), but it only runs when something
/// drives the worker, and a quiet page can go a while between drains.
pub const RENEWAL_MARGIN_SECONDS: u64 = 60 * 60;

/// A signing session: the operator that signs presigns, and the moment
/// the delegation authorizing it stops being valid.
pub struct Session<S: Clone = DefaultSpace> {
    /// The operator, keyed for this session alone.
    pub operator: Operator<S>,
    /// Expiry of the `profile → operator` delegation, unix seconds.
    pub expires_at: u64,
}

/// Open a fresh signing session for `profile` over `storage`.
///
/// `storage` is cloned rather than created, so the session's operator
/// mounts into the same pool as every handle already open against it.
/// A session built over its own pool would leave the reactor's cached
/// repositories talking to the previous one.
pub async fn open<S>(profile: &Profile, storage: &Storage<S>) -> Result<Session<S>, TonkWorkerError>
where
    S: SpaceProvider + Clone + 'static,
    S: Provider<Prove<Ucan>> + Provider<Retain<Ucan>>,
{
    // A random derivation context is what makes the operator key
    // session-scoped: `derive` is a KDF over the profile seed and this
    // context, so a fixed context would hand every session the same
    // audience and file a rotated delegation in the same bucket as the
    // one it replaces.
    let context: [u8; 16] = rand::random();

    // No `.allow(...)`: that mints an *unexpiring* profile → operator
    // delegation, which is the thing this module exists to replace. The
    // bounded equivalent is minted below.
    let operator = profile
        .derive(context.to_vec())
        .build(storage.clone())
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to derive a session operator: {error}"))
        })?;

    let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(SESSION_TTL_SECONDS))
        .map_err(|error| {
        TonkWorkerError::Internal(format!("session expiration out of range: {error}"))
    })?;

    let delegation = profile
        .access()
        .claim(Subject::any())
        .expires(expiration)
        .delegate(operator.did())
        .perform(&operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to mint the session delegation: {error}"))
        })?;

    profile
        .access()
        .save(delegation)
        .perform(&operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to save the session delegation: {error}"))
        })?;

    Ok(Session {
        operator,
        expires_at: expiration.to_unix(),
    })
}

/// Whether a session expiring at `expires_at` is close enough to lapsing
/// to be rotated now.
pub fn needs_renewal(expires_at: u64, now: u64) -> bool {
    now.saturating_add(RENEWAL_MARGIN_SECONDS) >= expires_at
}

/// The current wall clock in unix seconds, as [`needs_renewal`] expects.
pub fn now() -> u64 {
    Timestamp::now().to_unix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    use std::sync::atomic::{AtomicU64, Ordering};

    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use dialog_ucan::UcanDelegation;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal;

    /// A throwaway profile in a scratch directory, plus the storage it
    /// is mounted in. Names are unique per call so tests never share a
    /// profile key or a certificate store.
    async fn scratch() -> (Storage<DefaultSpace>, Profile) {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let name = format!("session-test-{}", SEQ.fetch_add(1, Ordering::Relaxed));
        let storage = Storage::<DefaultSpace>::default();
        let profile = Profile::open(name)
            .at(Directory::Temp)
            .perform(&storage)
            .await
            .expect("profile opens");
        (storage, profile)
    }

    #[dialog_common::test]
    async fn it_bounds_the_session_within_the_ttl() {
        let (storage, profile) = scratch().await;
        let before = now();

        let session = open(&profile, &storage).await.unwrap();

        assert!(session.expires_at >= before + SESSION_TTL_SECONDS);
        assert!(session.expires_at <= now() + SESSION_TTL_SECONDS);
    }

    #[dialog_common::test]
    async fn it_keys_every_session_separately() {
        let (storage, profile) = scratch().await;

        let first = open(&profile, &storage).await.unwrap();
        let second = open(&profile, &storage).await.unwrap();

        assert_ne!(
            first.operator.did(),
            second.operator.did(),
            "a rotated session needs its own audience, or its delegation \
             lands in the same bucket as the lapsed one it replaces"
        );
    }

    /// The one that matters: swapping `.allow(Subject::any())` for a
    /// bounded claim must still authorize a presign. This walks the same
    /// BFS the presign path does — operator toward a space subject,
    /// composing the session hop with a `space -> profile` grant — and
    /// checks both that it resolves and that the session's expiry is
    /// what bounds the result.
    #[dialog_common::test]
    async fn it_authorizes_a_presign_chain_bounded_by_the_session() {
        let (storage, profile) = scratch().await;
        let session = open(&profile, &storage).await.unwrap();

        let space = Ed25519Signer::generate().await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(space.clone())
            .audience(&profile.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        profile
            .access()
            .save(UcanDelegation(DelegationChain::new(grant)))
            .perform(&session.operator)
            .await
            .unwrap();

        let proof = profile
            .access()
            .prove(Subject::from(space.did()))
            .audience(&session.operator)
            .perform(&session.operator)
            .await
            .expect("the session operator must still reach the space");

        assert_eq!(
            proof.proofs.len(),
            2,
            "the chain is space -> profile -> operator"
        );
        assert_eq!(
            proof.duration.expiration,
            Some(session.expires_at),
            "an unexpiring space grant composed with a bounded session \
             must come out bounded by the session"
        );
    }

    #[dialog_common::test]
    async fn it_holds_a_session_open_well_before_expiry() {
        let expires_at = 1_000_000;

        assert!(!needs_renewal(
            expires_at,
            expires_at - RENEWAL_MARGIN_SECONDS - 1
        ));
    }

    #[dialog_common::test]
    async fn it_renews_once_inside_the_margin() {
        let expires_at = 1_000_000;

        assert!(needs_renewal(
            expires_at,
            expires_at - RENEWAL_MARGIN_SECONDS
        ));
    }

    #[dialog_common::test]
    async fn it_renews_a_session_that_already_lapsed() {
        let expires_at = 1_000_000;

        assert!(
            needs_renewal(expires_at, expires_at + 1),
            "a lapsed session must rotate rather than keep presenting a dead delegation"
        );
    }

    #[dialog_common::test]
    async fn it_does_not_overflow_renewing_far_from_the_epoch() {
        assert!(
            needs_renewal(0, u64::MAX),
            "the margin must saturate rather than wrap a clock near u64::MAX"
        );
    }
}
