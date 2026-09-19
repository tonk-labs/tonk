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
//! Each boot and renewal creates a disposable operator and a matching
//! bounded grant held only in memory. Membership and invitation authority
//! belong to accounts and profiles, so replacing this final hop needs no
//! invitation replay or durable session write.
//!
//! Renewal rides the sync drain — the regular beat this worker has —
//! rather than chasing every presign path. The gap that leaves: a
//! worker alive past the TTL whose next presign is not preceded by a
//! drain presents a lapsed chain and takes one 401, which the next
//! drain's rotation heals. Service-worker lifetimes make that window
//! rare; revisit only if it is ever observed.

use dialog_capability::{Provider, Subject};
use dialog_operator::{DeriveOperator, Operator, Profile};
use dialog_storage::provider::space::SpaceProvider;
use dialog_storage::provider::storage::Storage;
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
    S: Provider<dialog_effects::blob::Read>
        + Provider<dialog_effects::blob::Write>
        + Provider<dialog_effects::blob::Import>,
{
    rotate(profile, storage).await
}

/// Create a fresh operator and bounded in-memory profile grant.
/// Existing session credentials and delegations are left untouched.
pub async fn rotate<S>(
    profile: &Profile,
    storage: &Storage<S>,
) -> Result<Session<S>, TonkWorkerError>
where
    S: SpaceProvider + Clone + 'static,
    S: Provider<dialog_effects::blob::Read>
        + Provider<dialog_effects::blob::Write>
        + Provider<dialog_effects::blob::Import>,
{
    let mut context = [0u8; 32];
    getrandom::fill(&mut context).map_err(|error| {
        TonkWorkerError::Internal(format!("failed to generate session entropy: {error}"))
    })?;
    let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(SESSION_TTL_SECONDS))
        .map_err(|error| {
        TonkWorkerError::Internal(format!("session expiration out of range: {error}"))
    })?;
    let operator = profile
        .derive(context)
        .allow_until(Subject::any(), expiration)
        .build(storage.clone())
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to build a session operator: {error}"))
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

    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use dialog_ucan::UcanDelegation;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::Principal;

    /// A throwaway profile in a scratch directory, plus the storage it
    /// is mounted in. Names are unique per call so tests never share a
    /// profile key or a certificate store.
    ///
    /// The name must be unique across PROCESSES, not just within one: the
    /// runner starts a process per test, so a bare per-process counter
    /// hands two concurrent tests the same name — and therefore the same
    /// profile directory, whose writer lock one of them then loses.
    /// `unique_name` folds in the pid for exactly this reason.
    async fn scratch() -> (Storage<DefaultSpace>, Profile) {
        let name = dialog_operator::helpers::unique_name("session-test");
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
    async fn it_creates_distinct_sessions_across_opens() {
        let (storage, profile) = scratch().await;

        let first = open(&profile, &storage).await.unwrap();
        let second = open(&profile, &storage).await.unwrap();

        assert_ne!(first.operator.did(), second.operator.did());
        assert_eq!(first.operator.profile_did(), profile.did());
        assert_eq!(second.operator.profile_did(), profile.did());
    }

    async fn access_revision(
        profile: &Profile,
        operator: &Operator<DefaultSpace>,
    ) -> Option<dialog_repository::Revision> {
        dialog_repository::Repository::from(profile.signer().clone())
            .branch(dialog_repository::ACCESS_BRANCH)
            .open()
            .perform(operator)
            .await
            .unwrap()
            .revision()
    }

    async fn retain_space(
        profile: &Profile,
        operator: &Operator<DefaultSpace>,
    ) -> dialog_varsig::Did {
        let space = Ed25519Signer::generate().await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&profile.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        profile
            .access()
            .save(UcanDelegation(DelegationChain::new(grant)))
            .perform(operator)
            .await
            .unwrap();
        space.did()
    }

    async fn assert_proof(profile: &Profile, session: &Session, space: &dialog_varsig::Did) {
        let proof = profile
            .access()
            .prove(Subject::from(space.clone()).attenuate(dialog_effects::Use))
            .audience(&session.operator)
            .perform(&session.operator)
            .await
            .unwrap();
        assert_eq!(proof.proofs.len(), 2);
        assert_eq!(proof.proofs[0].0.audience(), proof.proofs[1].0.issuer());
        assert_eq!(proof.proofs[1].0.audience(), &session.operator.did());
        assert_eq!(proof.duration.expiration, Some(session.expires_at));
    }

    #[dialog_common::test]
    async fn it_authorizes_replacement_sessions_without_committing() {
        let (storage, profile) = scratch().await;
        let setup = open(&profile, &storage).await.unwrap();
        let space = retain_space(&profile, &setup.operator).await;
        let revision = access_revision(&profile, &setup.operator).await;
        let first = open(&profile, &storage).await.unwrap();
        assert_eq!(access_revision(&profile, &first.operator).await, revision);
        let second = open(&profile, &storage).await.unwrap();
        assert_eq!(access_revision(&profile, &second.operator).await, revision);
        assert_ne!(first.operator.did(), second.operator.did());
        assert_eq!(second.operator.profile_did(), profile.did());
        for session in [&first, &second] {
            assert_proof(&profile, session, &space).await;
            assert_proof(&profile, session, &space).await;
        }
    }

    #[dialog_common::test]
    async fn it_ignores_legacy_sessions_and_reopens_durable_storage() {
        let name = dialog_operator::helpers::unique_name("session-reopen");
        let (profile_did, old_operator, space, revision, legacy) = {
            let storage = Storage::<DefaultSpace>::default();
            let profile = Profile::open(&name)
                .at(Directory::Temp)
                .perform(&storage)
                .await
                .unwrap();
            // Simulate Safari's saved grant naming an audience unrelated
            // to the operator reconstructed from the legacy context.
            let old = profile
                .derive(b"legacy-other-operator")
                .build(storage.clone())
                .await
                .unwrap();
            let expiration =
                Timestamp::new(SystemTime::now() + Duration::from_secs(SESSION_TTL_SECONDS))
                    .unwrap();
            let grant = profile
                .access()
                .claim(Subject::any())
                .expires(expiration)
                .delegate(old.did())
                .perform(&old)
                .await
                .unwrap();
            profile.access().save(grant).perform(&old).await.unwrap();
            let space = retain_space(&profile, &old).await;
            let legacy = serde_json::to_vec(&serde_json::json!({
                "version": 1, "context": b"worker".to_vec(), "expires_at": expiration.to_unix()
            }))
            .unwrap();
            profile
                .credential()
                .site("tonk-session-v1")
                .save(legacy.clone())
                .perform(&storage)
                .await
                .unwrap();
            (
                profile.did(),
                old.did(),
                space,
                access_revision(&profile, &old).await,
                legacy,
            )
        };
        // All prior operators, profiles, branches and the storage pool have
        // been released. Reopen the same durable profile with a new pool.
        let storage = Storage::<DefaultSpace>::default();
        let profile = Profile::open(&name)
            .at(Directory::Temp)
            .perform(&storage)
            .await
            .unwrap();
        let session = open(&profile, &storage).await.unwrap();
        assert_eq!(profile.did(), profile_did);
        assert_ne!(session.operator.did(), old_operator);
        assert_eq!(access_revision(&profile, &session.operator).await, revision);
        assert_proof(&profile, &session, &space).await;
        let after = profile
            .credential()
            .site("tonk-session-v1")
            .load::<Vec<u8>>()
            .perform(&storage)
            .await
            .unwrap();
        assert_eq!(
            after, legacy,
            "legacy session metadata is neither consulted nor replaced"
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
