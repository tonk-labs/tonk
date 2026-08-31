//! Checking a `root → device` delegation chain presented at account
//! creation or device linking.

use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::{Delegation, DelegationChain};
use dialog_varsig::AnySignature;

use crate::core::CeremonyError;

/// Why a purported stable root-to-device grant is not canonical.
#[derive(Debug)]
pub(crate) enum DeviceGrantError {
    /// The container did not resolve to exactly one delegation proof.
    ProofCount,
    /// The proof was not issued by the expected account root.
    WrongIssuer,
    /// The proof was not addressed to the expected device.
    WrongAudience,
    /// The proof narrowed the account subject.
    SubjectScoped,
    /// The proof narrowed the commands available to the device.
    CommandScoped,
    /// The durable device grant carried a not-before or expiration bound.
    TimeBounded,
    /// The proof signature did not verify under its issuer.
    InvalidSignature(String),
}

/// Validate the one canonical durable `root → device` grant shape.
///
/// Account creation, device registration, and response-loss setup recovery all
/// call this seam, so none can persist or authenticate a grant another path
/// would later refuse.
pub(crate) async fn validate_root_device_grant(
    proof_count: usize,
    proof: Option<&Delegation<AnySignature>>,
    root_did: &str,
    device_did: &str,
) -> Result<String, DeviceGrantError> {
    if proof_count != 1 {
        return Err(DeviceGrantError::ProofCount);
    }
    let Some(proof) = proof else {
        return Err(DeviceGrantError::ProofCount);
    };
    if proof.issuer().to_string() != root_did {
        return Err(DeviceGrantError::WrongIssuer);
    }
    if proof.audience().to_string() != device_did {
        return Err(DeviceGrantError::WrongAudience);
    }
    if proof.subject() != &Subject::Any {
        return Err(DeviceGrantError::SubjectScoped);
    }
    if !proof.command().0.is_empty() {
        return Err(DeviceGrantError::CommandScoped);
    }
    if proof.not_before().is_some() || proof.expiration().is_some() {
        return Err(DeviceGrantError::TimeBounded);
    }
    proof
        .verify_signature(&DidKeyResolver)
        .await
        .map_err(|error| DeviceGrantError::InvalidSignature(error.to_string()))?;
    Ok(proof.to_cid().to_string())
}

fn creation_error(error: DeviceGrantError) -> CeremonyError {
    let message = match error {
        DeviceGrantError::ProofCount => "delegation chain must have exactly one proof",
        DeviceGrantError::WrongIssuer => "delegation issuer does not match the claimed root",
        DeviceGrantError::WrongAudience => "delegation audience does not match the device",
        DeviceGrantError::SubjectScoped => "root to device delegation must be subject-open",
        DeviceGrantError::CommandScoped => "root to device delegation must be command-open",
        DeviceGrantError::TimeBounded => "root to device delegation must be unbounded",
        DeviceGrantError::InvalidSignature(detail) => {
            return CeremonyError::Unauthorized(format!("bad delegation signature: {detail}"));
        }
    };
    CeremonyError::Invalid(message.to_string())
}

/// Parse and check a hex-encoded `root → device` delegation chain.
///
/// Requires exactly one proof, issued by `root_did` to `device_did`, with a
/// valid signature and the subject-open, command-open, unbounded shape of a
/// durable device grant. Returns the delegation's CID, stringified — the key
/// `devices.delegation_cid` is stored under.
pub async fn check_device_delegation(
    delegation_hex: &str,
    root_did: &str,
    device_did: &str,
) -> Result<String, CeremonyError> {
    let bytes = hex::decode(delegation_hex)
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation hex: {err}")))?;
    let chain = DelegationChain::try_from(&bytes[..])
        .map_err(|err| CeremonyError::Invalid(format!("bad delegation chain: {err}")))?;

    validate_root_device_grant(
        chain.proof_cids().len(),
        chain.proofs().next(),
        root_did,
        device_did,
    )
    .await
    .map_err(creation_error)
}
