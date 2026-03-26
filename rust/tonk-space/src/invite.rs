//! Invite token creation and validation for space collaboration.
//!
//! An invite token grants a specific DID access to one or more spaces.
//! The token is DID-bound (not bearer) — only the intended recipient can
//! use it. Each grant in the token contains a signed UCAN delegation from
//! the space key to the invited DID.
//!
//! Token format: `carry_inv1_<base64url_no_pad(dag-cbor(InviteEnvelopeV1))>`

use crate::delegation::Delegation;
use crate::operator::Operator;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dialog_credentials::Ed25519Signer;
use dialog_ucan::subject::Subject;
use dialog_ucan::time::Timestamp;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Token prefix for v1 invite tokens.
const TOKEN_PREFIX: &str = "carry_inv1_";

/// Default invite lifetime: 7 days in seconds.
const DEFAULT_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Error)]
pub enum InviteError {
    #[error("Invalid token: missing 'carry_inv1_' prefix")]
    MissingPrefix,

    #[error("Invalid token: base64 decode failed: {0}")]
    Base64Decode(#[from] base64::DecodeError),

    #[error("Invalid token: DAG-CBOR decode failed: {0}")]
    CborDecode(String),

    #[error("Invalid token: unsupported version {0}")]
    UnsupportedVersion(u8),

    #[error("Invalid token: wrong kind '{0}', expected 'carry.invite'")]
    WrongKind(String),

    #[error("Grant verification failed: audience DID mismatch (expected {expected}, got {got})")]
    AudienceMismatch { expected: String, got: String },

    #[error("Grant verification failed: subject must be Specific, not Any")]
    SubjectIsAny,

    #[error("Grant verification failed: subject DID mismatch (expected {expected}, got {got})")]
    SubjectMismatch { expected: String, got: String },

    #[error("Grant verification failed: delegation expired")]
    Expired,

    #[error("Grant verification failed: delegation not yet valid")]
    NotYetValid,

    #[error("Grant verification failed: invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Grant verification failed: issuer DID mismatch (expected {expected}, got {got})")]
    IssuerMismatch { expected: String, got: String },

    #[error("Delegation build error: {0}")]
    DelegationBuild(String),

    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Invalid DID: {0}")]
    InvalidDid(String),

    #[error("No grants in invite token")]
    EmptyGrants,
}

/// V1 invite envelope — the top-level structure serialized into the token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteEnvelopeV1 {
    /// Version number (must be 1).
    pub v: u8,
    /// Token kind discriminator.
    pub kind: String,
    /// The DID this invite is addressed to.
    pub invited: String,
    /// Unix timestamp when the invite was created.
    pub issued_at: u64,
    /// Hint: DID of the user who created the invite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_hint: Option<String>,
    /// Hint: repo label for display purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_hint: Option<String>,
    /// One grant per space being shared.
    pub grants: Vec<InviteGrantV1>,
}

/// A single space grant within an invite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteGrantV1 {
    /// The space DID this grant applies to.
    pub space: String,
    /// Base64url-no-pad encoded DAG-CBOR of the signed UCAN delegation.
    pub delegation_b64u: String,
    /// The ability being granted (e.g., "/").
    pub ability: String,
    /// Earliest valid time (unix seconds), if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    /// Expiration time (unix seconds).
    pub exp: u64,
}

impl InviteGrantV1 {
    /// Decode the embedded delegation from base64url DAG-CBOR.
    pub fn delegation(&self) -> Result<Delegation, InviteError> {
        let bytes = URL_SAFE_NO_PAD.decode(&self.delegation_b64u)?;
        let delegation: Delegation = serde_ipld_dagcbor::from_slice(&bytes)
            .map_err(|e| InviteError::CborDecode(e.to_string()))?;
        Ok(delegation)
    }
}

// ---------------------------------------------------------------------------
// Encode / Decode
// ---------------------------------------------------------------------------

/// Encode an invite envelope into a `carry_inv1_...` token string.
pub fn encode_invite(envelope: &InviteEnvelopeV1) -> Result<String, InviteError> {
    let cbor =
        serde_ipld_dagcbor::to_vec(envelope).map_err(|e| InviteError::Encoding(e.to_string()))?;
    let b64 = URL_SAFE_NO_PAD.encode(&cbor);
    Ok(format!("{}{}", TOKEN_PREFIX, b64))
}

