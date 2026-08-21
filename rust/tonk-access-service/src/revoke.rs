//! The `/ucan/revoke` command.
//!
//! Arrives at the one `/ucan/` endpoint like everything else, and is
//! answered before the presign path: it writes to the revocation index
//! rather than reading from it, so it is not a storage authorization.
//!
//! Three questions, in order. Have we recorded this already, in which
//! case the answer is yes and nothing else needs asking. Is the subject
//! one whose data we hold, since a revocation about a space we serve
//! nothing for guards nothing. And does the evidence prove the subject
//! could revoke the target, which
//! [`tonk_identity::revocation::verify`] answers.

use tonk_account::customer::{RegistrationError, RevokeReceipt};
use tonk_identity::revocation::{VerifyError, verify};

use crate::revocation::index::RevocationIndex;
use crate::store::{ConsumerDeletionState, Store, StoreError};

/// The command path, as it appears in an invocation.
pub const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// Whether a container carries a revocation, without verifying it.
///
/// `None` on any parse failure or another command, so the caller falls
/// through to the paths that own those.
pub fn is_revocation(container_bytes: &[u8]) -> bool {
    use dialog_ucan_core::{Container, Invocation};
    use dialog_varsig::AnySignature;

    let Ok(container) = Container::from_bytes(container_bytes) else {
        return false;
    };
    let tokens = container.into_tokens();
    let Some(first) = tokens.first() else {
        return false;
    };
    let Ok(invocation) = serde_ipld_dagcbor::from_slice::<Invocation<AnySignature>>(first) else {
        return false;
    };
    let segments: Vec<&str> = invocation.command().0.iter().map(String::as_str).collect();
    segments.as_slice() == REVOKE_COMMAND
}

fn internal(error: StoreError) -> RegistrationError {
    RegistrationError::Internal {
        message: error.to_string(),
    }
}

