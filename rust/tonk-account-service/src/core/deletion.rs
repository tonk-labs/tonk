//! Permanent account-service removal.

use serde::Serialize;

use crate::chains::ChainStore;
use crate::store::Store;

use super::CeremonyError;

/// Successful removal from the account service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionReceipt {
    /// Root DID whose account-service namespace was removed.
    pub root_did: String,
    /// Verified email released for a future account.
    pub email: String,
}

/// Delete all account-service objects and rows after exact email confirmation.
///
/// Object storage goes first. If that fails, the D1 identity remains and the
/// root can retry. The dependent-row D1 removal is one transaction/batch.
pub async fn delete_account<S: Store, C: ChainStore>(
    store: &S,
    chains: &C,
    root_did: &str,
    confirmed_email: &str,
) -> Result<AccountDeletionReceipt, CeremonyError> {
    let account = store
        .account_by_root(root_did)
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;
    if confirmed_email.trim().to_lowercase() != account.email {
        return Err(CeremonyError::Forbidden(
            "email confirmation does not match this account".to_string(),
        ));
    }

    chains.delete_namespace(root_did).await?;
    if !store.delete_account(account.id, &account.email).await? {
        return Err(CeremonyError::Internal(
            "account disappeared while deletion was finalizing".to_string(),
        ));
    }
    Ok(AccountDeletionReceipt {
        root_did: root_did.to_string(),
        email: account.email,
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use crate::chains::{ChainStore, MemoryChainStore, SpotHeadSlot};
    use crate::store::sqlite::SqliteStore;
    use crate::store::{CodeRow, Device, DeviceStatus, LinkRequest, Store};

    async fn populated() -> (SqliteStore, MemoryChainStore, String, i64) {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let root = "did:key:z6Mktest-root".to_string();
        let account_id = store
            .create_account("owner@example.com", &root, "credential", 1)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                id: 0,
                account_id,
                device_did: "did:key:z6Mktest-device".into(),
                attachment_id: "attachment".into(),
                delegation_cid: "bafkdelegation".into(),
                delegation_hex: "aa".into(),
                name: "laptop".into(),
                status: DeviceStatus::Active,
                created_at: 1,
            })
            .await
            .unwrap();
        store
            .put_link(&LinkRequest {
                token_hash: "link".into(),
                device_did: "did:key:z6Mkpending".into(),
                device_name: "phone".into(),
                account_id: None,
                attachment_id: None,
                delegation_cid: None,
                delegation_hex: None,
                descriptor_hex: None,
                created_at: 1,
                expires_at: 2,
                completed_at: None,
                consumed_at: None,
                activated_at: None,
                cancelled_at: None,
            })
            .await
            .unwrap();
        store
            .put_code(&CodeRow {
                email: "owner@example.com".into(),
                code_hash: "hash".into(),
                created_at: 1,
                expires_at: 2,
                attempts: 0,
            })
            .await
            .unwrap();
        chains.put(&root, "delegation", b"chain").await.unwrap();
        chains
            .put_spot_head(&root, "subject", SpotHeadSlot::Named, "blob")
            .await
            .unwrap();
        (store, chains, root, account_id)
    }

    #[dialog_common::test]
    async fn exact_email_confirmation_is_required_without_mutation() {
        let (store, chains, root, account_id) = populated().await;

        let error = super::delete_account(&store, &chains, &root, "wrong@example.com")
            .await
            .unwrap_err();

        assert!(matches!(error, crate::core::CeremonyError::Forbidden(_)));
        assert!(store.account_by_root(&root).await.unwrap().is_some());
        assert_eq!(store.devices(account_id).await.unwrap().len(), 1);
        assert_eq!(chains.list(&root).await.unwrap(), vec!["delegation"]);
    }

    #[dialog_common::test]
    async fn it_removes_account_objects_dependents_and_email_for_reuse() {
        let (store, chains, root, account_id) = populated().await;

        let receipt = super::delete_account(&store, &chains, &root, "owner@example.com")
            .await
            .unwrap();

        assert_eq!(receipt.root_did, root);
        assert_eq!(receipt.email, "owner@example.com");
        assert!(store.account_by_root(&root).await.unwrap().is_none());
        assert!(
            store
                .account_by_email("owner@example.com")
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.devices(account_id).await.unwrap().is_empty());
        assert!(store.code("owner@example.com").await.unwrap().is_none());
        assert!(chains.list(&root).await.unwrap().is_empty());
        assert!(
            chains
                .list_spot_heads(&root, SpotHeadSlot::Named)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .create_account("owner@example.com", "did:key:z6Mknew", "new", 3)
                .await
                .is_ok()
        );
    }
}