/// Decode a `carry_inv1_...` token string into an invite envelope.
pub fn decode_invite(token: &str) -> Result<InviteEnvelopeV1, InviteError> {
    let payload = token
        .strip_prefix(TOKEN_PREFIX)
        .ok_or(InviteError::MissingPrefix)?;
    let cbor = URL_SAFE_NO_PAD.decode(payload)?;
    let envelope: InviteEnvelopeV1 = serde_ipld_dagcbor::from_slice(&cbor)
        .map_err(|e| InviteError::CborDecode(e.to_string()))?;

    if envelope.v != 1 {
        return Err(InviteError::UnsupportedVersion(envelope.v));
    }
    if envelope.kind != "carry.invite" {
        return Err(InviteError::WrongKind(envelope.kind));
    }
    if envelope.grants.is_empty() {
        return Err(InviteError::EmptyGrants);
    }

    Ok(envelope)
}

// ---------------------------------------------------------------------------
// Grant creation
// ---------------------------------------------------------------------------

/// Create a delegation grant for a single space.
///
/// Signs a UCAN delegation: `issuer=space_operator, audience=invited,
/// subject=Specific(space_did), command=/`.
pub async fn create_space_grant(
    space_operator: &Operator,
    space_did: &Did,
    invited: &Did,
    exp_unix: u64,
    nbf_unix: Option<u64>,
) -> Result<(InviteGrantV1, Delegation), InviteError> {
    let signer = Ed25519Signer::from(space_operator);

    let exp_ts = Timestamp::try_from(exp_unix as i128)
        .map_err(|e| InviteError::DelegationBuild(format!("invalid expiration: {}", e)))?;

    let mut builder = Delegation::builder()
        .issuer(signer)
        .audience(invited)
        .subject(Subject::Specific(space_did.clone()))
        .command(Vec::new()) // empty = root command "/"
        .expiration(exp_ts);

    if let Some(nbf) = nbf_unix {
        let nbf_ts = Timestamp::try_from(nbf as i128)
            .map_err(|e| InviteError::DelegationBuild(format!("invalid not_before: {}", e)))?;
        builder = builder.not_before(nbf_ts);
    }

    let ucan_delegation = builder
        .try_build()
        .await
        .map_err(|e| InviteError::DelegationBuild(e.to_string()))?;

    let delegation = Delegation::from(ucan_delegation);
    let delegation_bytes = delegation.to_bytes();
    let delegation_b64u = URL_SAFE_NO_PAD.encode(&delegation_bytes);

    let grant = InviteGrantV1 {
        space: space_did.to_string(),
        delegation_b64u,
        ability: delegation.command().to_string(),
        nbf: nbf_unix,
        exp: exp_unix,
    };

    Ok((grant, delegation))
}

/// Convenience: create a full invite envelope for a single space with default
/// lifetime (7 days).
pub async fn create_invite(
    space_operator: &Operator,
    space_did: &Did,
    invited: &Did,
    repo_hint: Option<String>,
) -> Result<(InviteEnvelopeV1, Delegation), InviteError> {
    let now = Timestamp::now().to_unix();
    let exp = now + DEFAULT_LIFETIME_SECS;

    let (grant, delegation) =
        create_space_grant(space_operator, space_did, invited, exp, Some(now)).await?;

    let envelope = InviteEnvelopeV1 {
        v: 1,
        kind: "carry.invite".to_string(),
        invited: invited.to_string(),
        issued_at: now,
        issuer_hint: Some(space_operator.did().to_string()),
        repo_hint,
        grants: vec![grant],
    };

    Ok((envelope, delegation))
}

// ---------------------------------------------------------------------------
// Grant verification
// ---------------------------------------------------------------------------

