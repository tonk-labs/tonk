//! Audience-keyed publication of enrollment chains.
//!
//! A device that has just derived a credential key from a new passkey holds
//! nothing else — not the account subject, not a delegation, and not the
//! account repository it would eventually read them from, because the
//! authority to sync that repository is the thing it is looking for. The one
//! question it can ask is "what has been addressed to this key?".
//!
//! So entries are keyed by audience DID. Each is an ordinary signed
//! delegation chain: self-authenticating, and useless to anyone without the
//! audience's private key. That is why claiming needs no authorization and
//! why this store confers nothing by holding an entry. It can withhold one;
//! it cannot forge or alter one.

use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use tonk_identity::delegation::{GrantError, validate_account_grant};

use crate::core::backup::chain_key;
use crate::revocations::PutOutcome;

/// Prefix containing every published enrollment chain.
pub const ENROLLMENT_PREFIX: &str = "enrollments/";

/// Most entries returned for one credential.
///
/// A credential collects one chain per anchor path it was enrolled through,
/// so the real number is a handful. The bound is here so that a hostile
/// account cannot turn one claim into an unbounded read.
pub const MAX_CLAIMED: usize = 64;

/// Facts derived from a verified enrollment chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEnrollment {
    /// Account subject the chain runs from.
    pub account_root: Did,
    /// Credential the chain is addressed to, and the key it is stored under.
    pub credential: Did,
    /// Content-addressed key for these exact bytes.
    pub key: String,
}

/// Errors surfaced by an enrollment store.
#[derive(Debug, thiserror::Error)]
pub enum EnrollmentStoreError {
    /// The durable backend failed.
    #[error("enrollment store failed: {0}")]
    Internal(String),
}

/// Why an enrollment chain could not be accepted.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The bytes are malformed or not shaped like account authority.
    #[error("invalid enrollment chain: {0}")]
    Invalid(String),
    /// The chain is well-formed but a hop's signature does not verify.
    #[error("unauthorized enrollment chain: {0}")]
    Unauthorized(String),
}

/// Immutable writer and audience-keyed reader for enrollment chains.
#[allow(async_fn_in_trait)]
pub trait EnrollmentStore {
    /// Store `bytes` at the key derived from `verified`.
    async fn put(
        &self,
        verified: &VerifiedEnrollment,
        bytes: &[u8],
    ) -> Result<PutOutcome, EnrollmentStoreError>;

    /// Every chain addressed to `credential`, at most [`MAX_CLAIMED`].
    async fn claim(&self, credential: &Did) -> Result<Vec<Vec<u8>>, EnrollmentStoreError>;
}

/// Derive the only permitted object key for a verified chain.
pub fn object_key(verified: &VerifiedEnrollment) -> String {
    format!(
        "{ENROLLMENT_PREFIX}{}/{}",
        verified.credential, verified.key
    )
}

/// Parse and check an enrollment chain.
///
/// The chain must run from an account subject to the credential it is
/// addressed to, with every hop subject-open, command-open and signed. That
/// is the same walk a device's grant gets, so it is the same function; the
/// audience checked against is the chain's own, which is what makes this a
/// statement about where the chain ends rather than about who is asking.
pub async fn verify(bytes: &[u8]) -> Result<VerifiedEnrollment, VerifyError> {
    let chain = DelegationChain::try_from(bytes)
        .map_err(|error| VerifyError::Invalid(format!("bad delegation container: {error}")))?;
    let canonical = chain
        .to_bytes()
        .map_err(|error| VerifyError::Invalid(format!("chain does not re-encode: {error}")))?;
    if canonical != bytes {
        return Err(VerifyError::Invalid(
            "delegation container is not canonical".to_string(),
        ));
    }

    let credential = chain.audience().clone();
    let grant = validate_account_grant(&chain, &credential)
        .await
        .map_err(|error| match error {
            GrantError::Signature(message) => VerifyError::Unauthorized(message),
            other => VerifyError::Invalid(other.to_string()),
        })?;

    Ok(VerifiedEnrollment {
        account_root: grant.root_did,
        credential,
        key: chain_key(bytes),
    })
}

#[cfg(any(test, feature = "helpers"))]
mod memory {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    /// In-memory enrollment store for tests and local development.
    #[derive(Default)]
    pub struct MemoryEnrollmentStore(Mutex<BTreeMap<String, Vec<u8>>>);

    impl EnrollmentStore for MemoryEnrollmentStore {
        async fn put(
            &self,
            verified: &VerifiedEnrollment,
            bytes: &[u8],
        ) -> Result<PutOutcome, EnrollmentStoreError> {
            let mut entries = self.0.lock().map_err(|_| {
                EnrollmentStoreError::Internal("enrollment store lock poisoned".to_string())
            })?;
            let key = object_key(verified);
            if entries.contains_key(&key) {
                return Ok(PutOutcome::Existing);
            }
            entries.insert(key, bytes.to_vec());
            Ok(PutOutcome::Stored)
        }

