//! Checking a `root → device` delegation chain presented at account
//! creation or device linking.

use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::DelegationChain;

use crate::core::CeremonyError;

/// Parse and check a hex-encoded `root → device` delegation chain.
///
/// Requires exactly one proof, issued by `root_did` to `device_did`,
/// subject-open, with a valid signature. Returns the delegation's CID,
/// stringified — the key `devices.delegation_cid` is stored under.
pub async fn check_device_delegation(
    delegation_hex: &str,
    root_did: &str,
    device_did: &str,
) -> Result<String, CeremonyError> {
    let bytes = hex::decode(delegation_hex)
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation hex: {err}")))?;
    let chain = DelegationChain::try_from(&bytes[..])
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation chain: {err}")))?;

    if chain.proof_cids().len() != 1 {
        return Err(CeremonyError::Invalid(
            "delegation chain must have exactly one proof".to_string(),
        ));
    }
    if chain.issuer().to_string() != root_did {
        return Err(CeremonyError::Invalid(
            "delegation issuer does not match the claimed root".to_string(),
        ));
    }
    if chain.audience().to_string() != device_did {
        return Err(CeremonyError::Invalid(
            "delegation audience does not match the device".to_string(),
        ));
    }
    if chain.subject().is_some() {
        return Err(CeremonyError::Invalid(
            "root to device delegation must be subject-open".to_string(),
        ));
    }

    let proof = chain
        .proofs()
        .next()
        .expect("a chain with one proof cid has one proof");
    proof
        .verify_signature(&DidKeyResolver)
        .await
        .map_err(|err| CeremonyError::Unauthorized(format!("bad delegation signature: {err}")))?;

    Ok(chain.proof_cids()[0].to_string())
}
