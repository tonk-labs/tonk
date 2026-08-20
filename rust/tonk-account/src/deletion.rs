//! Capability contracts for irreversible hosted-space deletion.

use std::collections::HashMap;

use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::time::timestamp::Timestamp;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::Did;

/// The only command an irreversible hosted-space deletion grant may carry.
pub const SPACE_DELETE_COMMAND: [&str; 2] = ["space", "delete"];

/// A verified, direct grant from one space to its creator account.
#[derive(Debug)]
pub struct SpaceDeletionGrant {
    /// Repository subject this grant permits deleting.
    pub space: Did,
    /// Account root that may exercise the grant.
    pub owner: Did,
    /// Human-readable exact ability, useful in receipts and diagnostics.
    pub command: &'static str,
    /// CID of the exact delegation the access service registers.
    pub cid: String,
    /// Verified one-proof chain, retained for invocation construction.
    pub chain: DelegationChain,
}

/// Stable reasons a candidate deletion grant is not creator authority.
#[derive(Debug, thiserror::Error)]
pub enum SpaceDeletionGrantError {
    /// Bytes did not decode as a delegation chain.
    #[error("deletion grant container is invalid: {0}")]
    InvalidChain(String),
    /// The grant must contain exactly one direct proof.
    #[error("deletion grant must be one direct space-to-account proof")]
    Indirect,
    /// The proof issuer was not the repository being deleted.
    #[error("deletion grant was not issued by the space")]
    WrongIssuer,
    /// The proof subject was not the repository being deleted.
    #[error("deletion grant is not scoped to the space")]
    WrongSubject,
    /// The proof audience was not the creator account root.
    #[error("deletion grant was not issued to this account")]
    WrongOwner,
    /// Broad grants are deliberately insufficient for destructive authority.
    #[error("deletion grant must carry exactly /space/delete")]
    WrongCommand,
    /// The grant signature did not verify against its space DID.
    #[error("deletion grant signature is invalid: {0}")]
    InvalidSignature(String),
    /// The grant is expired or not active yet.
    #[error("deletion grant is not currently valid: {0}")]
    NotCurrentlyValid(String),
}

/// Failure to construct the root-signed deletion invocation.
#[derive(Debug, thiserror::Error)]
pub enum DeletionInvocationError {
    #[error("deletion grant is invalid: {0}")]
    Grant(#[from] SpaceDeletionGrantError),
    #[error("deletion invocation signer is not the deletion-grant owner")]
    WrongRoot,
    #[error("failed to sign deletion invocation: {0}")]
    Build(String),
    #[error("failed to encode deletion invocation: {0}")]
    Encode(String),
}

/// Mint the non-expiring exact deletion grant while the space signer exists.
pub async fn mint_deletion_grant(
    space: &dialog_credentials::Signer,
    owner: &Did,
) -> Result<DelegationChain, dialog_ucan_core::delegation::builder::BuildError> {
    use dialog_ucan_core::DelegationBuilder;
    use dialog_varsig::Principal as _;

    let space_did = space.did();
    let delegation = DelegationBuilder::new()
        .issuer(space.clone())
        .audience(owner)
        .subject(Subject::Specific(space_did))
        .command(
            SPACE_DELETE_COMMAND
                .iter()
                .map(ToString::to_string)
                .collect(),
        )
        .try_build()
        .await?;
    Ok(DelegationChain::new(delegation))
}

/// Build a short-lived root-signed deletion invocation. Calling this is a
/// passkey/root ceremony; ordinary device authority is deliberately
/// insufficient for irreversible deletion.
pub async fn build_deletion_invocation(
    root: dialog_credentials::Ed25519Signer,
    deletion_grant: &DelegationChain,
) -> Result<Vec<u8>, DeletionInvocationError> {
    use dialog_varsig::Principal as _;

    let space = deletion_grant.issuer().clone();
    let owner = deletion_grant.audience().clone();
    validate_deletion_grant(
        &deletion_grant
            .to_bytes()
            .map_err(|error| DeletionInvocationError::Encode(error.to_string()))?,
        &space,
        &owner,
    )
    .await?;

    if root.did() != owner {
        return Err(DeletionInvocationError::WrongRoot);
    }
    let grant_cid = *deletion_grant
        .proof_cids()
        .first()
        .ok_or(DeletionInvocationError::Grant(
            SpaceDeletionGrantError::Indirect,
        ))?;
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&space)
        .subject(&space)
        .command(
            SPACE_DELETE_COMMAND
                .iter()
                .map(ToString::to_string)
                .collect(),
        )
        .proofs(vec![grant_cid])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .map_err(|error| DeletionInvocationError::Build(error.to_string()))?;
    let proofs: HashMap<_, _> = deletion_grant.export().collect();
    InvocationChain::new(invocation, proofs)
        .to_bytes()
        .map_err(|error| DeletionInvocationError::Encode(error.to_string()))
}