/// Verify a single grant against the expected invited DID and current time.
///
/// Checks:
/// - Delegation deserializes and has a valid cryptographic signature
/// - Issuer matches the grant's space DID (only the space key can delegate)
/// - Audience matches the expected invited DID
/// - Subject is Specific (not Any) and matches the grant's space field
/// - Time bounds are valid
pub async fn verify_grant(
    grant: &InviteGrantV1,
    invited: &Did,
    now_unix: u64,
) -> Result<Delegation, InviteError> {
    let delegation = grant.delegation()?;

    // Verify cryptographic signature before trusting any fields
    delegation
        .verify_signature()
        .await
        .map_err(|e| InviteError::InvalidSignature(e.to_string()))?;

    // Issuer must match the space DID — only the space key can delegate for itself
    if delegation.issuer().to_string() != grant.space {
        return Err(InviteError::IssuerMismatch {
            expected: grant.space.clone(),
            got: delegation.issuer().to_string(),
        });
    }

    // Audience must match invited DID
    if delegation.audience().to_string() != invited.to_string() {
        return Err(InviteError::AudienceMismatch {
            expected: invited.to_string(),
            got: delegation.audience().to_string(),
        });
    }

    // Subject must be Specific, not Any
    match delegation.subject() {
        Subject::Specific(did) => {
            if did.to_string() != grant.space {
                return Err(InviteError::SubjectMismatch {
                    expected: grant.space.clone(),
                    got: did.to_string(),
                });
            }
        }
        Subject::Any => {
            return Err(InviteError::SubjectIsAny);
        }
    }

    // Time validation
    let now_ts = Timestamp::try_from(now_unix as i128)
        .map_err(|e| InviteError::DelegationBuild(format!("invalid timestamp: {}", e)))?;
    delegation.validate(now_ts).map_err(|e| match e {
        crate::DelegationError::Expired => InviteError::Expired,
        crate::DelegationError::NotYetValid => InviteError::NotYetValid,
        other => InviteError::DelegationBuild(other.to_string()),
    })?;

    Ok(delegation)
}

