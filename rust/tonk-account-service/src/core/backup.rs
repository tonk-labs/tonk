//! Chain backup: content-addressed storage of delegation chain bytes,
//! namespaced by an account's root DID.

use crate::chains::ChainStore;
use crate::core::CeremonyError;
use crate::store::Account;

/// The content-addressed key for `bytes`: the blake3 hash, hex-encoded.
pub fn chain_key(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Store `bytes` under `account.root_did`'s namespace, returning the
/// content-addressed key. Re-storing identical bytes is idempotent.
pub async fn put_chain<C: ChainStore>(
    chains: &C,
    account: &Account,
    bytes: &[u8],
) -> Result<String, CeremonyError> {
    let key = chain_key(bytes);
    chains.put(&account.root_did, &key, bytes).await?;
    Ok(key)
}

/// List the chain keys backed up under `account.root_did`'s namespace.
pub async fn list_chains<C: ChainStore>(
    chains: &C,
    account: &Account,
) -> Result<Vec<String>, CeremonyError> {
    Ok(chains.list(&account.root_did).await?)
}

/// Look up the bytes backed up under `key` in `account.root_did`'s
/// namespace.
///
/// Returns `CeremonyError::Invalid` if no such chain is backed up.
pub async fn get_chain<C: ChainStore>(
    chains: &C,
    account: &Account,
    key: &str,
) -> Result<Vec<u8>, CeremonyError> {
    chains
        .get(&account.root_did, key)
        .await?
        .ok_or_else(|| CeremonyError::Invalid("unknown chain".to_string()))
}

#[cfg(all(test, feature = "helpers"))]
mod tests {
    use super::*;
    use crate::chains::MemoryChainStore;

    fn account(id: i64, root_did: &str) -> Account {
        Account {
            id,
            email: format!("account-{id}@x.com"),
            root_did: root_did.to_string(),
            credential_id: "cred".to_string(),
            repository_descriptor: None,
            created_at: 100,
        }
    }

    #[dialog_common::test]
    async fn it_stores_chains_content_addressed_and_idempotent() {
        let chains = MemoryChainStore::default();
        let account = account(1, "did:key:root1");
        let bytes = b"chain-bytes";

        let key1 = put_chain(&chains, &account, bytes).await.unwrap();
        let key2 = put_chain(&chains, &account, bytes).await.unwrap();

        assert_eq!(key1, key2);
        let listed = list_chains(&chains, &account).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], key1);
    }

    #[dialog_common::test]
    async fn it_scopes_backups_to_the_root_did() {
        let chains = MemoryChainStore::default();
        let account_a = account(1, "did:key:root-a");
        let account_b = account(2, "did:key:root-b");

        put_chain(&chains, &account_a, b"a-bytes").await.unwrap();
        put_chain(&chains, &account_b, b"b-bytes").await.unwrap();

        let listed_a = list_chains(&chains, &account_a).await.unwrap();
        let listed_b = list_chains(&chains, &account_b).await.unwrap();
        assert_eq!(listed_a.len(), 1);
        assert_eq!(listed_b.len(), 1);
        assert_ne!(listed_a[0], listed_b[0]);
    }

    #[dialog_common::test]
    async fn it_round_trips_chain_bytes() {
        let chains = MemoryChainStore::default();
        let account = account(1, "did:key:root1");
        let bytes = b"chain-bytes".to_vec();

        let key = put_chain(&chains, &account, &bytes).await.unwrap();
        let round_tripped = get_chain(&chains, &account, &key).await.unwrap();

        assert_eq!(round_tripped, bytes);
    }

    #[dialog_common::test]
    async fn it_refuses_a_chain_get_across_accounts() {
        let chains = MemoryChainStore::default();
        let account_a = account(1, "did:key:root-a");
        let account_b = account(2, "did:key:root-b");

        let key = put_chain(&chains, &account_a, b"a-bytes").await.unwrap();

        assert!(matches!(
            get_chain(&chains, &account_b, &key).await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}
