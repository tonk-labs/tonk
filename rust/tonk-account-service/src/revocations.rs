//! Provider-independent immutable revocation publication.
//!
//! Writers receive verified artifacts rather than arbitrary keys. The key is
//! always derived from the signed target and artifact CIDs, so callers cannot
//! overwrite unrelated objects or address the bucket directly.

use tonk_identity::revocation::{VerifiedRevocation, VerifyError};

/// Prefix containing every published revocation artifact.
pub const REVOCATION_PREFIX: &str = "revocations/";

/// Whether a publication created a new immutable object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    /// The object was absent and has been stored.
    Stored,
    /// The identical content-addressed object was already present.
    Existing,
}

/// Errors surfaced by a revocation store.
#[derive(Debug, thiserror::Error)]
pub enum RevocationStoreError {
    /// The durable backend failed.
    #[error("revocation store failed: {0}")]
    Internal(String),
}

/// Immutable writer for verified revocation artifacts.
#[allow(async_fn_in_trait)]
pub trait RevocationStore {
    /// Store `bytes` at the key derived from `verified`.
    async fn put(
        &self,
        verified: &VerifiedRevocation,
        bytes: &[u8],
    ) -> Result<PutOutcome, RevocationStoreError>;
}

/// Successful result of verifying and publishing an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOutcome {
    /// Facts derived by the shared verifier.
    pub verified: VerifiedRevocation,
    /// Whether this call created the object.
    pub stored: bool,
}

/// Why publication failed.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The artifact is malformed or unauthorized.
    #[error(transparent)]
    Verification(#[from] VerifyError),
    /// Durable storage failed.
    #[error(transparent)]
    Store(#[from] RevocationStoreError),
}

/// Derive the only permitted object key for a verified artifact.
pub fn object_key(verified: &VerifiedRevocation) -> String {
    format!(
        "{REVOCATION_PREFIX}{}/{}",
        verified.target_cid, verified.artifact_cid
    )
}

/// Verify and durably publish an immutable revocation artifact.
pub async fn publish<R: RevocationStore>(
    store: &R,
    bytes: &[u8],
) -> Result<PublishOutcome, PublishError> {
    let verified = tonk_identity::revocation::verify(bytes).await?;
    let stored = store.put(&verified, bytes).await? == PutOutcome::Stored;
    Ok(PublishOutcome { verified, stored })
}

#[cfg(any(test, feature = "helpers"))]
mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// In-memory immutable revocation store for tests and local development.
    #[derive(Default)]
    pub struct MemoryRevocationStore(Mutex<HashMap<String, Vec<u8>>>);

    #[cfg(test)]
    impl MemoryRevocationStore {
        pub(super) fn len(&self) -> usize {
            self.0.lock().unwrap().len()
        }

        pub(super) fn keys(&self) -> Vec<String> {
            self.0.lock().unwrap().keys().cloned().collect()
        }

        pub(super) fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(key).cloned()
        }
    }

    impl RevocationStore for MemoryRevocationStore {
        async fn put(
            &self,
            verified: &VerifiedRevocation,
            bytes: &[u8],
        ) -> Result<PutOutcome, RevocationStoreError> {
            let mut objects = self.0.lock().map_err(|_| {
                RevocationStoreError::Internal("revocation store lock poisoned".to_string())
            })?;
            let key = object_key(verified);
            if objects.contains_key(&key) {
                return Ok(PutOutcome::Existing);
            }
            objects.insert(key, bytes.to_vec());
            Ok(PutOutcome::Stored)
        }
    }
}

#[cfg(any(test, feature = "helpers"))]
pub use memory::MemoryRevocationStore;

#[cfg(target_arch = "wasm32")]
pub mod r2;

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;

    async fn artifact() -> Vec<u8> {
        let root = Ed25519Signer::import(&[1u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[2u8; 32]).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        tonk_identity::revocation::mint_root_revocation(root, &grant, &grant.proof_cids()[0])
            .await
            .unwrap()
    }

    #[dialog_common::test]
    async fn it_keys_an_artifact_by_target_and_content_cids() {
        let store = MemoryRevocationStore::default();
        let bytes = artifact().await;
        let outcome = publish(&store, &bytes).await.unwrap();
        let expected = format!(
            "revocations/{}/{}",
            outcome.verified.target_cid, outcome.verified.artifact_cid
        );
        assert_eq!(store.keys(), vec![expected.clone()]);
        assert_eq!(store.get(&expected), Some(bytes));
    }

    #[dialog_common::test]
    async fn it_republishes_identical_bytes_idempotently() {
        let store = MemoryRevocationStore::default();
        let bytes = artifact().await;

        assert!(publish(&store, &bytes).await.unwrap().stored);
        assert!(!publish(&store, &bytes).await.unwrap().stored);
        assert_eq!(store.len(), 1);
    }

    #[dialog_common::test]
    async fn it_keeps_distinct_valid_artifacts_for_the_same_target() {
        let store = MemoryRevocationStore::default();
        let root = Ed25519Signer::import(&[1u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[2u8; 32]).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let target = grant.proof_cids()[0];
        let first = tonk_identity::revocation::mint_root_revocation(root.clone(), &grant, &target)
            .await
            .unwrap();
        let second = tonk_identity::revocation::mint_root_revocation(root, &grant, &target)
            .await
            .unwrap();
        let first_outcome = publish(&store, &first).await.unwrap();
        let second_outcome = publish(&store, &second).await.unwrap();

        assert_eq!(
            first_outcome.verified.target_cid,
            second_outcome.verified.target_cid
        );
        assert_ne!(
            first_outcome.verified.artifact_cid,
            second_outcome.verified.artifact_cid
        );
        assert_eq!(store.len(), 2);
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_bytes_before_storage() {
        let store = MemoryRevocationStore::default();

        assert!(matches!(
            publish(&store, b"invalid").await,
            Err(PublishError::Verification(_))
        ));
        assert_eq!(store.len(), 0);
    }

    /// The writer API has no delete method and accepts no caller-supplied key.
    /// Its only mutation takes a `VerifiedRevocation`, and each implementation
    /// derives the object key with `object_key`.
    #[dialog_common::test]
    fn it_exposes_no_delete_or_arbitrary_key_operation() {
        fn accepts_store<T: RevocationStore>() {}
        accepts_store::<MemoryRevocationStore>();
    }
}
