//! Device registry operations: list, register, and revoke devices
//! under an account's root DID.

use crate::chains::ChainStore;
use crate::core::CeremonyError;
use crate::core::delegation::check_device_delegation;
use crate::core::revocation::{Attestation, check_revocation, put_revocation};
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

/// Revoke a device under an account, recording the signed revocation
/// that authorized it when one is supplied.
///
/// A supplied artifact is verified and stored *before* the status
/// flips: a stored artifact with no flag is a recoverable
/// inconsistency, while a flag with no artifact is a silent gap in the
/// audit trail. A rejected artifact leaves the device active.
///
/// The artifact is optional so that callers which cannot yet mint one —
/// anything without access to the account root, which today is every
/// caller but the browser — keep working. Requiring root attestation
/// for cross-device revocation is a later, deliberate tightening; it
/// closes the exposure where any device can lock out its siblings, and
/// it must not land before a device that lost its passkey has a
/// recovery path.
///
/// Returns the attestation level when an artifact was recorded.
/// Returns `CeremonyError::Invalid` if the device does not exist under
/// this account.
pub async fn revoke_device<S: Store, C: ChainStore>(
    store: &S,
    chains: &C,
    account: &Account,
    device_did: &str,
    revocation: Option<&[u8]>,
) -> Result<Option<Attestation>, CeremonyError> {
    let device = store
        .device_by_did(device_did)
        .await?
        .filter(|device| device.account_id == account.id)
        .ok_or_else(|| CeremonyError::Invalid("unknown device".to_string()))?;

    let attestation = match revocation {
        Some(bytes) => {
            let attestation = check_revocation(bytes, &account.root_did, &device).await?;
            put_revocation(chains, account, attestation, bytes).await?;
            Some(attestation)
        }
        None => None,
    };

    let changed = store.revoke_device(account.id, device_did).await?;
    if !changed {
        return Err(CeremonyError::Invalid("unknown device".to_string()));
    }
    Ok(attestation)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::chains::MemoryChainStore;
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

    /// Register a device, then mint the root-signed revocation of its
    /// grant. Returns (device did, revocation container bytes).
    async fn registered_with_revocation(
        store: &SqliteStore,
        account: &Account,
    ) -> (String, Vec<u8>) {
        let (device_did, delegation_hex) = delegation_from(ROOT_PRF, DEVICE_SEED).await;
        register_device(store, account, &device_did, "phone", &delegation_hex, 200)
            .await
            .unwrap();
        let device = store.device_by_did(&device_did).await.unwrap().unwrap();
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let revocation =
            tonk_identity::revocation::mint_root_revocation(root, &device.delegation_cid)
                .await
                .unwrap();
        (device_did, revocation)
    }

    #[dialog_common::test]
    async fn it_revokes_and_reflects_status_in_the_listing() {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let account = seeded_account(&store).await;
        let (device_did, revocation) = registered_with_revocation(&store, &account).await;

        revoke_device(&store, &chains, &account, &device_did, Some(&revocation))
            .await
            .unwrap();

        let views = list_devices(&store, &account).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "revoked");
    }

    #[dialog_common::test]
    async fn it_revokes_without_an_artifact_and_records_nothing() {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let account = seeded_account(&store).await;
        let (device_did, _) = registered_with_revocation(&store, &account).await;

        let attestation = revoke_device(&store, &chains, &account, &device_did, None)
            .await
            .unwrap();

        assert!(
            attestation.is_none(),
            "a caller that cannot mint an artifact still revokes"
        );
        let views = list_devices(&store, &account).await.unwrap();
        assert_eq!(views[0].status, "revoked");
        let stored = crate::core::revocation::list_revocations(&chains, &account)
            .await
            .unwrap();
        assert!(
            stored.is_empty(),
            "nothing unsigned enters the artifact set"
        );
    }

    #[dialog_common::test]
    async fn it_stores_the_artifact_alongside_the_status_flip() {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let account = seeded_account(&store).await;
        let (device_did, revocation) = registered_with_revocation(&store, &account).await;

        let attestation = revoke_device(&store, &chains, &account, &device_did, Some(&revocation))
            .await
            .unwrap();

        assert_eq!(attestation, Some(Attestation::Root));
        let stored = crate::core::revocation::list_revocations(&chains, &account)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].attestation, "root");
    }

    #[dialog_common::test]
    async fn it_leaves_the_device_active_when_the_artifact_is_rejected() {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let account = seeded_account(&store).await;
        let (device_did, _) = registered_with_revocation(&store, &account).await;
        let foreign = tonk_identity::derive::derive_root_signer(&FOREIGN_ROOT_PRF)
            .await
            .unwrap();
        let device = store.device_by_did(&device_did).await.unwrap().unwrap();
        let forged =
            tonk_identity::revocation::mint_root_revocation(foreign, &device.delegation_cid)
                .await
                .unwrap();

        let result = revoke_device(&store, &chains, &account, &device_did, Some(&forged)).await;

        assert!(matches!(result, Err(CeremonyError::Forbidden(_))));
        let views = list_devices(&store, &account).await.unwrap();
        assert_eq!(
            views[0].status, "active",
            "a rejected artifact must not flip the status"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_revoking_a_device_of_another_account() {
        let store = SqliteStore::in_memory().unwrap();
        let chains = MemoryChainStore::default();
        let account = seeded_account(&store).await;
        let (device_did, revocation) = registered_with_revocation(&store, &account).await;
        let foreign = Account {
            id: account.id + 1,
            ..account.clone()
        };

        let result = revoke_device(&store, &chains, &foreign, &device_did, Some(&revocation)).await;

        assert!(matches!(result, Err(CeremonyError::Invalid(_))));
    }
}