/// Verify all grants in an envelope.
pub async fn verify_envelope(
    envelope: &InviteEnvelopeV1,
    now_unix: u64,
) -> Result<Vec<Delegation>, InviteError> {
    let invited: Did = envelope
        .invited
        .parse()
        .map_err(|e| InviteError::InvalidDid(format!("{:?}", e)))?;

    let mut delegations = Vec::with_capacity(envelope.grants.len());
    for grant in &envelope.grants {
        delegations.push(verify_grant(grant, &invited, now_unix).await?);
    }
    Ok(delegations)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Operator;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn encode_decode_roundtrip() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();

        let (envelope, _delegation) = create_invite(
            &space_op,
            &space_op.did(),
            &invited_op.did(),
            Some("my-repo".to_string()),
        )
        .await
        .expect("create_invite should succeed");

        let token = encode_invite(&envelope).expect("encode should succeed");
        assert!(token.starts_with(TOKEN_PREFIX));

        let decoded = decode_invite(&token).expect("decode should succeed");
        assert_eq!(decoded.v, 1);
        assert_eq!(decoded.kind, "carry.invite");
        assert_eq!(decoded.invited, invited_op.did().to_string());
        assert_eq!(decoded.grants.len(), 1);
        assert_eq!(decoded.grants[0].space, space_op.did().to_string());
        assert_eq!(decoded.repo_hint.as_deref(), Some("my-repo"));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn grant_has_correct_delegation_fields() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        let (grant, delegation) = create_space_grant(
            &space_op,
            &space_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .expect("create_space_grant should succeed");

        assert_eq!(delegation.issuer().to_string(), space_op.did().to_string());
        assert_eq!(
            delegation.audience().to_string(),
            invited_op.did().to_string()
        );
        match delegation.subject() {
            Subject::Specific(did) => {
                assert_eq!(did.to_string(), space_op.did().to_string());
            }
            Subject::Any => panic!("Expected specific subject"),
        }
        assert_eq!(grant.exp, exp);
        assert_eq!(grant.nbf, Some(now));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_succeeds_for_valid_grant() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        let (grant, _) = create_space_grant(
            &space_op,
            &space_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        let result = verify_grant(&grant, &invited_op.did(), now).await;
        assert!(result.is_ok());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_rejects_wrong_audience() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();
        let wrong_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        let (grant, _) = create_space_grant(
            &space_op,
            &space_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        let result = verify_grant(&grant, &wrong_op.did(), now).await;
        assert!(matches!(result, Err(InviteError::AudienceMismatch { .. })));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_rejects_wrong_issuer() {
        // An attacker signs a delegation with their own key but claims it's
        // for a different space — the signature is valid but the issuer doesn't
        // match the space DID in the grant.
        let attacker_op = Operator::generate();
        let real_space_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        // Attacker creates a grant using their own key as issuer, but
        // the grant's space field points to the real space DID.
        let (mut grant, _) = create_space_grant(
            &attacker_op,
            &attacker_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        // Overwrite the space field to point to the real space
        grant.space = real_space_op.did().to_string();

        let result = verify_grant(&grant, &invited_op.did(), now).await;
        assert!(
            matches!(result, Err(InviteError::IssuerMismatch { .. })),
            "Expected IssuerMismatch, got: {:?}",
            result
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_rejects_expired() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 10; // expires in 10 seconds

        let (grant, _) =
            create_space_grant(&space_op, &space_op.did(), &invited_op.did(), exp, None)
                .await
                .unwrap();

        // Verify at a time after expiration
        let future = exp + 100;
        let result = verify_grant(&grant, &invited_op.did(), future).await;
        assert!(matches!(result, Err(InviteError::Expired)));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_rejects_not_yet_valid() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let nbf = now + 3600; // not valid for another hour
        let exp = now + 7200;

        let (grant, _) = create_space_grant(
            &space_op,
            &space_op.did(),
            &invited_op.did(),
            exp,
            Some(nbf),
        )
        .await
        .unwrap();

        // Verify at current time (before nbf)
        let result = verify_grant(&grant, &invited_op.did(), now).await;
        assert!(matches!(result, Err(InviteError::NotYetValid)));
    }

    #[test]
    fn decode_rejects_missing_prefix() {
        let result = decode_invite("not_a_valid_token");
        assert!(matches!(result, Err(InviteError::MissingPrefix)));
    }

    #[test]
    fn decode_rejects_invalid_base64() {
        let result = decode_invite("carry_inv1_!!!not-base64!!!");
        assert!(matches!(result, Err(InviteError::Base64Decode(_))));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn multi_grant_roundtrip() {
        let space1_op = Operator::generate();
        let space2_op = Operator::generate();
        let invited_op = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        let (grant1, _) = create_space_grant(
            &space1_op,
            &space1_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        let (grant2, _) = create_space_grant(
            &space2_op,
            &space2_op.did(),
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        let envelope = InviteEnvelopeV1 {
            v: 1,
            kind: "carry.invite".to_string(),
            invited: invited_op.did().to_string(),
            issued_at: now,
            issuer_hint: None,
            repo_hint: None,
            grants: vec![grant1, grant2],
        };

        let token = encode_invite(&envelope).unwrap();
        let decoded = decode_invite(&token).unwrap();
        assert_eq!(decoded.grants.len(), 2);
        assert_eq!(decoded.grants[0].space, space1_op.did().to_string());
        assert_eq!(decoded.grants[1].space, space2_op.did().to_string());

        // Verify all grants
        let delegations = verify_envelope(&decoded, now).await.unwrap();
        assert_eq!(delegations.len(), 2);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn verify_grant_rejects_subject_mismatch() {
        let space_op = Operator::generate();
        let invited_op = Operator::generate();
        let other = Operator::generate();

        let now = Timestamp::now().to_unix();
        let exp = now + 3600;

        // Create a grant where the delegation's subject differs from the
        // grant's space field, but the issuer still matches grant.space.
        // We do this by passing a different space_did to create_space_grant.
        let (mut grant, _) = create_space_grant(
            &space_op,
            &other.did(), // subject will be other's DID
            &invited_op.did(),
            exp,
            Some(now),
        )
        .await
        .unwrap();

        // Set grant.space to match the issuer (passes issuer check)
        // but now it mismatches the delegation's subject (other.did())
        grant.space = space_op.did().to_string();

        let result = verify_grant(&grant, &invited_op.did(), now).await;
        assert!(matches!(result, Err(InviteError::SubjectMismatch { .. })));
    }
}
