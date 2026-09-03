//! The `root → device` delegation.

use anyhow::Result;
use dialog_credentials::Signer;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;
use tonk_invite::home_address_meta;
use url::Url;

/// Mint the `root → device` delegation: subject-open, audience-specific —
/// "this device may act as me, for anything". Deliberately the opposite
/// shape from space invites, which are subject-specific and must stay so.
pub async fn mint_device_delegation(
    root: impl Into<Signer>,
    device: &Did,
) -> Result<DelegationChain> {
    mint(root.into(), device, None).await
}

/// [`mint_device_delegation`], naming the sync endpoint the account is
/// served from in the delegation's `meta`.
///
/// The address rides inside the signed payload, so wherever the grant
/// travels — a callback handoff, a stored credential — the endpoint
/// travels with it and cannot be swapped independently. Read it back with
/// [`tonk_invite::home_address`].
pub async fn mint_addressed_device_delegation(
    root: impl Into<Signer>,
    device: &Did,
    remote: &Url,
) -> Result<DelegationChain> {
    mint(root.into(), device, Some(remote)).await
}

async fn mint(root: Signer, device: &Did, remote: Option<&Url>) -> Result<DelegationChain> {
    let mut builder = DelegationBuilder::new()
        .issuer(root)
        .audience(device)
        .subject(UcanSubject::Any)
        .command(vec![]);
    if let Some(remote) = remote {
        builder = builder.meta(home_address_meta(remote));
    }
    let delegation = builder
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the device delegation: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_varsig::principal::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const ROOT_PRF: [u8; 32] = [1u8; 32];
    const DEVICE_SEED: [u8; 32] = [2u8; 32];
    const SPACE_SEED: [u8; 32] = [3u8; 32];

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    /// One-hop chain `issuer → audience` scoped to `subject` — the shape
    /// of a space delegating to a root identity.
    async fn space_chain(issuer: Ed25519Signer, audience: &Did, subject: &Did) -> DelegationChain {
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    #[dialog_common::test]
    async fn it_mints_a_subject_open_delegation_to_the_device() {
        let root = Ed25519Signer::import(&ROOT_PRF).await.unwrap();
        let device = signer(&DEVICE_SEED).await;
        let chain = mint_device_delegation(root, &device.did()).await.unwrap();
        assert_eq!(*chain.audience(), device.did());
        assert!(
            chain.subject().is_none(),
            "root → device must be subject-open"
        );
        assert_eq!(chain.proof_cids().len(), 1);
    }

    #[dialog_common::test]
    async fn it_extends_a_space_chain_through_the_root_to_the_device() {
        let space = signer(&SPACE_SEED).await;
        let space_did = space.did();
        let root = Ed25519Signer::import(&ROOT_PRF).await.unwrap();
        let root_did = root.did();
        let device = signer(&DEVICE_SEED).await;

        // space → root, scoped to the space itself.
        let chain = space_chain(space, &root_did, &space_did).await;

        // root → device is minted independently and subject-open; pushed
        // onto the space chain it must still verify, with the space
        // subject carried through.
        let device_link = mint_device_delegation(root, &device.did()).await.unwrap();
        let device_delegation = device_link.proofs().last().unwrap().clone();
        let composed = chain.push(device_delegation).unwrap();

        assert_eq!(*composed.audience(), device.did());
        assert_eq!(composed.subject(), Some(&space_did));
        assert_eq!(composed.proof_cids().len(), 2);
    }
}
