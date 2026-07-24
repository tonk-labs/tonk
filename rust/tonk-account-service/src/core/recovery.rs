//! Surviving-device recovery: a linked device plus a freshly created
//! passkey re-anchor the account when the old passkey is gone.

use crate::auth::Caller;
use crate::core::CeremonyError;
use crate::core::delegation::check_subject_open_delegation;
use crate::store::Store;

/// Flip `caller.account` onto `new_root_did` under the authority of one
/// of its active devices plus proof of control of the new root.
///
/// The surviving device's row is repointed at its fresh
/// `newRoot → device` delegation so it can keep making device-signed
/// calls; every other device keeps its old-root delegation (still valid
/// for space access) and re-links on its next ceremony. The old passkey
/// credential is superseded: the registry no longer honors the root it
/// derives.
pub async fn recover_account<S: Store>(
    store: &S,
    caller: &Caller,
    new_root_did: &str,
    new_credential_id: &str,
    device_delegation_hex: &str,
) -> Result<(), CeremonyError> {
    let delegation_cid = check_subject_open_delegation(
        device_delegation_hex,
        new_root_did,
        &caller.device.device_did,
    )
    .await?;
    let repointed = store
        .update_device_delegation(
            caller.account.id,
            &caller.device.device_did,
            &delegation_cid,
        )
        .await?;
    if !repointed {
        return Err(CeremonyError::Invalid(
            "surviving device is not registered under this account".to_string(),
        ));
    }
    store
        .rotate_root(caller.account.id, new_root_did, new_credential_id)
        .await?;
    Ok(())
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::auth::authorize;
    use crate::store::sqlite::SqliteStore;
    use crate::store::{Device, DeviceStatus, Store};
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::promise::Promised;
    use dialog_ucan_core::time::timestamp::Timestamp;
    use dialog_ucan_core::{InvocationBuilder, InvocationChain};
    use dialog_varsig::Principal;
    use std::collections::BTreeMap;

    const OLD_ROOT_PRF: [u8; 32] = [7u8; 32];
    const NEW_ROOT_PRF: [u8; 32] = [9u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    /// Build the device-signed recovery container: subject = the old
    /// root, issuer = the surviving device, proof = its existing
    /// `oldRoot → device` delegation — the same `container(...)` shape
    /// as `auth.rs`'s test module.
    async fn container(
        old_root: Ed25519Signer,
        device: Ed25519Signer,
        args: BTreeMap<String, Promised>,
    ) -> Vec<u8> {
        let old_root_did = old_root.did();
        let chain = tonk_identity::delegation::mint_device_delegation(old_root, &device.did())
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device)
            .audience(&old_root_did)
            .subject(&old_root_did)
            .command(vec!["account".into(), "recover".into()])
            .arguments(args)
            .proofs(vec![cid])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        InvocationChain::new(invocation, proofs).to_bytes().unwrap()
    }

    fn recovery_args(
        new_root_did: &str,
        new_credential_id: &str,
        device_delegation_hex: &str,
    ) -> BTreeMap<String, Promised> {
        let mut args = BTreeMap::new();
        args.insert(
            "newRootDid".to_string(),
            Promised::String(new_root_did.to_string()),
        );
        args.insert(
            "newCredentialId".to_string(),
            Promised::String(new_credential_id.to_string()),
        );
        args.insert(
            "deviceDelegation".to_string(),
            Promised::String(device_delegation_hex.to_string()),
        );
        args
    }

    /// Seed an account (root PRF `[7u8; 32]`) with one active device
    /// (seed `[8u8; 32]`).
    async fn fixture(store: &SqliteStore) -> (Ed25519Signer, Ed25519Signer, String, String) {
        let old_root = tonk_identity::derive::derive_root_signer(&OLD_ROOT_PRF)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let old_root_did = old_root.did().to_string();
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

        (old_root, device, old_root_did, device_did)
    }

    #[dialog_common::test]
    async fn it_flips_the_root_under_device_and_new_root_authority() {
        let store = SqliteStore::in_memory().unwrap();
        let (old_root, device, _old_root_did, device_did) = fixture(&store).await;
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let new_root_did = new_root.did().to_string();

        let device_delegation =
            tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
                .await
                .unwrap();
        let device_delegation_hex = hex::encode(device_delegation.to_bytes().unwrap());

        let args = recovery_args(&new_root_did, "cred-new", &device_delegation_hex);
        let bytes = container(old_root, device, args).await;
        let caller = authorize(&store, &bytes, &["account", "recover"])
            .await
            .unwrap();
        let account_id = caller.account.id;

        recover_account(
            &store,
            &caller,
            &new_root_did,
            "cred-new",
            &device_delegation_hex,
        )
        .await
        .unwrap();

        let rotated = store.account_by_root(&new_root_did).await.unwrap().unwrap();
        assert_eq!(rotated.id, account_id);
        assert_eq!(rotated.credential_id, "cred-new");
        let repointed = store.device_by_did(&device_did).await.unwrap().unwrap();
        assert_ne!(repointed.delegation_cid, "bafyOld");
        assert_eq!(repointed.status, DeviceStatus::Active);
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_delegation_not_issued_by_the_new_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (old_root, device, old_root_did, _device_did) = fixture(&store).await;
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let new_root_did = new_root.did().to_string();
        let stranger = tonk_identity::derive::derive_root_signer(&[13u8; 32])
            .await
            .unwrap();

        // Bogus deviceDelegation minted by a third key, not the new root.
        let bogus = tonk_identity::delegation::mint_device_delegation(stranger, &device.did())
            .await
            .unwrap();
        let bogus_hex = hex::encode(bogus.to_bytes().unwrap());

        let args = recovery_args(&new_root_did, "cred-new", &bogus_hex);
        let bytes = container(old_root, device, args).await;
        let caller = authorize(&store, &bytes, &["account", "recover"])
            .await
            .unwrap();

        assert!(matches!(
            recover_account(&store, &caller, &new_root_did, "cred-new", &bogus_hex).await,
            Err(CeremonyError::Invalid(_))
        ));
        // Account untouched.
        assert!(
            store
                .account_by_root(&old_root_did)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_revoked_surviving_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (old_root, device, _old_root_did, device_did) = fixture(&store).await;
        let new_root = tonk_identity::derive::derive_root_signer(&NEW_ROOT_PRF)
            .await
            .unwrap();
        let new_root_did = new_root.did().to_string();
        let device_delegation =
            tonk_identity::delegation::mint_device_delegation(new_root, &device.did())
                .await
                .unwrap();
        let device_delegation_hex = hex::encode(device_delegation.to_bytes().unwrap());

        let args = recovery_args(&new_root_did, "cred-new", &device_delegation_hex);
        let bytes = container(old_root, device, args).await;
        let caller = authorize(&store, &bytes, &["account", "recover"])
            .await
            .unwrap();

        // The device row is revoked only after the Caller has already
        // been constructed: the repoint itself must catch this.
        store
            .revoke_device(caller.account.id, &device_did)
            .await
            .unwrap();

        assert!(matches!(
            recover_account(
                &store,
                &caller,
                &new_root_did,
                "cred-new",
                &device_delegation_hex
            )
            .await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}
