//! The account rotation ceremony: flip the account onto a new root DID
//! under authority of the old root, keeping every registered device.

use crate::core::CeremonyError;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::{Account, Store};

/// A verified request to rotate an account onto a new root.
pub struct RotateAccount {
    /// The DID the account rotates onto.
    pub new_root_did: String,
    /// The passkey credential backing the new root.
    pub new_credential_id: String,
    /// Hex-encoded subject-open `oldRoot → newRoot` succession chain.
    pub succession_hex: String,
    /// The ceremony device re-registering under the new root.
    pub device_did: String,
    /// Hex-encoded subject-open `newRoot → device` delegation chain.
    pub device_delegation_hex: String,
}

/// Rotate `account` onto `request.new_root_did`.
///
/// Verifies the succession chain (old root delegates to the new root) and
/// the ceremony device's fresh delegation before touching the registry.
/// Devices other than the ceremony device keep their existing rows: their
/// old-root delegations remain cryptographically valid for space access,
/// and they re-link on their next service ceremony.
pub async fn rotate_account<S: Store>(
    store: &S,
    account: &Account,
    request: &RotateAccount,
) -> Result<(), CeremonyError> {
    check_subject_open_delegation(
        &request.succession_hex,
        &account.root_did,
        &request.new_root_did,
    )
    .await?;
    let delegation_cid = check_subject_open_delegation(
        &request.device_delegation_hex,
        &request.new_root_did,
        &request.device_did,
    )
    .await?;

    let repointed = store
        .update_device_delegation(account.id, &request.device_did, &delegation_cid)
        .await?;
    if !repointed {
        return Err(CeremonyError::Invalid(
            "ceremony device is not registered under this account".to_string(),
        ));
    }
    store
        .rotate_root(
            account.id,
            &request.new_root_did,
            &request.new_credential_id,
        )
        .await?;
    Ok(())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus, Store};
    use dialog_varsig::Principal;

    const OLD_ROOT_PRF: [u8; 32] = [7u8; 32];
    const NEW_ROOT_PRF: [u8; 32] = [9u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn fixture(store: &SqliteStore) -> (crate::store::Account, RotateAccount, String) {
        let old_root = tonk_identity::derive::derive_root_signer(&OLD_ROOT_PRF)
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&DEVICE_SEED)
            .await
            .unwrap();
        let old_root_did = old_root.did().to_string();
        let new_root_did = new_root.did().to_string();
        let device_did = device.did().to_string();

        let id = store
            .create_account("a@x.com", &old_root_did, "cred-old", 100)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: id,
                device_did: device_did.clone(),
                delegation_cid: "bafyOld".into(),
                name: "laptop".into(),
                status: DeviceStatus::Active,
                created_at: 100,
            })
            .await
            .unwrap();

        let succession = tonk_identity::delegation::mint_root_succession(old_root, &new_root.did())
            .await
            .unwrap();
        let device_link =
            tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
                .await
                .unwrap();
        let account = store.account_by_root(&old_root_did).await.unwrap().unwrap();
        let request = RotateAccount {
            new_root_did: new_root_did.clone(),
            new_credential_id: "cred-new".into(),
            succession_hex: hex::encode(succession.to_bytes().unwrap()),
            device_did,
            device_delegation_hex: hex::encode(device_link.to_bytes().unwrap()),
        };
        (account, request, new_root_did)
    }

    #[dialog_common::test]
    async fn it_flips_the_root_and_repoints_the_ceremony_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, request, new_root_did) = fixture(&store).await;

        rotate_account(&store, &account, &request).await.unwrap();

        let rotated = store.account_by_root(&new_root_did).await.unwrap().unwrap();
        assert_eq!(rotated.id, account.id);
        assert_eq!(rotated.credential_id, "cred-new");
        let device = store
            .device_by_did(&request.device_did)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(device.delegation_cid, "bafyOld");
        assert_eq!(device.status, DeviceStatus::Active);
    }

    #[dialog_common::test]
    async fn it_rejects_a_succession_not_issued_by_the_account_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, mut request, _) = fixture(&store).await;
        // Succession minted by an unrelated key: issuer check must fail.
        let stranger = tonk_identity::derive::derive_root_signer(&[13u8; 32])
            .await
            .unwrap();
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let bogus = tonk_identity::delegation::mint_root_succession(stranger, &{
            use dialog_varsig::Principal;
            new_root.did()
        })
        .await
        .unwrap();
        request.succession_hex = hex::encode(bogus.to_bytes().unwrap());

        assert!(matches!(
            rotate_account(&store, &account, &request).await,
            Err(CeremonyError::Invalid(_))
        ));
        // Nothing flipped.
        assert!(
            store
                .account_by_root(&account.root_did)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_ceremony_device_unknown_to_the_account() {
        let store = SqliteStore::in_memory().unwrap();
        let (account, mut request, _) = fixture(&store).await;
        request.device_did = "did:key:zGhost".into();
        assert!(matches!(
            rotate_account(&store, &account, &request).await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}
