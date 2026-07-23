//! Device-signed account-service invocation containers.
//!
//! The account service's `authorize` accepts requests issued by a device
//! key whose `root → device` delegation is attached as a proof, with the
//! account root as subject. This builds exactly that container from a
//! profile's live device signer and its stored `root → device` link — no
//! root key, no raw seed.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
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
        .try_build()
        .await
        .context("failed to sign the device invocation")?;

    let mut proofs = HashMap::new();
    proofs.insert(cid, Arc::new(delegation));
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .context("failed to serialize the device invocation")
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
}
