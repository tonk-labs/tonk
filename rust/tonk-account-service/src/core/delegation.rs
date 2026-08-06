//! Checking the account-authority chain a device presents at account
//! creation or device linking.

use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use tonk_identity::delegation::{GrantError, validate_account_grant};

use crate::core::CeremonyError;

/// Parse and check a hex-encoded chain from `root_did` to `device_did`.
///
/// One hop is the common case; a device whose passkey was enrolled as a
/// peer of another presents `root → credential → device`. Every hop must be
/// subject-open, command-open and signed. Returns the CID of the device's
/// own inbound hop — the key `devices.delegation_cid` is stored under, and
/// the one a revocation of this device names.
pub async fn check_device_delegation(
    delegation_hex: &str,
    root_did: &str,
    device_did: &str,
) -> Result<String, CeremonyError> {
    let bytes = hex::decode(delegation_hex)
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation hex: {err}")))?;
    let chain = DelegationChain::try_from(&bytes[..])
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation chain: {err}")))?;
    let device: Did = device_did
        .parse()
        .map_err(|err| CeremonyError::Invalid(format!("bad device DID: {err}")))?;

    if chain.issuer().to_string() != root_did {
        return Err(CeremonyError::Invalid(
            "delegation issuer does not match the claimed root".to_string(),
        ));
    }

    let grant = validate_account_grant(&chain, &device)
        .await
        .map_err(|err| match err {
            GrantError::Signature(message) => {
                CeremonyError::Unauthorized(format!("bad delegation signature: {message}"))
            }
            other => CeremonyError::Invalid(other.to_string()),
        })?;

    Ok(grant.delegation_cid.to_string())
}

// Native only: a Worker exports `fetch`, which the wasm-bindgen harness
// shadows when it loads the module, so no test in this crate runs under wasm.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use tonk_identity::credential::{extend_with_enrollment, mint_enrollment};
    use tonk_identity::delegation::mint_device_delegation;

    use super::*;

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    #[dialog_common::test]
    async fn it_accepts_a_one_hop_root_grant() {
        let root = signer(1).await;
        let device = signer(2).await;
        let chain = mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();

        let cid = check_device_delegation(
            &hex::encode(chain.to_bytes().unwrap()),
            root.did().as_ref(),
            device.did().as_ref(),
        )
        .await
        .unwrap();

        assert_eq!(cid, chain.proof_cids()[0].to_string());
    }

    #[dialog_common::test]
    async fn it_accepts_a_device_granted_by_an_enrolled_peer_credential() {
        let root = signer(1).await;
        let peer = signer(3).await;
        let device = signer(2).await;
        let enrollment = mint_enrollment(root.clone(), &peer.did()).await.unwrap();
        let chain = extend_with_enrollment(&enrollment, peer, &device.did())
            .await
            .unwrap();

        let cid = check_device_delegation(
            &hex::encode(chain.to_bytes().unwrap()),
            root.did().as_ref(),
            device.did().as_ref(),
        )
        .await
        .unwrap();

        assert_eq!(
            cid,
            chain.proof_cids()[1].to_string(),
            "the registry records the device's own hop, not the enrollment"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_rooted_at_another_account() {
        let root = signer(1).await;
        let other_root = signer(9).await;
        let device = signer(2).await;
        let chain = mint_device_delegation(other_root, &device.did())
            .await
            .unwrap();

        assert!(matches!(
            check_device_delegation(
                &hex::encode(chain.to_bytes().unwrap()),
                root.did().as_ref(),
                device.did().as_ref(),
            )
            .await,
            Err(CeremonyError::Invalid(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_that_stops_short_of_the_device() {
        let root = signer(1).await;
        let peer = signer(3).await;
        let device = signer(2).await;
        let enrollment = mint_enrollment(root.clone(), &peer.did()).await.unwrap();

        assert!(matches!(
            check_device_delegation(
                &hex::encode(enrollment.to_bytes().unwrap()),
                root.did().as_ref(),
                device.did().as_ref(),
            )
            .await,
            Err(CeremonyError::Invalid(_))
        ));
    }
}
