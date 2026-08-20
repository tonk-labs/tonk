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
use dialog_credentials::Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{
    Container, Delegation, DelegationBuilder, DelegationChain, InvocationBuilder, InvocationChain,
};
use dialog_varsig::AnySignature;
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use tonk_account::customer::deposit_scopes;

/// Build a device-signed account-service invocation container.
///
/// `link` is the stored `root → device` delegation: its issuer is the
/// account root (the invocation subject and audience), and its single
/// proof is attached so the service can bind the device to the account.
pub async fn build_device_invocation(
    device: impl Into<Signer>,
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
        .issuer(device.into())
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
/// [`build_device_invocation`] does, and additionally deposits the
/// scoped delegations granting `service` access to its own branch of the
/// account space — the [`deposit_scopes`], nothing broader. The deposits
/// here are device-issued, the fallback for a device holding no
/// ceremony-minted set; the service walks them back to the account
/// through the same `root → device` grant the invocation proves with,
/// which rides in the same container. Prefer
/// [`build_enroll_invocation_with_deposits`] with account-signed
/// deposits when a ceremony produced them.
pub async fn build_enroll_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    service: &Did,
    email: &str,
) -> Result<Vec<u8>> {
    let device: Signer = device.into();
    let root_did = link.issuer().clone();
    let mut deposits = Vec::new();
    for scope in deposit_scopes(&root_did, service) {
        let deposit = DelegationBuilder::new()
            .issuer(device.clone())
            .audience(service)
            .subject(scope.subject.clone())
            .command(scope.command.segments().clone())
            .policy(scope.policy())
            .try_build()
            .await
            .context("failed to mint the access deposit")?;
        deposits.push(deposit);
    }
    let named: Vec<(Cid, Vec<u8>)> = deposits
        .into_iter()
        .map(|deposit| (deposit.to_cid(), deposit.encoded().to_vec()))
        .collect();
    assemble_enroll_container(device, link, email, named).await
}

/// Build a `/customer/enroll` container around externally minted
/// deposits — the account-signed set a passkey ceremony produced. These
/// are issued by the customer directly, so they survive revocation of
/// the device presenting them.
pub async fn build_enroll_invocation_with_deposits(
    device: impl Into<Signer>,
    link: &DelegationChain,
    email: &str,
    deposits: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let named = deposits
        .iter()
        .map(|bytes| {
            let delegation: Delegation<AnySignature> = serde_ipld_dagcbor::from_slice(bytes)
                .context("a ceremony deposit does not decode as a delegation")?;
            Ok((delegation.to_cid(), bytes.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    assemble_enroll_container(device, link, email, named).await
}

/// Assemble the enroll invocation and append the named deposit tokens.
async fn assemble_enroll_container(
    device: impl Into<Signer>,
    link: &DelegationChain,
    email: &str,
    deposits: Vec<(Cid, Vec<u8>)>,
) -> Result<Vec<u8>> {
    let arguments = BTreeMap::from([
        ("email".to_string(), Promised::String(email.to_string())),
        (
            "access".to_string(),
            Promised::List(
                deposits
                    .iter()
                    .map(|(cid, _)| Promised::Link(*cid))
                    .collect(),
            ),
        ),
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
    for (_, bytes) in deposits {
        tokens.push(bytes);
    }
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
    device: impl Into<Signer>,
    link: &DelegationChain,
    consumer: &Did,
    consent: &DelegationChain,
    kind: Option<&str>,
) -> Result<Vec<u8>> {
    let head = consent
        .proofs()
        .next()
        .context("the consent chain carries no delegation")?;
    let mut arguments = BTreeMap::from([
        (
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        ),
        ("consent".to_string(), Promised::Link(head.to_cid())),
    ]);
    if let Some(kind) = kind {
        arguments.insert("kind".to_string(), Promised::String(kind.to_string()));
    }
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

/// Build a `/provider/remove` container — the reverse of
/// [`build_provider_add_invocation`], and how a hosted space is
/// deleted: the invocation names the customer as its subject and the
/// space as its `consumer` argument, proving through the account's own
/// chain. No per-space artifact is deposited.
pub async fn build_provider_remove_invocation(
    device: impl Into<Signer>,
    link: &DelegationChain,
    consumer: &Did,
) -> Result<Vec<u8>> {
    build_device_invocation(
        device,
        link,
        vec!["provider".to_string(), "remove".to_string()],
        BTreeMap::from([(
            "consumer".to_string(),
            Promised::String(consumer.to_string()),
        )]),
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
        let root = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
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
            .verify(&dialog_credentials::DidKeyResolver)
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
    async fn it_carries_ceremony_minted_deposits_issued_by_the_account() {
        let root = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let root_did = root.did();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let service = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let link = crate::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();

        let minted = crate::ceremony::mint_service_deposits(&root, &service.did())
            .await
            .unwrap();
        assert_eq!(minted.len(), 2, "one deposit per scope");
        let deposits: Vec<Vec<u8>> = minted
            .iter()
            .map(|deposit| hex::decode(deposit).unwrap())
            .collect();

        let bytes =
            build_enroll_invocation_with_deposits(device, &link, "a@example.com", &deposits)
                .await
                .unwrap();
        let tokens = Container::from_bytes(&bytes).unwrap().into_tokens();
        // Invocation, the root → device link, and the two deposits.
        assert_eq!(tokens.len(), 4);
        for token in &tokens[2..] {
            let deposit: Delegation<AnySignature> = serde_ipld_dagcbor::from_slice(token).unwrap();
            assert_eq!(
                deposit.issuer(),
                &root_did,
                "deposits are issued by the account itself"
            );
            assert_eq!(deposit.audience(), &service.did());
        }
    }
}
