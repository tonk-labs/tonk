//! UCAN authentication for registry endpoints.
//!
//! Endpoints outside the email-code bootstrap ceremonies are invoked as
//! signed UCAN containers: the invocation subject is the account's root
//! DID, and the invocation issuer is a device delegated under that
//! root. [`authorize`] parses and cryptographically verifies such a
//! container, checks the command matches what the caller expects, then
//! binds it to a registered account and one of its active devices.

use std::collections::BTreeMap;

use dialog_credentials::Ed25519KeyResolver;
use dialog_ucan_core::InvocationChain;
use dialog_ucan_core::promise::Promised;

use crate::core::CeremonyError;
use crate::store::{Account, Device, DeviceStatus, Store};

/// An authenticated caller: the account and device bound by a verified
/// UCAN invocation, plus the invocation's arguments.
pub struct Caller {
    /// The account the invocation's subject DID resolved to.
    pub account: Account,
    /// The device the invocation's issuer DID resolved to.
    pub device: Device,
    /// The invocation's arguments.
    pub arguments: BTreeMap<String, Promised>,
}

/// Parse + cryptographically verify an invocation container, require the
/// exact command, then bind it to a registered account and an active
/// device. The invocation subject is the root DID; the invocation issuer
/// must be a non-revoked device of that account.
pub async fn authorize<S: Store>(
    store: &S,
    body: &[u8],
    expected_command: &[&str],
) -> Result<Caller, CeremonyError> {
    let chain = InvocationChain::try_from(body)
        .map_err(|err| CeremonyError::Invalid(format!("bad invocation container: {err}")))?;

    chain.verify(&Ed25519KeyResolver).await.map_err(|err| {
        CeremonyError::Unauthorized(format!("invocation failed to verify: {err}"))
    })?;

    let command_segments: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if command_segments.as_slice() != expected_command {
        return Err(CeremonyError::Forbidden(format!(
            "expected command {expected_command:?}, got {command_segments:?}"
        )));
    }

    let account = store
        .account_by_root(chain.subject().as_ref())
        .await?
        .ok_or_else(|| CeremonyError::Unauthorized("unknown account".to_string()))?;

    let device = store
        .device_by_did(chain.issuer().as_ref())
        .await?
        .filter(|device| device.account_id == account.id && device.status == DeviceStatus::Active)
        .ok_or_else(|| {
            CeremonyError::Forbidden("device is not an active member of this account".to_string())
        })?;

    Ok(Caller {
        account,
        device,
        arguments: chain.arguments().clone(),
    })
}

/// Extract a required string argument.
pub fn string_argument(caller: &Caller, name: &str) -> Result<String, CeremonyError> {
    match caller.arguments.get(name) {
        Some(Promised::String(value)) => Ok(value.clone()),
        _ => Err(CeremonyError::Invalid(format!(
            "missing or invalid argument: {name}"
        ))),
    }
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::store::sqlite::SqliteStore;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationBuilder;
    use dialog_varsig::Principal;

    const ROOT_PRF: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    async fn container(
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
    ) -> (String, String, Vec<u8>) {
        let root = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let root_did = root.did();
        let chain = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(command)
            .arguments(args)
            .proofs(vec![cid])
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (root_did.to_string(), device.did().to_string(), bytes)
    }

    async fn seed_device(
        store: &SqliteStore,
        root_did: &str,
        device_did: &str,
        status: DeviceStatus,
    ) -> i64 {
        let account_id = store
            .create_account("a@x.com", root_did, "cred", 0)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id,
                device_did: device_did.to_string(),
                delegation_cid: "cid".to_string(),
                name: "laptop".to_string(),
                status,
                created_at: 0,
            })
            .await
            .unwrap();
        account_id
    }

    #[dialog_common::test]
    async fn it_authorizes_a_registered_device_acting_as_its_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        let caller = authorize(&store, &bytes, &["account", "device", "list"])
            .await
            .unwrap();
        assert_eq!(caller.account.root_did, root_did);
        assert_eq!(caller.device.device_did, device_did);
    }

    #[dialog_common::test]
    async fn it_rejects_an_unknown_root() {
        let store = SqliteStore::in_memory().unwrap();
        let (_, _, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_revoked_device() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        let account_id = seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;
        store.revoke_device(account_id, &device_did).await.unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_command_mismatch() {
        let store = SqliteStore::in_memory().unwrap();
        let (root_did, device_did, bytes) = container(
            vec!["account".into(), "device".into(), "list".into()],
            BTreeMap::new(),
        )
        .await;
        seed_device(&store, &root_did, &device_did, DeviceStatus::Active).await;

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "revoke"]).await,
            Err(CeremonyError::Forbidden(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_device_of_a_different_account() {
        let store = SqliteStore::in_memory().unwrap();

        let root_a = tonk_identity::derive::derive_root_signer(&ROOT_PRF)
            .await
            .unwrap();
        let root_b = tonk_identity::derive::derive_root_signer(&[9u8; 32])
            .await
            .unwrap();
        let device_b = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();

        let root_a_did = root_a.did();
        let root_b_did = root_b.did();
        let device_b_did = device_b.did();

        store
            .create_account("a@x.com", root_a_did.as_ref(), "cred-a", 0)
            .await
            .unwrap();
        let account_b = store
            .create_account("b@x.com", root_b_did.as_ref(), "cred-b", 0)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                account_id: account_b,
                device_did: device_b_did.to_string(),
                delegation_cid: "cid".to_string(),
                name: "phone".to_string(),
                status: DeviceStatus::Active,
                created_at: 0,
            })
            .await
            .unwrap();

        // The delegation is legitimately root B -> device B, but the
        // invocation claims subject = root A: the chain from subject to
        // issuer breaks, so verification itself must fail.
        let chain = tonk_identity::delegation::mint_device_delegation(root_b, &device_b_did)
            .await
            .unwrap();
        let delegation = chain.proofs().last().unwrap().clone();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(device_b.clone())
            .audience(&root_a_did)
            .subject(&root_a_did)
            .command(vec!["account".into(), "device".into(), "list".into()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .try_build()
            .await
            .unwrap();
        let mut proofs = std::collections::HashMap::new();
        proofs.insert(cid, std::sync::Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();

        assert!(matches!(
            authorize(&store, &bytes, &["account", "device", "list"]).await,
            Err(CeremonyError::Unauthorized(_))
        ));
    }
}
