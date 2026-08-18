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
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{
    Container, DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain,
};
use dialog_varsig::Did;

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

/// Build a `/customer/enroll` container for the access service.
///
/// The invocation is device-signed on the account's subject, exactly as
/// [`build_device_invocation`] does, and additionally deposits a
/// delegation granting `service` access to the account space. The
/// deposit is device-issued; the service walks it back to the account
/// through the same `root → device` grant the invocation proves with,
/// which rides in the same container.
pub async fn build_enroll_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    service: &Did,
    email: &str,
) -> Result<Vec<u8>> {
    let root_did = link.issuer().clone();
    let deposit = DelegationBuilder::new()
        .issuer(device.clone())
        .audience(service)
        .subject(Subject::Specific(root_did))
        .command(vec![])
        .try_build()
        .await
        .context("failed to mint the access deposit")?;
    let arguments = BTreeMap::from([
        ("email".to_string(), Promised::String(email.to_string())),
        ("access".to_string(), Promised::Link(deposit.to_cid())),
    ]);
    let invocation = build_device_invocation(
        device,
        link,
        vec!["customer".to_string(), "enroll".to_string()],
        arguments,
    )
    .await?;
    let mut tokens = Container::from_bytes(&invocation)
        .context("failed to reopen the enroll container")?
        .into_tokens();
    tokens.push(deposit.encoded().to_vec());
    Container::new(tokens)
        .to_bytes()
        .context("failed to encode the enroll container")
}

/// Build a `/provider/add` container for the access service.
///
/// The invocation is device-signed on the account's subject, and the
/// space's consent chain — its powerline to the account — is deposited
/// alongside, named by the CID of its head. The server walks the consent
/// from the consumer to the invoking customer.
pub async fn build_provider_add_invocation(
    device: Ed25519Signer,
    link: &DelegationChain,
    consumer: &Did,
    consent: &DelegationChain,
) -> Result<Vec<u8>> {
    let head = consent
        .proofs()
        .next()
        .context("the consent chain carries no delegation")?;
    let arguments = BTreeMap::from([
        (
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        ),
        ("consent".to_string(), Promised::Link(head.to_cid())),
    ]);
    let invocation = build_device_invocation(
        device,
        link,
        vec!["provider".to_string(), "add".to_string()],
        arguments,
    )
    .await?;
    let mut tokens = Container::from_bytes(&invocation)
        .context("failed to reopen the add container")?
        .into_tokens();
    for delegation in consent.proofs() {
        tokens.push(delegation.encoded().to_vec());
    }
    Container::new(tokens)
        .to_bytes()
        .context("failed to encode the add container")
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
}
