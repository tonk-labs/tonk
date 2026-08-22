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
use dialog_operator::{DeriveOperator, Operator, Profile};
use dialog_storage::provider::space::SpaceProvider;
use dialog_storage::provider::storage::Storage;
use dialog_ucan::Ucan;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::time::timestamp::{Duration, SystemTime};
use serde::{Deserialize, Serialize};
use tonk_common::log;

use crate::TonkWorkerError;
use crate::worker::DefaultSpace;

/// Credential site on the profile holding the current session's
/// derivation context and expiry, as JSON. Device-local — credential
/// sites never ride a branch — so persisting a session shares nothing.
const SESSION_SITE: &str = "tonk-session-v1";

/// The persisted shape of a session: enough to re-derive the operator
/// (derivation is a deterministic KDF over the profile seed and the
/// context) and to know when reuse must stop.
#[derive(Serialize, Deserialize)]
struct PersistedSession {
    version: u8,
    context: Vec<u8>,
    expires_at: u64,
}

/// How long a session delegation is good for.
///
/// Hours rather than minutes: a session has to survive a stretch offline
/// and a closed laptop, or renewal failure becomes the common path
/// instead of the exceptional one.
pub use tonk_identity::session::SESSION_TTL_SECONDS;

/// Derivation context for the device's operator key.
///
/// Constant, so the operator DID is stable for the life of the profile:
/// `derive` is a KDF over (profile seed, context), and renewal re-mints
/// the delegation rather than the key. Anything addressed to the
/// operator — notably a guest's invite chain — therefore stays valid
/// across a renewal.
const OPERATOR_CONTEXT: &[u8] = b"worker";

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
    S: Provider<Prove<Ucan>> + Provider<Retain<Ucan>>,
{
    // Reuse the persisted session while it is still fresh: derivation
    // is deterministic over (seed, context), so the operator
    // reconstitutes without minting, and the delegation saved when the
    // session was first opened still proves. A reused session makes
    // boot READ-ONLY on the access branch — no commit, no
    // authorization walk — so a worker restart cannot churn the
    // shared account root and a partial access branch cannot brick a
    // boot (offline included). Minting resumes only near expiry, on
    // the renewal beat that already owns it.
    let now = Timestamp::now().to_unix();
    if let Some(persisted) = load_persisted(profile, storage).await
        && !needs_renewal(persisted.expires_at, now)
    {
        match profile
            .derive(persisted.context.clone())
            .build(storage.clone())
            .await
        {
            Ok(operator) => {
                return Ok(Session {
                    operator,
                    expires_at: persisted.expires_at,
                });
            }
            Err(error) => {
                log!("persisted session unusable, minting a fresh one: {error}");
            }
        }
    }

    rotate(profile, storage).await
}

/// Mint a fresh `profile → operator` delegation for the device's
/// operator key.
///
/// The KEY is stable — [`OPERATOR_CONTEXT`] is constant, and derivation
/// is deterministic over (seed, context) — so renewal replaces the
/// delegation, not the audience.
///
/// It used to derive a new key from a random context every time. That
/// bought nothing the expiry did not already buy: revocation withholds
/// authority by refusing to renew the DELEGATION, and a chain is revoked
/// by CID, so nothing needs the audience to move. What it cost was
/// substantial — a guest's chain is addressed to the operator, so every
/// rotation invalidated it, and the only way to re-mint one was to
/// replay the invite. That is the sole reason a guest's invite URL, a
/// bearer secret, had to be kept on disk at all, and why a guest nearing
/// expiry could force a rotation that was not otherwise due.
pub async fn rotate<S>(
    profile: &Profile,
    storage: &Storage<S>,
) -> Result<Session<S>, TonkWorkerError>
where
    S: SpaceProvider + Clone + 'static,
    S: Provider<dialog_effects::blob::Read>
        + Provider<dialog_effects::blob::Write>
        + Provider<dialog_effects::blob::Import>,
    S: Provider<Prove<Ucan>> + Provider<Retain<Ucan>>,
{
    // No `.allow(...)`: that mints an *unexpiring* profile → operator
    // delegation, which is the thing this module exists to replace. The
    // bounded equivalent is minted below.
    let operator = profile
        .derive(OPERATOR_CONTEXT.to_vec())
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

    // Persist AFTER the delegation is durably saved, so a stored
    // context always has a provable delegation behind it. Best-effort:
    // a failed persist only costs the next boot a fresh mint.
    persist_session(
        profile,
        storage,
        &PersistedSession {
            version: 1,
            context: OPERATOR_CONTEXT.to_vec(),
            expires_at: expiration.to_unix(),
        },
    )
    .await;

    Ok(Session {
        operator,
        expires_at: expiration.to_unix(),
    })
}

/// Read the persisted session, if any. Absence and decode failure both
/// read as "no persisted session".
async fn load_persisted<S>(profile: &Profile, storage: &Storage<S>) -> Option<PersistedSession>
where
    S: SpaceProvider + Clone + 'static,
{
    let bytes = match profile
        .credential()
        .site(SESSION_SITE)
        .load::<Vec<u8>>()
        .perform(storage)
        .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            if !crate::credential::is_missing(&error) {
                log!("persisted session unreadable: {error}");
            }
            return None;
        }
    };
    match serde_json::from_slice::<PersistedSession>(&bytes) {
        Ok(persisted) if persisted.version == 1 => Some(persisted),
        Ok(_) => None,
        Err(error) => {
            log!("persisted session malformed: {error}");
            None
        }
    }
}

/// Store the session for reuse by later boots. Best-effort.
async fn persist_session<S>(profile: &Profile, storage: &Storage<S>, session: &PersistedSession)
where
    S: SpaceProvider + Clone + 'static,
{
    let Ok(encoded) = serde_json::to_vec(session) else {
        return;
    };
    if let Err(error) = profile
        .credential()
        .site(SESSION_SITE)
        .save(encoded)
        .perform(storage)
        .await
    {
        log!("failed to persist the session (next boot mints fresh): {error}");
    }
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
    async fn it_reuses_a_fresh_session_across_opens() {
        let (storage, profile) = scratch().await;

        let first = open(&profile, &storage).await.unwrap();
        let second = open(&profile, &storage).await.unwrap();

        assert_eq!(
            first.operator.did(),
            second.operator.did(),
            "a still-fresh session reconstitutes: reuse is what keeps a \
             boot read-only on the access branch"
        );
        assert_eq!(first.expires_at, second.expires_at);
    }

    #[dialog_common::test]
    async fn it_keys_an_expiring_session_separately() {
        let (storage, profile) = scratch().await;

        let first = open(&profile, &storage).await.unwrap();
        // Force the persisted session into the renewal window: an
        // expiring session must rotate to a NEW audience, or its
        // delegation lands in the same bucket as the lapsed one.
        persist_session(
            &profile,
            &storage,
            &PersistedSession {
                version: 1,
                context: vec![1; 16],
                expires_at: now(), // inside the renewal margin
            },
        )
        .await;
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
