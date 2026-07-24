//! The account creation ceremony: consume a verified email code, check
//! the presented `root → device` delegation, then register the account
//! and its first device.

use crate::core::CeremonyError;
use crate::core::codes::verify_code;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::{NewDevice, Store};

/// A request to create a new account and register its first device.
pub struct CreateAccount {
    /// The account's verified email address.
    pub email: String,
    /// The verification code sent to `email`.
    pub code: String,
    /// The account's root DID.
    pub root_did: String,
    /// Opaque identifier for the passkey credential backing the root.
    pub credential_id: String,
    /// The first device's DID.
    pub device_did: String,
    /// Human-readable name for the first device.
    pub device_name: String,
    /// Hex-encoded `root → device` delegation chain.
    pub delegation_hex: String,
}

/// Create a new account and register its first device.
///
/// Verifies `request.code` first, consuming it, then checks the
/// presented delegation before touching the account registry. The
/// email address is lowercased before being stored. The account and its
/// first device are created atomically: a conflict on the device DID
/// (for example, an attacker who has pre-registered a delegation to the
/// victim's device DID under a different account) rolls back the
/// account row too, rather than stranding a zero-device account that
/// has permanently burned the email and root DID.
pub async fn create_account<S: Store>(
    store: &S,
    request: &CreateAccount,
    now: u64,
) -> Result<i64, CeremonyError> {
    verify_code(store, &request.email, &request.code, now).await?;
    let delegation_cid = check_subject_open_delegation(
        &request.delegation_hex,
        &request.root_did,
        &request.device_did,
    )
    .await?;

    let account_id = store
        .create_account_with_device(
            &request.email.to_lowercase(),
            &request.root_did,
            &request.credential_id,
            &NewDevice {
                device_did: request.device_did.clone(),
                delegation_cid,
                name: request.device_name.clone(),
            },
            now,
        )
        .await?;

    Ok(account_id)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::core::codes::request_code;
    use crate::email::CapturedEmail;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus};

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn fixture() -> (String, String, String) {
        // (root_did, device_did, delegation_hex)
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = dialog_credentials::Ed25519Signer::import(&DEVICE_SEED)
            .await
            .unwrap();
        let root_did = {
            use dialog_varsig::Principal;
            root.did().to_string()
        };
        let device_did = {
            use dialog_varsig::Principal;
            device.did().to_string()
        };
        let chain = tonk_identity::delegation::mint_device_delegation(root, &{
            use dialog_varsig::Principal;
            device.did()
        })
        .await
        .unwrap();
        (root_did, device_did, hex::encode(chain.to_bytes().unwrap()))
    }

    #[dialog_common::test]
    async fn it_creates_an_account_with_a_valid_code_and_delegation() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
        };
        let id = create_account(&store, &request, 200).await.unwrap();
        let account = store.account_by_root(&root_did).await.unwrap().unwrap();
        assert_eq!((account.id, account.email.as_str()), (id, "a@x.com"));
        assert_eq!(store.devices(id).await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegation_issued_by_a_different_root() {
        // fixture delegation, but the request claims a different root DID:
        // possession of the claimed root is not proven, so creation fails.
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (_, device_did, delegation_hex) = fixture().await;
        let other_root = {
            use dialog_varsig::Principal;
            tonk_identity::derive::derive_root_signer(&[9u8; 32])
                .await
                .unwrap()
                .did()
                .to_string()
        };
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: other_root,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Invalid(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_bad_code_before_touching_the_registry() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, delegation_hex) = fixture().await;
        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "000000".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::CodeInvalid)
        ));
        assert!(store.account_by_root(&root_did).await.unwrap().is_none());
    }

    #[dialog_common::test]
    async fn it_rejects_a_second_account_for_the_same_email() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex) = fixture().await;
        let first = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did,
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
        };
        create_account(&store, &first, 200).await.unwrap();

        // Same email, different root: build a second fixture from PRF
        // [10u8; 32] and device seed [12u8; 32], mint its delegation as
        // in fixture(), request a fresh code past the cooldown.
        let root2 = tonk_identity::derive::derive_root_signer(&[10u8; 32])
            .await
            .unwrap();
        let device2 = dialog_credentials::Ed25519Signer::import(&[12u8; 32])
            .await
            .unwrap();
        let (root2_did, device2_did) = {
            use dialog_varsig::Principal;
            (root2.did().to_string(), device2.did().to_string())
        };
        let chain2 = {
            use dialog_varsig::Principal;
            tonk_identity::delegation::mint_device_delegation(root2, &device2.did())
                .await
                .unwrap()
        };
        request_code(&store, &sender, "a@x.com", "654321", 400)
            .await
            .unwrap();
        let second = CreateAccount {
            email: "a@x.com".into(),
            code: "654321".into(),
            root_did: root2_did,
            credential_id: "cred2".into(),
            device_did: device2_did,
            device_name: "phone".into(),
            delegation_hex: hex::encode(chain2.to_bytes().unwrap()),
        };
        assert!(matches!(
            create_account(&store, &second, 500).await,
            Err(CeremonyError::Conflict(_))
        ));
    }

    #[dialog_common::test]
    async fn it_does_not_strand_an_account_when_the_device_insert_conflicts() {
        let store = SqliteStore::in_memory().unwrap();
        let sender = CapturedEmail::default();
        request_code(&store, &sender, "a@x.com", "123456", 100)
            .await
            .unwrap();
        let (root_did, device_did, delegation_hex) = fixture().await;

        // Pre-register the fixture's device DID under a different
        // account, as an attacker front-running the victim's device DID
        // would. Without atomicity, the account insert below would
        // still succeed and the device insert would fail, permanently
        // burning the email and root DID with zero devices registered.
        let other_id = store
            .create_account("other@x.com", "did:key:zOther", "cred-other", 1)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: other_id,
                device_did: device_did.clone(),
                delegation_cid: "bafyStolen".into(),
                name: "attacker".into(),
                status: DeviceStatus::Active,
                created_at: 1,
            })
            .await
            .unwrap();

        let request = CreateAccount {
            email: "a@x.com".into(),
            code: "123456".into(),
            root_did: root_did.clone(),
            credential_id: "cred".into(),
            device_did,
            device_name: "laptop".into(),
            delegation_hex,
        };
        assert!(matches!(
            create_account(&store, &request, 200).await,
            Err(CeremonyError::Conflict(_))
        ));
        assert!(store.account_by_root(&root_did).await.unwrap().is_none());
    }
}
