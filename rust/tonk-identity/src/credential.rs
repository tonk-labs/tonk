//! Enrolling a second passkey as a peer of the first.
//!
//! A passkey is bound to one authenticator. An account that lives in a
//! single passkey therefore lives on whatever platforms that authenticator
//! reaches — Chrome on a Mac and Safari on an iPhone are routinely not the
//! same set. Enrollment breaks that: the credential derived from a second
//! passkey receives its own subject-open delegation, and every device it
//! grants presents `account → credential → device`.
//!
//! The enrollment link is the same shape as a device grant, deliberately.
//! A peer credential must be able to do everything the credential that
//! enrolled it can do, including enrolling further peers and revoking the
//! link it came through.

use anyhow::Result;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;

use crate::delegation::mint_device_delegation;

/// Mint the `credential → peer` enrollment link.
///
/// `credential` is whichever key is reachable at enrollment time: the
/// account root itself for an account whose subject is its first passkey,
/// or any already-enrolled peer.
pub async fn mint_enrollment(credential: Ed25519Signer, peer: &Did) -> Result<DelegationChain> {
    mint_device_delegation(credential, peer).await
}

/// Extend the enrolling credential's own chain with a new enrollment hop.
///
/// Passing the enroller's chain rather than assuming it is the account root
/// is what lets a peer enrol a further peer: the composed chain still runs
/// from the account subject, so verifiers need no notion of enrolment depth.
pub async fn extend_with_enrollment(
    enroller_chain: &DelegationChain,
    enroller: Ed25519Signer,
    peer: &Did,
) -> Result<DelegationChain> {
    let enrollment = mint_enrollment(enroller, peer).await?;
    let link = enrollment
        .proofs()
        .last()
        .ok_or_else(|| anyhow::anyhow!("enrollment chain has no proof"))?
        .clone();
    enroller_chain
        .push(link)
        .map_err(|err| anyhow::anyhow!("failed to extend the enroller's chain: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::{GrantError, validate_account_grant};
    use dialog_varsig::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const ROOT_PRF: [u8; 32] = [1u8; 32];
    const PEER_PRF: [u8; 32] = [2u8; 32];
    const THIRD_PRF: [u8; 32] = [3u8; 32];
    const DEVICE_SEED: [u8; 32] = [4u8; 32];

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    #[dialog_common::test]
    async fn it_grants_a_peer_credential_the_same_shape_as_a_device() {
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
        let peer = crate::derive::derive_root_signer(&PEER_PRF).await.unwrap();

        let enrollment = mint_enrollment(root, &peer.did()).await.unwrap();

        assert_eq!(*enrollment.audience(), peer.did());
        assert!(
            enrollment.subject().is_none(),
            "a peer credential must be able to act for the account anywhere"
        );
        assert!(enrollment.expiration().is_none());
    }

    #[dialog_common::test]
    async fn it_validates_a_device_granted_by_an_enrolled_peer() {
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
        let root_did = root.did();
        let peer = crate::derive::derive_root_signer(&PEER_PRF).await.unwrap();
        let device = signer(&DEVICE_SEED).await;

        let enrollment = mint_enrollment(root, &peer.did()).await.unwrap();
        let chain = extend_with_enrollment(&enrollment, peer, &device.did())
            .await
            .unwrap();

        let grant = validate_account_grant(&chain, &device.did()).await.unwrap();
        assert_eq!(grant.root_did, root_did, "the account subject is unchanged");
        assert_eq!(grant.device_did, device.did());
        assert_eq!(
            grant.delegation_cid,
            chain.proof_cids()[1],
            "a revocation of this device names the device's own hop"
        );
    }

    #[dialog_common::test]
    async fn it_lets_a_peer_enrol_a_further_peer() {
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
        let root_did = root.did();
        let peer = crate::derive::derive_root_signer(&PEER_PRF).await.unwrap();
        let third = crate::derive::derive_root_signer(&THIRD_PRF).await.unwrap();
        let device = signer(&DEVICE_SEED).await;

        let first = mint_enrollment(root, &peer.did()).await.unwrap();
        let second = extend_with_enrollment(&first, peer, &third.did())
            .await
            .unwrap();
        let chain = extend_with_enrollment(&second, third, &device.did())
            .await
            .unwrap();

        let grant = validate_account_grant(&chain, &device.did()).await.unwrap();
        assert_eq!(grant.root_did, root_did);
        assert_eq!(chain.proof_cids().len(), 3);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_that_ends_at_another_device() {
        let root = crate::derive::derive_root_signer(&ROOT_PRF).await.unwrap();
        let peer = crate::derive::derive_root_signer(&PEER_PRF).await.unwrap();
        let other = signer(&DEVICE_SEED).await;

        let enrollment = mint_enrollment(root, &peer.did()).await.unwrap();

        assert!(matches!(
            validate_account_grant(&enrollment, &other.did()).await,
            Err(GrantError::Audience(_))
        ));
    }
}
