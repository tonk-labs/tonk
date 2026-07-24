//! Device-signed account-service invocation containers.
//!
//! The account service's `authorize` accepts requests issued by a device
//! key whose `root → device` delegation is attached as a proof, with the
//! account root as subject. This builds exactly that container from a
//! profile's live device signer and its stored `root → device` link — no
//! root key, no raw seed. Invocations carry a five-minute expiration; the
//! service refuses stale ones.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};

/// Build a device-signed account-service invocation container.
///
/// `link` is the stored `root → device` delegation: its issuer is the
/// account root (the invocation subject and audience), and its single
/// proof is attached so the service can bind the device to the account.
pub async fn build_device_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    command: Vec<String>,
    arguments: BTreeMap<String, Promised>,
) -> Result<Vec<u8>> {
    let root_did = link.issuer().clone();
    debug_assert!(
        link.proofs().count() == 1,
        "build_device_invocation expects a single-hop root -> device link"
    );
    let delegation = link
        .proofs()
        .last()
        .context("account link carries no delegation to prove the device")?
        .clone();
    let cid = delegation.to_cid();

    let invocation = InvocationBuilder::new()
        .issuer(device)
        .audience(&root_did)
        .subject(&root_did)
        .command(command)
        .arguments(arguments)
        .proofs(vec![cid])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .context("failed to sign the device invocation")?;

    let mut proofs = HashMap::new();
    proofs.insert(cid, Arc::new(delegation));
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .context("failed to serialize the device invocation")
}

/// Build the device-signed half of the surviving-device recovery
/// ceremony: proof of the old `root → device` link (attached as `link`'s
/// proof), naming the new root, its credential, and the fresh
/// `newRoot → device` delegation the service installs on success.
pub async fn build_recovery_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    new_root_did: String,
    new_credential_id: String,
    device_delegation_hex: String,
) -> Result<Vec<u8>> {
    let mut arguments = BTreeMap::new();
    arguments.insert("newRootDid".to_owned(), Promised::String(new_root_did));
    arguments.insert(
        "newCredentialId".to_owned(),
        Promised::String(new_credential_id),
    );
    arguments.insert(
        "deviceDelegation".to_owned(),
        Promised::String(device_delegation_hex),
    );
    build_device_invocation(
        device,
        link,
        vec!["account".into(), "recover".into()],
        arguments,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::InvocationChain;
    use dialog_varsig::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_builds_a_device_signed_invocation_the_service_verifies() {
        let root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let root_did = root.did();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let device_did = device.did();
        let link = crate::delegation::mint_device_delegation(root, &device_did)
            .await
            .unwrap();

        let arguments = [("chain".to_owned(), Promised::String("deadbeef".to_owned()))]
            .into_iter()
            .collect();
        let bytes = build_device_invocation(
            device,
            &link,
            vec!["account".into(), "chain".into(), "put".into()],
            arguments,
        )
        .await
        .unwrap();

        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert!(
            chain.invocation.expiration().is_some(),
            "device invocations must carry a ceremony expiration"
        );
        assert_eq!(chain.issuer(), &device_did);
        assert_eq!(chain.subject(), &root_did);
        assert_eq!(
            chain.command().0,
            vec![
                "account".to_string(),
                "chain".to_string(),
                "put".to_string()
            ],
        );
    }

    #[dialog_common::test]
    async fn it_builds_a_device_signed_recovery_invocation() {
        let old_root = crate::derive::derive_root_signer(&[7u8; 32]).await.unwrap();
        let old_root_did = old_root.did();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let device_did = device.did();
        let link = crate::delegation::mint_device_delegation(old_root, &device_did)
            .await
            .unwrap();
        let expected_proof = link.proofs().last().unwrap().to_cid();

        let bytes = build_recovery_invocation(
            device,
            &link,
            "did:key:new-root".to_owned(),
            "cred-new".to_owned(),
            "deadbeef".to_owned(),
        )
        .await
        .unwrap();

        let chain = InvocationChain::try_from(bytes.as_slice()).unwrap();
        chain
            .verify(&dialog_credentials::Ed25519KeyResolver)
            .await
            .unwrap();
        assert_eq!(chain.issuer(), &device_did);
        assert_eq!(chain.subject(), &old_root_did);
        assert_eq!(chain.proofs(), &vec![expected_proof]);
        assert_eq!(
            chain.command().0,
            vec!["account".to_string(), "recover".to_string()],
        );
        assert_eq!(
            chain.arguments().get("newRootDid"),
            Some(&Promised::String("did:key:new-root".to_owned()))
        );
        assert_eq!(
            chain.arguments().get("newCredentialId"),
            Some(&Promised::String("cred-new".to_owned()))
        );
        assert_eq!(
            chain.arguments().get("deviceDelegation"),
            Some(&Promised::String("deadbeef".to_owned()))
        );
    }
}