/// Build the one-time compatibility invocation for an original direct broad
/// `space -> account-root` proof. Services may accept this only when that
/// exact proof CID was registered as `legacy-direct`; indirect invite chains
/// are always refused.
pub async fn build_legacy_deletion_invocation(
    root: dialog_credentials::Ed25519Signer,
    direct_owner_chain: &DelegationChain,
) -> Result<Vec<u8>, DeletionInvocationError> {
    use dialog_varsig::Principal as _;

    let mut proofs = direct_owner_chain.proofs();
    let proof = proofs.next().ok_or(DeletionInvocationError::Grant(
        SpaceDeletionGrantError::Indirect,
    ))?;
    if proofs.next().is_some() {
        return Err(SpaceDeletionGrantError::Indirect.into());
    }
    let space = proof.issuer().clone();
    if proof.subject() != &Subject::Specific(space.clone()) {
        return Err(SpaceDeletionGrantError::WrongSubject.into());
    }
    if proof.audience() != &root.did() {
        return Err(DeletionInvocationError::WrongRoot);
    }
    if !proof.command().0.is_empty() {
        return Err(SpaceDeletionGrantError::WrongCommand.into());
    }
    proof
        .verify_signature(&dialog_credentials::DidKeyResolver)
        .await
        .map_err(|error| SpaceDeletionGrantError::InvalidSignature(error.to_string()))?;
    dialog_ucan_core::time::TimeRange::new(proof.not_before(), proof.expiration())
        .check(&dialog_ucan_core::time::Timestamp::now())
        .map_err(|error| SpaceDeletionGrantError::NotCurrentlyValid(error.to_string()))?;
    let cid = proof.to_cid();
    drop(proofs);
    let invocation = InvocationBuilder::new()
        .issuer(dialog_credentials::Signer::from(root))
        .audience(&space)
        .subject(&space)
        .command(SPACE_DELETE_COMMAND.map(str::to_string).to_vec())
        .proofs(vec![cid])
        .expiration(Timestamp::five_minutes_from_now())
        .try_build()
        .await
        .map_err(|error| DeletionInvocationError::Build(error.to_string()))?;
    InvocationChain::new(invocation, direct_owner_chain.export().collect())
        .to_bytes()
        .map_err(|error| DeletionInvocationError::Encode(error.to_string()))
}

