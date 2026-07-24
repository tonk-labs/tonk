//! Signed device revocations.
//!
//! A registry row saying `status = 'revoked'` is not a revocation; it
//! is a status flag whose authority is "whoever can write that
//! database". A revocation proper is a signed statement naming the
//! delegation it withdraws, which anyone can verify against the account
//! root without holding a database credential.
//!
//! Authority scales with blast radius. A device may revoke *itself*
//! with its own key — always available, and a stolen device can only
//! sign itself out. Revoking *another* device takes the root, so a
//! stolen device cannot lock out its siblings. The artifact records
//! whichever authority was used; a consumer that demands root
//! attestation is making a policy choice, not detecting a defect.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::{Did, Principal};

/// The command a revocation invokes.
pub const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// The argument naming the withdrawn delegation.
pub const REVOKE_ARGUMENT: &str = "revoke";

fn arguments(delegation_cid: &str) -> BTreeMap<String, Promised> {
    let mut arguments = BTreeMap::new();
    arguments.insert(
        REVOKE_ARGUMENT.to_string(),
        Promised::String(delegation_cid.to_string()),
    );
    arguments
}

fn command() -> Vec<String> {
    REVOKE_COMMAND
        .iter()
        .map(|part| (*part).to_string())
        .collect()
}

/// Mint a root-signed revocation of `delegation_cid`.
///
/// The CID is the stringified `root → device` delegation the registry
/// recorded at link time, exactly as `/devices/list` reports it.
///
/// The issuer is the account root and the subject is itself, so the
/// invocation needs no proof: a valid signature is proof of control.
/// This is the attestation a stolen device cannot forge, and the one
/// required to revoke a device other than yourself.
pub async fn mint_root_revocation(root: Ed25519Signer, delegation_cid: &str) -> Result<Vec<u8>> {
    let root_did = root.did();
    let invocation = InvocationBuilder::new()
        .issuer(root)
        .audience(&root_did)
        .subject(&root_did)
        .command(command())
        .arguments(arguments(delegation_cid))
        .proofs(Vec::new())
        .try_build()
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;

    InvocationChain::new(invocation, HashMap::new())
        .to_bytes()
        .map_err(|err| anyhow::anyhow!("failed to serialize the revocation: {err}"))
}

/// Mint a device-signed revocation of the device's own grant.
///
/// The device signs under the `root → device` grant it already holds,
/// which it carries as the invocation's proof so a verifier can see the
/// authority used. This is "sign this account out of this device", and
/// it works offline with no prompt.
pub async fn mint_self_revocation(
    device: Ed25519Signer,
    grant: &DelegationChain,
    root: &Did,
) -> Result<Vec<u8>> {
    let delegation_cid = grant
        .proof_cids()
        .last()
        .ok_or_else(|| anyhow::anyhow!("grant has no proof to revoke"))?
        .to_string();

    let invocation = InvocationBuilder::new()
        .issuer(device)
        .audience(root)
        .subject(root)
        .command(command())
        .arguments(arguments(&delegation_cid))
        .proofs(grant.proof_cids().to_vec())
        .try_build()
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the self-revocation: {err}"))?;

    let proofs = grant.export().collect::<HashMap<_, _>>();
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .map_err(|err| anyhow::anyhow!("failed to serialize the self-revocation: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const ROOT_PRF: [u8; 32] = [1u8; 32];
    const DEVICE_SEED: [u8; 32] = [2u8; 32];

    async fn grant() -> (Ed25519Signer, Ed25519Signer, DelegationChain) {
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let chain = crate::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        (root, device, chain)
    }

    fn parse(bytes: &[u8]) -> InvocationChain<dialog_varsig::algorithm::eddsa::Ed25519Signature> {
        InvocationChain::try_from(bytes).unwrap()
    }

    #[dialog_common::test]
    async fn it_mints_a_root_signed_revocation_naming_the_delegation() {
        let (root, _, chain) = grant().await;
        let cid = chain.proof_cids()[0].to_string();

        let bytes = mint_root_revocation(root.clone(), &cid).await.unwrap();

        let parsed = parse(&bytes);
        assert_eq!(*parsed.issuer(), root.did());
        assert_eq!(parsed.command().0, command());
        assert_eq!(
            parsed.arguments().get(REVOKE_ARGUMENT),
            Some(&dialog_ucan_core::promise::Promised::String(
                cid.to_string()
            ))
        );
    }

    #[dialog_common::test]
    async fn it_mints_a_device_signed_self_revocation() {
        let (root, device, chain) = grant().await;
        let cid = chain.proof_cids()[0].to_string();

        let bytes = mint_self_revocation(device.clone(), &chain, &root.did())
            .await
            .unwrap();

        let parsed = parse(&bytes);
        assert_eq!(*parsed.issuer(), device.did());
        assert_eq!(
            parsed.arguments().get(REVOKE_ARGUMENT),
            Some(&dialog_ucan_core::promise::Promised::String(
                cid.to_string()
            )),
            "a self-revocation names the device's own grant"
        );
        assert!(
            !parsed.proofs().is_empty(),
            "the grant must ride along so a verifier sees the authority used"
        );
    }

    #[dialog_common::test]
    async fn it_distinguishes_a_root_signed_revocation_from_a_self_revocation() {
        let (root, device, chain) = grant().await;
        let cid = chain.proof_cids()[0].to_string();

        let by_root = parse(&mint_root_revocation(root.clone(), &cid).await.unwrap());
        let by_device = parse(
            &mint_self_revocation(device.clone(), &chain, &root.did())
                .await
                .unwrap(),
        );

        assert_eq!(*by_root.issuer(), root.did());
        assert_eq!(*by_device.issuer(), device.did());
        assert_ne!(
            by_root.issuer(),
            by_device.issuer(),
            "attestation level must be readable from the issuer alone"
        );
    }
}