/// Verify a revocation and record it.
///
/// The consumer-row check is deliberately not the provisioning gate.
/// That gate asks who is paying; the question here is whether we hold
/// anything this revocation could protect. So a customer who is merely
/// unactivated, suspended, or over limit may still revoke: revocation
/// answers a compromised key, which is exactly when a bill may also be
/// unpaid. A subject we never registered, or one whose data we already
/// purged, is refused — otherwise the index is an open write surface.
pub async fn revoke<S: Store, I: RevocationIndex>(
    store: &S,
    index: &I,
    container: &[u8],
) -> Result<RevokeReceipt, RegistrationError> {
    let verified = verify(container).await.map_err(|error| match error {
        VerifyError::Malformed(message) => RegistrationError::Invalid { message },
        VerifyError::Unauthorized(message) => RegistrationError::Unauthorized { message },
    })?;

    let subject = verified.subject.to_string();
    match store.consumer(&subject).await.map_err(internal)? {
        // Nothing here to protect, and an unbounded write surface if we
        // accepted it.
        None => return Err(RegistrationError::UnknownConsumer),
        Some(consumer) if consumer.deletion_state == ConsumerDeletionState::Deleted => {
            return Err(RegistrationError::UnknownConsumer);
        }
        Some(_) => {}
    }

    let recorded = index
        .record(&verified.target_cid, &subject)
        .await
        .map_err(|error| RegistrationError::Internal {
            message: error.to_string(),
        })?;

    Ok(RevokeReceipt {
        revoked: verified
            .target_cid
            .parse()
            .map_err(|error| RegistrationError::Internal {
                message: format!("verified target CID does not re-parse: {error}"),
            })?,
        subject: verified.subject,
        recorded,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;

    use std::collections::BTreeSet;

    use super::*;
    use crate::revocation::index::MemoryRevocationIndex;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{ConsumerKind, SIGNUP_PLAN};

    /// A space, a device it granted, and a revocation of that grant
    /// signed by the space itself.
    async fn revocation() -> (Ed25519Signer, Vec<u8>) {
        let space = Ed25519Signer::import(&[21u8; 32]).await.expect("space key");
        let device = Ed25519Signer::import(&[22u8; 32])
            .await
            .expect("device key");
        let grant = tonk_identity::delegation::mint_device_delegation(space.clone(), &device.did())
            .await
            .expect("grant");
        let target = grant.proof_cids()[0];
        let bytes = tonk_identity::revocation::mint_root_revocation(space.clone(), &grant, &target)
            .await
            .expect("revocation");
        (space, bytes)
    }

    /// A store holding `subject` as a consumer in `state`.
    async fn store_holding(subject: &str, state: ConsumerDeletionState) -> SqliteStore {
        let store = SqliteStore::in_memory().expect("in-memory store");
        store
            .enroll_customer(subject, "holder@example.com", b"", SIGNUP_PLAN, 0)
            .await
            .expect("customer");
        store
            .add_consumer(subject, subject, 0, ConsumerKind::Space)
            .await
            .expect("consumer");
        if state == ConsumerDeletionState::Deleted {
            store.mark_consumer_deleting(subject).await.expect("deny");
            store
                .finish_consumer_deletion(subject, 1)
                .await
                .expect("purge");
        }
        store
    }

    #[dialog_common::test]
    async fn it_records_a_revocation_for_a_subject_we_hold() {
        let (space, bytes) = revocation().await;
        let subject = space.did().to_string();
        let store = store_holding(&subject, ConsumerDeletionState::Active).await;
        let index = MemoryRevocationIndex::default();

        let receipt = revoke(&store, &index, &bytes).await.expect("recorded");
        assert_eq!(receipt.subject, space.did());
        assert!(receipt.recorded);

        // And it is the fact the presign screen will find.
        assert!(
            index
                .revoked_by_any(&receipt.revoked.to_string(), &BTreeSet::from([subject]))
                .await
                .unwrap()
        );
    }

    #[dialog_common::test]
    async fn it_answers_a_replay_without_recording_twice() {
        // Revocation is idempotent, so presenting the same artifact
        // again succeeds and says it changed nothing.
        let (space, bytes) = revocation().await;
        let store = store_holding(space.did().as_ref(), ConsumerDeletionState::Active).await;
        let index = MemoryRevocationIndex::default();

        assert!(revoke(&store, &index, &bytes).await.unwrap().recorded);
        assert!(!revoke(&store, &index, &bytes).await.unwrap().recorded);
    }

    #[dialog_common::test]
    async fn it_refuses_a_subject_we_never_registered() {
        // Nothing here to protect, and accepting would leave the index
        // open to anyone with a keypair.
        let (_, bytes) = revocation().await;
        let store = SqliteStore::in_memory().expect("in-memory store");
        let index = MemoryRevocationIndex::default();

        assert!(matches!(
            revoke(&store, &index, &bytes).await,
            Err(RegistrationError::UnknownConsumer)
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_a_subject_whose_data_we_purged() {
        let (space, bytes) = revocation().await;
        let store = store_holding(space.did().as_ref(), ConsumerDeletionState::Deleted).await;
        let index = MemoryRevocationIndex::default();

        assert!(matches!(
            revoke(&store, &index, &bytes).await,
            Err(RegistrationError::UnknownConsumer)
        ));
    }

    #[dialog_common::test]
    async fn it_accepts_a_revocation_from_a_customer_who_never_activated() {
        // The consumer row is the question, not the billing gate: a
        // compromised key is exactly when a bill may also be unpaid.
        let (space, bytes) = revocation().await;
        let subject = space.did().to_string();
        let store = SqliteStore::in_memory().expect("in-memory store");
        store
            .enroll_customer(&subject, "pending@example.com", b"", SIGNUP_PLAN, 0)
            .await
            .expect("customer stays Registered");
        store
            .add_consumer(&subject, &subject, 0, ConsumerKind::Space)
            .await
            .expect("consumer");
        let index = MemoryRevocationIndex::default();

        assert!(revoke(&store, &index, &bytes).await.unwrap().recorded);
    }

    #[dialog_common::test]
    async fn it_refuses_a_container_that_is_not_a_revocation() {
        let (space, _) = revocation().await;
        let store = store_holding(space.did().as_ref(), ConsumerDeletionState::Active).await;
        let index = MemoryRevocationIndex::default();

        assert!(revoke(&store, &index, b"not a container").await.is_err());
    }

    #[dialog_common::test]
    async fn it_recognizes_a_revocation_container() {
        let (_, bytes) = revocation().await;
        assert!(is_revocation(&bytes));
        assert!(!is_revocation(b"not a container"));
    }
}