/// Validate exact creator authority for deleting `space`.
///
/// Ordinary Tonk ownership and invite chains historically grant the empty
/// command prefix. That broad operational authority must not silently acquire
/// a newly introduced destructive power, so this validator requires one
/// direct proof carrying exactly `/space/delete`.
pub async fn validate_deletion_grant(
    bytes: &[u8],
    space: &Did,
    owner: &Did,
) -> Result<SpaceDeletionGrant, SpaceDeletionGrantError> {
    let chain = DelegationChain::try_from(bytes)
        .map_err(|error| SpaceDeletionGrantError::InvalidChain(error.to_string()))?;
    let mut proofs = chain.proofs();
    let proof = proofs.next().ok_or(SpaceDeletionGrantError::Indirect)?;
    if proofs.next().is_some() {
        return Err(SpaceDeletionGrantError::Indirect);
    }
    if proof.issuer() != space {
        return Err(SpaceDeletionGrantError::WrongIssuer);
    }
    if proof.subject() != &Subject::Specific(space.clone()) {
        return Err(SpaceDeletionGrantError::WrongSubject);
    }
    if proof.audience() != owner {
        return Err(SpaceDeletionGrantError::WrongOwner);
    }
    let expected: Vec<String> = SPACE_DELETE_COMMAND
        .iter()
        .map(ToString::to_string)
        .collect();
    if proof.command().0 != expected {
        return Err(SpaceDeletionGrantError::WrongCommand);
    }
    proof
        .verify_signature(&dialog_credentials::DidKeyResolver)
        .await
        .map_err(|error| SpaceDeletionGrantError::InvalidSignature(error.to_string()))?;
    dialog_ucan_core::time::TimeRange::new(proof.not_before(), proof.expiration())
        .check(&dialog_ucan_core::time::Timestamp::now())
        .map_err(|error| SpaceDeletionGrantError::NotCurrentlyValid(error.to_string()))?;
    let cid = proof.to_cid().to_string();
    drop(proofs);

    Ok(SpaceDeletionGrant {
        space: space.clone(),
        owner: owner.clone(),
        command: "/space/delete",
        cid,
        chain,
    })
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::time::Timestamp;
    use dialog_ucan_core::{Delegation, DelegationBuilder, DelegationChain};
    use dialog_varsig::AnySignature;
    use dialog_varsig::Principal as _;

    use super::{
        SPACE_DELETE_COMMAND, SpaceDeletionGrantError, mint_deletion_grant, validate_deletion_grant,
    };

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    async fn grant(
        issuer: Ed25519Signer,
        audience: &dialog_varsig::Did,
        subject: Subject,
        command: &[&str],
        expiration: Option<Timestamp>,
    ) -> Delegation<AnySignature> {
        let builder = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience)
            .subject(subject)
            .command(command.iter().map(ToString::to_string).collect());
        match expiration {
            Some(expiration) => builder.expiration(expiration).try_build().await.unwrap(),
            None => builder.try_build().await.unwrap(),
        }
    }

    #[dialog_common::test]
    async fn it_accepts_only_an_exact_direct_space_delete_grant() {
        let space = signer(41).await;
        let owner = signer(42).await;
        let delegation = grant(
            space.clone(),
            &owner.did(),
            Subject::Specific(space.did()),
            &SPACE_DELETE_COMMAND,
            None,
        )
        .await;
        let bytes = DelegationChain::new(delegation).to_bytes().unwrap();

        let validated = validate_deletion_grant(&bytes, &space.did(), &owner.did())
            .await
            .unwrap();

        assert_eq!(validated.space, space.did());
        assert_eq!(validated.owner, owner.did());
        assert_eq!(validated.command, "/space/delete");
    }

    #[dialog_common::test]
    async fn it_mints_the_exact_direct_grant() {
        let space = signer(43).await;
        let owner = signer(44).await;

        let chain = mint_deletion_grant(
            &dialog_credentials::Signer::from(space.clone()),
            &owner.did(),
        )
        .await
        .unwrap();
        let bytes = chain.to_bytes().unwrap();
        let validated = validate_deletion_grant(&bytes, &space.did(), &owner.did())
            .await
            .unwrap();

        assert_eq!(validated.cid, chain.proof_cids()[0].to_string());
    }

    #[dialog_common::test]
    async fn it_builds_a_root_invocation_from_the_exact_space_grant() {
        let space = signer(45).await;
        let root = signer(46).await;
        let grant = mint_deletion_grant(
            &dialog_credentials::Signer::from(space.clone()),
            &root.did(),
        )
        .await
        .unwrap();

        let bytes = super::build_deletion_invocation(root, &grant)
            .await
            .unwrap();
        let invocation = dialog_ucan_core::InvocationChain::try_from(bytes.as_slice()).unwrap();
        invocation
            .verify(&dialog_credentials::DidKeyResolver)
            .await
            .unwrap();

        assert_eq!(invocation.subject(), &space.did());
        assert_eq!(invocation.command().0, ["space", "delete"]);
        assert_eq!(invocation.proofs().len(), 1);
    }

    #[dialog_common::test]
    async fn legacy_upgrade_builds_only_from_the_original_direct_broad_proof() {
        let space = signer(47).await;
        let root = signer(48).await;
        let broad = grant(
            space.clone(),
            &root.did(),
            Subject::Specific(space.did()),
            &[],
            None,
        )
        .await;
        let chain = DelegationChain::new(broad);

        let bytes = super::build_legacy_deletion_invocation(root, &chain)
            .await
            .unwrap();
        let invocation = dialog_ucan_core::InvocationChain::try_from(bytes.as_slice()).unwrap();
        invocation
            .verify(&dialog_credentials::DidKeyResolver)
            .await
            .unwrap();
        assert_eq!(invocation.subject(), &space.did());
        assert_eq!(invocation.command().0, ["space", "delete"]);
        assert_eq!(invocation.proofs(), chain.proof_cids());
    }

    #[dialog_common::test]
    async fn it_rejects_broad_indirect_or_misdirected_grants() {
        let space = signer(51).await;
        let owner = signer(52).await;
        let member = signer(53).await;
        let other = signer(54).await;
        let space_did = space.did();
        let owner_did = owner.did();

        let broad = grant(
            space.clone(),
            &owner_did,
            Subject::Specific(space_did.clone()),
            &[],
            None,
        )
        .await;
        assert!(matches!(
            validate_deletion_grant(
                &DelegationChain::new(broad).to_bytes().unwrap(),
                &space_did,
                &owner_did,
            )
            .await,
            Err(SpaceDeletionGrantError::WrongCommand)
        ));

        let first = grant(
            space.clone(),
            &member.did(),
            Subject::Specific(space_did.clone()),
            &SPACE_DELETE_COMMAND,
            None,
        )
        .await;
        let second = grant(
            member,
            &owner_did,
            Subject::Specific(space_did.clone()),
            &SPACE_DELETE_COMMAND,
            None,
        )
        .await;
        let indirect = DelegationChain::new(first).push(second).unwrap();
        assert!(matches!(
            validate_deletion_grant(&indirect.to_bytes().unwrap(), &space_did, &owner_did,).await,
            Err(SpaceDeletionGrantError::Indirect)
        ));

        for (candidate, expected) in [
            (
                grant(
                    other.clone(),
                    &owner_did,
                    Subject::Specific(space_did.clone()),
                    &SPACE_DELETE_COMMAND,
                    None,
                )
                .await,
                "issuer",
            ),
            (
                grant(
                    space.clone(),
                    &owner_did,
                    Subject::Specific(other.did()),
                    &SPACE_DELETE_COMMAND,
                    None,
                )
                .await,
                "subject",
            ),
            (
                grant(
                    space.clone(),
                    &other.did(),
                    Subject::Specific(space_did.clone()),
                    &SPACE_DELETE_COMMAND,
                    None,
                )
                .await,
                "owner",
            ),
        ] {
            let error = validate_deletion_grant(
                &DelegationChain::new(candidate).to_bytes().unwrap(),
                &space_did,
                &owner_did,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(
                    (&error, expected),
                    (SpaceDeletionGrantError::WrongIssuer, "issuer")
                        | (SpaceDeletionGrantError::WrongSubject, "subject")
                        | (SpaceDeletionGrantError::WrongOwner, "owner")
                ),
                "unexpected {expected} error: {error}"
            );
        }
    }

    #[dialog_common::test]
    async fn it_rejects_an_expired_delete_grant() {
        use dialog_ucan_core::time::timestamp::{Duration, SystemTime};

        let space = signer(61).await;
        let owner = signer(62).await;
        let expired_at = Timestamp::new(SystemTime::now() - Duration::from_secs(60)).unwrap();
        let expired = grant(
            space.clone(),
            &owner.did(),
            Subject::Specific(space.did()),
            &SPACE_DELETE_COMMAND,
            Some(expired_at),
        )
        .await;

        assert!(matches!(
            validate_deletion_grant(
                &DelegationChain::new(expired).to_bytes().unwrap(),
                &space.did(),
                &owner.did(),
            )
            .await,
            Err(SpaceDeletionGrantError::NotCurrentlyValid(_))
        ));
    }
}