        async fn claim(&self, credential: &Did) -> Result<Vec<Vec<u8>>, EnrollmentStoreError> {
            let entries = self.0.lock().map_err(|_| {
                EnrollmentStoreError::Internal("enrollment store lock poisoned".to_string())
            })?;
            let prefix = format!("{ENROLLMENT_PREFIX}{credential}/");
            Ok(entries
                .range(prefix.clone()..)
                .take_while(|(key, _)| key.starts_with(&prefix))
                .take(MAX_CLAIMED)
                .map(|(_, bytes)| bytes.clone())
                .collect())
        }
    }
}

#[cfg(any(test, feature = "helpers"))]
pub use memory::MemoryEnrollmentStore;

#[cfg(target_arch = "wasm32")]
pub mod r2;

// Native only, for the reason given in `revocations.rs`: a Worker exports
// `fetch`, which the wasm-bindgen harness shadows when it loads the module.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use tonk_identity::credential::{extend_with_enrollment, mint_enrollment};
    use tonk_identity::delegation::mint_device_delegation;

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    /// `root → recovery → credential`, the shape the anchor path produces.
    async fn anchor_chain() -> (Did, Did, Vec<u8>) {
        let root = signer(1).await;
        let recovery = signer(2).await;
        let credential = signer(3).await;
        let anchor = mint_enrollment(root.clone(), &recovery.did())
            .await
            .unwrap();
        let chain = extend_with_enrollment(&anchor, recovery, &credential.did())
            .await
            .unwrap();
        (root.did(), credential.did(), chain.to_bytes().unwrap())
    }

    #[dialog_common::test]
    async fn it_reports_the_account_and_credential_a_chain_connects() {
        let (root, credential, bytes) = anchor_chain().await;

        let verified = verify(&bytes).await.unwrap();

        assert_eq!(verified.account_root, root);
        assert_eq!(verified.credential, credential);
        assert_eq!(
            object_key(&verified),
            format!("enrollments/{credential}/{}", verified.key)
        );
    }

    #[dialog_common::test]
    async fn it_returns_every_chain_addressed_to_one_credential() {
        let store = MemoryEnrollmentStore::default();
        let root = signer(1).await;
        let recovery = signer(2).await;
        let credential = signer(3).await;

        // The same credential enrolled twice: once straight from the account
        // subject, once through the recovery anchor.
        let sibling = mint_device_delegation(root.clone(), &credential.did())
            .await
            .unwrap()
            .to_bytes()
            .unwrap();
        let anchor = mint_enrollment(root.clone(), &recovery.did())
            .await
            .unwrap();
        let anchored = extend_with_enrollment(&anchor, recovery, &credential.did())
            .await
            .unwrap()
            .to_bytes()
            .unwrap();
        for bytes in [&sibling, &anchored] {
            let verified = verify(bytes).await.unwrap();
            store.put(&verified, bytes).await.unwrap();
        }

        let mut claimed = store.claim(&credential.did()).await.unwrap();
        claimed.sort();
        let mut expected = vec![sibling, anchored];
        expected.sort();
        assert_eq!(claimed, expected);
    }

    #[dialog_common::test]
    async fn it_claims_nothing_for_an_unrelated_credential() {
        let store = MemoryEnrollmentStore::default();
        let (_, _, bytes) = anchor_chain().await;
        let verified = verify(&bytes).await.unwrap();
        store.put(&verified, &bytes).await.unwrap();
        let stranger = signer(9).await;

        assert!(store.claim(&stranger.did()).await.unwrap().is_empty());
    }

    #[dialog_common::test]
    async fn it_stores_identical_bytes_once() {
        let store = MemoryEnrollmentStore::default();
        let (_, credential, bytes) = anchor_chain().await;
        let verified = verify(&bytes).await.unwrap();

        assert_eq!(
            store.put(&verified, &bytes).await.unwrap(),
            PutOutcome::Stored
        );
        assert_eq!(
            store.put(&verified, &bytes).await.unwrap(),
            PutOutcome::Existing
        );
        assert_eq!(store.claim(&credential).await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_with_a_broken_signature() {
        let (_, _, bytes) = anchor_chain().await;
        let mut tampered = bytes.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xff;

        assert!(verify(&tampered).await.is_err());
    }

    #[dialog_common::test]
    async fn it_rejects_bytes_that_are_not_a_delegation_container() {
        assert!(matches!(
            verify(b"not a container").await,
            Err(VerifyError::Invalid(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_subject_specific_chain() {
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject;

        let space = signer(4).await;
        let member = signer(5).await;
        let invite = DelegationBuilder::new()
            .issuer(space.clone())
            .audience(&member.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let bytes = DelegationChain::new(invite).to_bytes().unwrap();

        assert!(
            matches!(verify(&bytes).await, Err(VerifyError::Invalid(_))),
            "a space invite is not account authority and must not be claimable"
        );
    }
}
