//! The delegation from an account root down to one device.

use anyhow::Result;
use dialog_credentials::{Ed25519KeyResolver, Ed25519Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;
use ipld_core::cid::Cid;

/// Mint the `root → device` delegation: subject-open, audience-specific —
/// "this device may act as me, for anything". Deliberately the opposite
/// shape from space invites, which are subject-specific and must stay so.
pub async fn mint_device_delegation(root: Ed25519Signer, device: &Did) -> Result<DelegationChain> {
    let delegation = DelegationBuilder::new()
        .issuer(root)
        .audience(device)
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .map_err(|e| anyhow::anyhow!("failed to mint the device delegation: {e}"))?;
    Ok(DelegationChain::new(delegation))
}

/// A validated chain of account authority ending at one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountGrant {
    /// Account root the chain starts from — the account subject.
    pub root_did: Did,
    /// Device the chain ends at.
    pub device_did: Did,
    /// CID of the device's own inbound link, the hop a revocation of this
    /// device names. For a one-hop grant this is the whole chain.
    pub delegation_cid: Cid,
}

/// Why a presented chain is not an account grant.
#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    /// The chain is not shaped like account authority.
    #[error("{0}")]
    Shape(String),
    /// The chain does not end at the expected device.
    #[error("{0}")]
    Audience(String),
    /// A hop's signature failed to verify.
    #[error("{0}")]
    Signature(String),
}

/// Validate a chain running from an account root to `device`.
///
/// One hop is the common case — the root delegating straight to a device —
/// but a device whose credential was enrolled as a peer presents
/// `root → credential → device`, and a peer of a peer presents more. Every
/// hop must keep the account shape: subject-open, command-open, and signed.
pub async fn validate_account_grant(
    chain: &DelegationChain,
    device: &Did,
) -> std::result::Result<AccountGrant, GrantError> {
    if chain.audience() != device {
        return Err(GrantError::Audience(
            "account grant audience is not this device".to_string(),
        ));
    }
    for delegation in chain.proofs() {
        if !delegation.command().0.is_empty() {
            return Err(GrantError::Shape(
                "account grant must be command-open".to_string(),
            ));
        }
        if !matches!(delegation.subject(), UcanSubject::Any) {
            return Err(GrantError::Shape(
                "every hop of an account grant must be subject-open".to_string(),
            ));
        }
        delegation
            .verify_signature(&Ed25519KeyResolver)
            .await
            .map_err(|error| {
                GrantError::Signature(format!("invalid account grant signature: {error}"))
            })?;
    }
    Ok(AccountGrant {
        root_did: chain.issuer().clone(),
        device_did: device.clone(),
        delegation_cid: chain.proof_cids()[chain.proof_cids().len() - 1],
    })
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
            .issuer(issuer)
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
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
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
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
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
