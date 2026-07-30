//! Shared validation and set-if-absent policy for repository descriptors.

use crate::core::CeremonyError;
use crate::store::{Account, Store};

/// Validate signed descriptor hex and require its subject to equal `root_did`.
pub async fn validate_descriptor(
    descriptor_hex: &str,
    root_did: &str,
) -> Result<Vec<u8>, CeremonyError> {
    let bytes = hex::decode(descriptor_hex)
        .map_err(|_| CeremonyError::Invalid("repositoryDescriptor must be hex".to_string()))?;
    let descriptor = tonk_account::AccountRepositoryDescriptorV1::validate(&bytes)
        .await
        .map_err(|error| {
            CeremonyError::Invalid(format!("invalid repositoryDescriptor: {error}"))
        })?;
    if descriptor.account_subject().as_ref() != root_did {
        return Err(CeremonyError::Invalid(
            "repositoryDescriptor subject does not match the account root".to_string(),
        ));
    }
    Ok(descriptor.bytes().to_vec())
}

/// Set-if-absent outcome carrying the exact stored winner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstablishedDescriptor {
    /// Exact canonical bytes stored on the account.
    pub descriptor: Vec<u8>,
    /// Whether this request installed the winner.
    pub created: bool,
}

/// Validate and atomically establish one descriptor for an existing account.
pub async fn establish_descriptor<S: Store>(
    store: &S,
    account: &Account,
    descriptor_hex: &str,
) -> Result<EstablishedDescriptor, CeremonyError> {
    let candidate = validate_descriptor(descriptor_hex, &account.root_did).await?;
    let (descriptor, created) = store
        .establish_repository_descriptor(account.id, &candidate)
        .await?;
    Ok(EstablishedDescriptor {
        descriptor,
        created,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use dialog_varsig::Principal as _;

    use super::*;
    use crate::store::Store;
    use crate::store::sqlite::SqliteStore;

    #[dialog_common::test]
    async fn it_returns_one_descriptor_winner_to_concurrent_establishers() {
        let store = SqliteStore::in_memory().unwrap();
        let root = dialog_credentials::Ed25519Signer::import(&[7; 32])
            .await
            .unwrap();
        let root_did = root.did().to_string();
        store
            .create_account("a@x.com", &root_did, "cred", 1)
            .await
            .unwrap();
        let account = store.account_by_root(&root_did).await.unwrap().unwrap();
        let candidate_a =
            tonk_account::AccountRepositoryDescriptorV1::sign(&root, "https://a.example/ucan/")
                .await
                .unwrap();
        let candidate_b =
            tonk_account::AccountRepositoryDescriptorV1::sign(&root, "https://b.example/ucan/")
                .await
                .unwrap();

        let candidate_a_hex = hex::encode(candidate_a.bytes());
        let candidate_b_hex = hex::encode(candidate_b.bytes());
        let (a, b) = tokio::join!(
            establish_descriptor(&store, &account, &candidate_a_hex),
            establish_descriptor(&store, &account, &candidate_b_hex),
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(a.created, b.created);
        assert_eq!(a.descriptor, b.descriptor);
        assert!(
            a.descriptor == candidate_a.bytes() || a.descriptor == candidate_b.bytes(),
            "every caller must receive the exact stored candidate"
        );
        assert_eq!(
            store
                .account_by_root(&root_did)
                .await
                .unwrap()
                .unwrap()
                .repository_descriptor
                .unwrap(),
            a.descriptor
        );
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_descriptors_without_partial_state() {
        let store = SqliteStore::in_memory().unwrap();
        let root = dialog_credentials::Ed25519Signer::import(&[7; 32])
            .await
            .unwrap();
        let root_did = root.did().to_string();
        store
            .create_account("a@x.com", &root_did, "cred", 1)
            .await
            .unwrap();
        let account = store.account_by_root(&root_did).await.unwrap().unwrap();

        assert!(establish_descriptor(&store, &account, "00").await.is_err());
        assert!(
            store
                .account_by_root(&root_did)
                .await
                .unwrap()
                .unwrap()
                .repository_descriptor
                .is_none()
        );
    }
}
