//! UCAN invocation verification.
//!
//! This module handles:
//! 1. Parsing DAG-CBOR invocations
//! 2. Signature verification
//! 3. Delegation chain validation
//! 4. Time bounds checking

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use ipld_core::cid::Cid;
use serde_ipld_dagcbor;
use ucan::{Delegation, did::Ed25519Did, future::Sendable, invocation::Invocation};

type DelegationStore = Arc<Mutex<HashMap<Cid, Arc<Delegation<Ed25519Did>>>>>;

/// Errors that can occur during verification.
#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("Failed to parse invocation: {0}")]
    ParseError(String),

    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Audience mismatch: expected {expected}, got {got}")]
    AudienceMismatch { expected: String, got: String },

    #[error("Invocation expired")]
    Expired,

    #[error("Delegation chain invalid: {0}")]
    ChainInvalid(String),

    #[error("Capability not authorized: {0}")]
    Unauthorized(String),

    #[error("Proof not found in store: {0}")]
    ProofNotFound(String),

    #[error("Policy check failed: {0}")]
    PolicyFailed(String),
}

/// Result of successful verification.
pub struct VerifiedInvocation {
    /// The command being invoked
    pub command: Vec<String>,

    /// The verified subject (space DID)
    pub subject: String,
}

/// Verify a UCAN invocation.
///
/// This performs complete verification:
/// 1. Parse the DAG-CBOR bytes into an Invocation
/// 2. Verify the Ed25519 signature
/// 3. Check that `aud` matches `sub`
/// 4. Validate time bounds
/// 5. Verify the delegation chain using provided proofs
///
/// Ensures the invocation is addressed to the space it operates on,
/// and the delegation chain proves the invoker has authority from that space.
///
/// # Arguments
///
/// * `cbor_bytes` - The raw DAG-CBOR encoded invocation
/// * `proof_bytes` - DAG-CBOR encoded array of Delegation proofs
///
/// # Returns
///
/// * `Ok(VerifiedInvocation)` - Verification succeeded
/// * `Err(VerificationError)` - Verification failed
pub async fn verify_invocation(
    cbor_bytes: &[u8],
    proof_bytes: &[Vec<u8>],
) -> Result<VerifiedInvocation, VerificationError> {
    // Step 1: Parse the invocation
    let invocation: Invocation<Ed25519Did> = serde_ipld_dagcbor::from_slice(cbor_bytes)
        .map_err(|e| VerificationError::ParseError(e.to_string()))?;

    // Step 2: Check invocation addressed to space
    if invocation.audience() != invocation.subject() {
        return Err(VerificationError::AudienceMismatch {
            expected: invocation.subject().to_string(),
            got: invocation.audience().to_string(),
        });
    }

    // Step 3: Check time bounds
    let now = chrono::Utc::now().timestamp() as u64;

    if let Some(exp) = invocation.expiration()
        && exp.to_unix() <= now
    {
        return Err(VerificationError::Expired);
    }

    // Step 4: Build delegation store from proofs
    let store = build_delegation_store(proof_bytes, now)?;

    // Step 5: Full verification via rs-ucan
    // This checks:
    //   - Signature is valid (issuer signed the invocation)
    //   - Proof chain is valid (issuer->subject chain via proofs)
    //   - Commands are properly attenuated
    //   - Policy predicates pass
    invocation
        .check::<Sendable, _, _>(&store)
        .await
        .map_err(|e| {
            // Convert the library error to our error type
            match e {
                ucan::invocation::InvocationCheckError::SignatureVerification(sig_err) => {
                    VerificationError::InvalidSignature(sig_err.to_string())
                }
                ucan::invocation::InvocationCheckError::StoredCheck(stored_err) => match stored_err
                {
                    ucan::invocation::StoredCheckError::GetError(get_err) => {
                        VerificationError::ProofNotFound(get_err.to_string())
                    }
                    ucan::invocation::StoredCheckError::CheckFailed(check_err) => {
                        map_check_failed(check_err)
                    }
                },
            }
        })?;

    // Step 6: Return verified invocation data
    Ok(VerifiedInvocation {
        command: invocation.command().segments().clone(),
        subject: invocation.subject().to_string(),
    })
}

/// Build an in-memory delegation store from proof bytes.
///
/// This parses each proof, validates time bounds, and inserts into the store.
/// The store is then used by `Invocation::check()` to validate the proof chain.
fn build_delegation_store(
    proof_bytes: &[Vec<u8>],
    now: u64,
) -> Result<DelegationStore, VerificationError> {
    let store: DelegationStore = Arc::new(Mutex::new(HashMap::new()));

    for (i, bytes) in proof_bytes.iter().enumerate() {
        // Parse the delegation
        let delegation: Delegation<Ed25519Did> =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|e| {
                VerificationError::ParseError(format!("Failed to parse proof[{}]: {}", i, e))
            })?;

        // Check time bounds
        if let Some(exp) = delegation.expiration()
            && exp.to_unix() <= now
        {
            return Err(VerificationError::ChainInvalid(format!(
                "Proof[{}] expired",
                i
            )));
        }

        if let Some(nbf) = delegation.not_before()
            && nbf.to_unix() > now
        {
            return Err(VerificationError::ChainInvalid(format!(
                "Proof[{}] not yet valid",
                i
            )));
        }

        // Compute CID and insert into store
        let cid = delegation.to_cid();
        let arc_delegation = Arc::new(delegation);

        store
            .lock()
            .map_err(|_| VerificationError::ChainInvalid("Store lock poisoned".to_string()))?
            .insert(cid, arc_delegation);
    }

    Ok(store)
}

/// Map rs-ucan's CheckFailed error to our VerificationError.
fn map_check_failed(err: ucan::invocation::CheckFailed) -> VerificationError {
    use ucan::invocation::CheckFailed;

    match err {
        CheckFailed::InvalidProofIssuerChain => {
            VerificationError::ChainInvalid("Invalid proof issuer chain".to_string())
        }
        CheckFailed::SubjectNotAllowedByProof => {
            VerificationError::ChainInvalid("Subject not allowed by proof".to_string())
        }
        CheckFailed::RootProofIssuerIsNotSubject => {
            VerificationError::ChainInvalid("Root proof issuer is not the subject".to_string())
        }
        CheckFailed::CommandMismatch { expected, found } => {
            VerificationError::Unauthorized(format!(
                "Command mismatch: expected {:?}, found {:?}",
                expected, found
            ))
        }
        CheckFailed::PredicateFailed(predicate) => {
            VerificationError::PolicyFailed(format!("Predicate failed: {:?}", predicate))
        }
        CheckFailed::PredicateRunError(run_err) => {
            VerificationError::PolicyFailed(format!("Predicate run error: {}", run_err))
        }
        CheckFailed::WaitingOnPromise(waiting) => {
            VerificationError::ChainInvalid(format!("Waiting on promise: {:?}", waiting))
        }
    }
}
