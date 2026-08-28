//! Permanent account-service removal.

use serde::Serialize;

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

/// Delete all account-service rows after exact email confirmation.
///
/// The dependent-row D1 removal is one transaction/batch. (The escrow
/// object store this once had to purge first no longer exists.)
pub async fn delete_account<S: Store>(
    store: &S,
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
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus, Store};

    async fn populated() -> (SqliteStore, String, i64) {
        let store = SqliteStore::in_memory().unwrap();
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
        (store, root, account_id)
    }

    #[dialog_common::test]
    async fn exact_email_confirmation_is_required_without_mutation() {
        let (store, root, account_id) = populated().await;

        let error = super::delete_account(&store, &root, "wrong@example.com")
            .await
            .unwrap_err();

        assert!(matches!(error, crate::core::CeremonyError::Forbidden(_)));
        assert!(store.account_by_root(&root).await.unwrap().is_some());
        assert_eq!(store.devices(account_id).await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_removes_account_objects_dependents_and_email_for_reuse() {
        let (store, root, account_id) = populated().await;

        let receipt = super::delete_account(&store, &root, "owner@example.com")
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
        assert!(
            store
                .create_account("owner@example.com", "did:key:z6Mknew", "new", 3)
                .await
                .is_ok()
        );
    }
}
