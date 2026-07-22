//! Device registry operations: list, register, and revoke devices
//! under an account's root DID.

use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::store::{Account, Device, DeviceStatus, Store};

/// A device row as surfaced to API callers.
pub struct DeviceView {
    /// The device's DID.
    pub did: String,
    /// Human-readable device name.
    pub name: String,
    /// `"active"` or `"revoked"`.
    pub status: String,
    /// CID of the root → device delegation.
    pub delegation_cid: String,
    /// Creation time, as a unix timestamp in seconds.
    pub created_at: u64,
}

impl From<Device> for DeviceView {
    fn from(device: Device) -> Self {
        DeviceView {
            did: device.device_did,
            name: device.name,
            status: device.status.as_str().to_string(),
            delegation_cid: device.delegation_cid,
            created_at: device.created_at,
        }
    }
}

/// List all devices registered under an account, in store order.
pub async fn list_devices<S: Store>(
    store: &S,
    account: &Account,
) -> Result<Vec<DeviceView>, CeremonyError> {
    let devices = store.devices(account.id).await?;
    Ok(devices.into_iter().map(DeviceView::from).collect())
}

/// Register a new device under an account.
///
/// Checks that `delegation_hex` is a valid `root → device` delegation
/// issued by `account.root_did` to `device_did` before registering the
/// device as active. A duplicate device DID surfaces as
/// `CeremonyError::Conflict`.
pub async fn register_device<S: Store>(
    store: &S,
    account: &Account,
    device_did: &str,
    device_name: &str,
    delegation_hex: &str,
    now: u64,
) -> Result<(), CeremonyError> {
    let delegation_cid =
        check_device_delegation(delegation_hex, &account.root_did, device_did).await?;
    store
        .insert_device(&Device {
            account_id: account.id,
            device_did: device_did.to_string(),
            delegation_cid,
            name: device_name.to_string(),
            status: DeviceStatus::Active,
            created_at: now,
        })
        .await?;
    Ok(())
}

/// Revoke a device under an account.
///
/// Returns `CeremonyError::Invalid` if the device does not exist under
/// this account.
pub async fn revoke_device<S: Store>(
    store: &S,
    account: &Account,
    device_did: &str,
) -> Result<(), CeremonyError> {
    let changed = store.revoke_device(account.id, device_did).await?;
    if !changed {
        return Err(CeremonyError::Invalid("unknown device".to_string()));
    }
    Ok(())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [11u8; 32];
    const FOREIGN_ROOT_PRF: [u8; 32] = [9u8; 32];

    /// Seed an account through the store directly (root PRF `[7u8; 32]`,
    /// the same root as Task 5's `fixture()`), then mint a fresh
    /// `root → device` delegation from the given root signer PRF to a
    /// device derived from `device_seed`.
    async fn delegation_from(root_prf: [u8; 32], device_seed: [u8; 32]) -> (String, String) {
        use dialog_varsig::Principal;
        let root = tonk_identity::derive::derive_root_signer(&root_prf)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&device_seed)
            .await
            .unwrap();
        let device_did = device.did().to_string();
        let chain = tonk_identity::delegation::mint_device_delegation(root, &device.did())
            .await
            .unwrap();
        (device_did, hex::encode(chain.to_bytes().unwrap()))
    }

    async fn seeded_account(store: &SqliteStore) -> Account {
        use dialog_varsig::Principal;
        let root_did = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap()
            .did()
            .to_string();
        store
            .create_account("a@x.com", &root_did, "cred", 100)
            .await
            .unwrap();
        store.account_by_root(&root_did).await.unwrap().unwrap()
    }

    #[dialog_common::test]
    async fn it_registers_a_device_delegated_by_the_account_root() {
        let store = SqliteStore::in_memory().unwrap();
        let account = seeded_account(&store).await;
        let (device_did, delegation_hex) = delegation_from(ROOT_PRF, DEVICE_SEED).await;

        register_device(&store, &account, &device_did, "phone", &delegation_hex, 200)
            .await
            .unwrap();

        let views = list_devices(&store, &account).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].did, device_did);
        assert_eq!(views[0].name, "phone");
        assert_eq!(views[0].status, "active");
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_delegated_by_a_foreign_root() {
        let store = SqliteStore::in_memory().unwrap();
        let account = seeded_account(&store).await;
        let (device_did, delegation_hex) = delegation_from(FOREIGN_ROOT_PRF, DEVICE_SEED).await;

        let result =
            register_device(&store, &account, &device_did, "phone", &delegation_hex, 200).await;
        assert!(matches!(result, Err(CeremonyError::Invalid(_))));
    }

    #[dialog_common::test]
    async fn it_revokes_and_reflects_status_in_the_listing() {
        let store = SqliteStore::in_memory().unwrap();
        let account = seeded_account(&store).await;
        let (device_did, delegation_hex) = delegation_from(ROOT_PRF, DEVICE_SEED).await;
        register_device(&store, &account, &device_did, "phone", &delegation_hex, 200)
            .await
            .unwrap();

        revoke_device(&store, &account, &device_did).await.unwrap();

        let views = list_devices(&store, &account).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "revoked");
    }
}
