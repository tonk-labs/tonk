//! Self-contained signed revocation artifacts.
//!
//! Every artifact names the delegation it withdraws and carries the ordered
//! delegation path that witnesses that target. Consumers can therefore verify
//! revocation authority without consulting an account provider or registry.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use dialog_credentials::{Ed25519KeyResolver, Ed25519Signer};
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::{DelegationChain, InvocationBuilder, InvocationChain};
use dialog_varsig::algorithm::eddsa::Ed25519Signature;
use dialog_varsig::{Did, Principal};
use ipld_core::cid::Cid;

/// The command a revocation invokes.
pub const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// The argument naming the withdrawn delegation.
pub const REVOKE_ARGUMENT: &str = "revoke";

/// The argument carrying the hex-encoded canonical delegation path.
pub const PATH_ARGUMENT: &str = "path";

/// Authority established by a verified revocation artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationAuthority {
    /// The signer issued a delegation in the witnessed prefix through the target.
    PathIssuer,
    /// The signer exercised authority delegated through the target.
    Delegated,
}

/// Facts derived from a verified revocation artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRevocation {
    /// Canonical CID of the delegation being withdrawn.
    pub target_cid: String,
    /// Canonical CID of the signed invocation artifact.
    pub artifact_cid: String,
    /// Expiration of the target delegation, if it has one.
    pub target_expires_at: Option<u64>,
    /// DID that signed the revocation invocation.
    pub issuer: Did,
    /// How the signer proved revocation authority.
    pub authority: RevocationAuthority,
}

/// Why a revocation artifact could not be verified.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// The bytes or signed fields are malformed.
    #[error("invalid revocation artifact: {0}")]
    Malformed(String),
    /// The artifact is well-formed but does not prove revocation authority.
    #[error("unauthorized revocation artifact: {0}")]
    Unauthorized(String),
}

fn command() -> Vec<String> {
    REVOKE_COMMAND
        .iter()
        .map(|part| (*part).to_string())
        .collect()
}

fn arguments(target: &Cid, path: &DelegationChain) -> Result<BTreeMap<String, Promised>> {
    let mut arguments = BTreeMap::new();
    arguments.insert(
        REVOKE_ARGUMENT.to_string(),
        Promised::String(target.to_string()),
    );
    arguments.insert(
        PATH_ARGUMENT.to_string(),
        Promised::String(hex::encode(
            path.to_bytes()
                .context("failed to serialize revocation path")?,
        )),
    );
    Ok(arguments)
}

fn target_index(path: &DelegationChain, target: &Cid) -> Result<usize> {
    let mut matches = path
        .proof_cids()
        .iter()
        .enumerate()
        .filter_map(|(index, cid)| (cid == target).then_some(index));
    let index = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("revocation path does not contain target {target}"))?;
    if matches.next().is_some() {
        anyhow::bail!("revocation path contains target {target} more than once");
    }
    Ok(index)
}

fn is_path_issuer(path: &DelegationChain, target_index: usize, issuer: &Did) -> bool {
    path.proofs()
        .take(target_index + 1)
        .any(|delegation| delegation.issuer() == issuer)
}

fn subject(path: &DelegationChain) -> Did {
    path.subject()
        .cloned()
        .unwrap_or_else(|| path.issuer().clone())
}

async fn mint(
    issuer: Ed25519Signer,
    path: &DelegationChain,
    target: &Cid,
    proofs: Option<&DelegationChain>,
) -> Result<Vec<u8>> {
    target_index(path, target)?;
    let subject = subject(path);
    let proof_cids = proofs
        .map(|chain| chain.proof_cids().to_vec())
        .unwrap_or_default();
    let invocation = InvocationBuilder::new()
        .issuer(issuer)
        .audience(&subject)
        .subject(&subject)
        .command(command())
        .arguments(arguments(target, path)?)
        .proofs(proof_cids)
        .try_build()
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;
    let delegations = proofs
        .map(|chain| chain.export().collect())
        .unwrap_or_default();

    InvocationChain::new(invocation, delegations)
        .to_bytes()
        .map_err(|err| anyhow::anyhow!("failed to serialize the revocation: {err}"))
}

/// Mint a proofless revocation signed by an issuer in the witnessed path.
pub async fn mint_root_revocation(
    root: Ed25519Signer,
    path: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    let index = target_index(path, target)?;
    if !is_path_issuer(path, index, &root.did()) {
        anyhow::bail!("revocation signer is not an issuer in the target path");
    }
    mint(root, path, target, None).await
}

/// Mint a device-signed revocation of the device's own grant.
pub async fn mint_self_revocation(
    device: Ed25519Signer,
    grant: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    mint(device, grant, target, Some(grant)).await
}

/// Mint a revocation signed under an attached delegation proof chain.
pub async fn mint_delegated_revocation(
    issuer: Ed25519Signer,
    path: &DelegationChain,
    target: &Cid,
    proofs: &DelegationChain,
) -> Result<Vec<u8>> {
    mint(issuer, path, target, Some(proofs)).await
}

fn string_argument<'a>(
    chain: &'a InvocationChain<Ed25519Signature>,
    name: &str,
) -> std::result::Result<&'a str, VerifyError> {
    match chain.arguments().get(name) {
        Some(Promised::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(VerifyError::Malformed(format!(
            "{name} must be a non-empty string"
        ))),
    }
}

/// Parse and verify a self-contained revocation artifact.
pub async fn verify(bytes: &[u8]) -> std::result::Result<VerifiedRevocation, VerifyError> {
    let chain = InvocationChain::<Ed25519Signature>::try_from(bytes)
        .map_err(|err| VerifyError::Malformed(format!("bad invocation container: {err}")))?;

    let actual_command: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if actual_command.as_slice() != REVOKE_COMMAND {
        return Err(VerifyError::Malformed(format!(
            "expected command {REVOKE_COMMAND:?}, got {actual_command:?}"
        )));
    }

    let target_string = string_argument(&chain, REVOKE_ARGUMENT)?;
    let target = target_string
        .parse::<Cid>()
        .map_err(|err| VerifyError::Malformed(format!("invalid target CID: {err}")))?;
    if target.to_string() != target_string {
        return Err(VerifyError::Malformed(
            "target CID is not canonical".to_string(),
        ));
    }

    let path_hex = string_argument(&chain, PATH_ARGUMENT)?;
    let path_bytes = hex::decode(path_hex)
        .map_err(|err| VerifyError::Malformed(format!("invalid path hex: {err}")))?;
    let path = DelegationChain::try_from(path_bytes.as_slice())
        .map_err(|err| VerifyError::Malformed(format!("invalid delegation path: {err}")))?;

    for delegation in path.proofs() {
        delegation
            .verify_signature(&Ed25519KeyResolver)
            .await
            .map_err(|err| {
                VerifyError::Unauthorized(format!("path signature failed to verify: {err}"))
            })?;
    }

    let mut matches = path
        .proof_cids()
        .iter()
        .enumerate()
        .filter_map(|(index, cid)| (cid == &target).then_some(index));
    let target_index = matches.next().ok_or_else(|| {
        VerifyError::Malformed("revocation path does not contain the named CID".to_string())
    })?;
    if matches.next().is_some() {
        return Err(VerifyError::Malformed(
            "revocation path contains the named CID more than once".to_string(),
        ));
    }

    chain
        .invocation
        .verify_signature(&Ed25519KeyResolver)
        .await
        .map_err(|err| {
            VerifyError::Unauthorized(format!("invocation signature failed to verify: {err}"))
        })?;

    let issuer = chain.issuer().clone();
    let authority = if is_path_issuer(&path, target_index, &issuer) {
        RevocationAuthority::PathIssuer
    } else {
        chain.verify(&Ed25519KeyResolver).await.map_err(|err| {
            VerifyError::Unauthorized(format!("delegated authority failed to verify: {err}"))
        })?;
        if !chain.proofs().contains(&target) {
            return Err(VerifyError::Unauthorized(
                "delegated revocation does not attach the target as a proof".to_string(),
            ));
        }
        RevocationAuthority::Delegated
    };

    let canonical_bytes = chain.to_bytes().map_err(|err| {
        VerifyError::Malformed(format!(
            "failed to canonicalize invocation container: {err}"
        ))
    })?;
    if canonical_bytes != bytes {
        return Err(VerifyError::Malformed(
            "invocation container is not canonical".to_string(),
        ));
    }

    let target_expires_at = path
        .proofs()
        .nth(target_index)
        .and_then(|delegation| delegation.expiration())
        .map(|expiration| expiration.to_unix());

    Ok(VerifiedRevocation {
        target_cid: target.to_string(),
        artifact_cid: chain.invocation.to_cid().to_string(),
        target_expires_at,
        issuer,
        authority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
    use dialog_ucan_core::{DelegationBuilder, InvocationBuilder};

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.unwrap()
    }

    async fn root_grant() -> (Ed25519Signer, Ed25519Signer, DelegationChain) {
        let root = signer(1).await;
        let device = signer(2).await;
        let grant = crate::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        (root, device, grant)
    }

    async fn invite_path() -> (Ed25519Signer, Ed25519Signer, Ed25519Signer, DelegationChain) {
        let space = signer(3).await;
        let member = signer(4).await;
        let invite = signer(5).await;
        let first = DelegationBuilder::new()
            .issuer(space.clone())
            .audience(&member.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let second = DelegationBuilder::new()
            .issuer(member.clone())
            .audience(&invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let path = DelegationChain::new(first).push(second).unwrap();
        (space, member, invite, path)
    }

    async fn raw_revocation(
        issuer: Ed25519Signer,
        path: &DelegationChain,
        named: String,
        proofs: Option<&DelegationChain>,
    ) -> Vec<u8> {
        let mut args = BTreeMap::new();
        args.insert(REVOKE_ARGUMENT.into(), Promised::String(named));
        args.insert(
            PATH_ARGUMENT.into(),
            Promised::String(hex::encode(path.to_bytes().unwrap())),
        );
        let subject = subject(path);
        let invocation = InvocationBuilder::new()
            .issuer(issuer)
            .audience(&subject)
            .subject(&subject)
            .command(command())
            .arguments(args)
            .proofs(
                proofs
                    .map(|chain| chain.proof_cids().to_vec())
                    .unwrap_or_default(),
            )
            .try_build()
            .await
            .unwrap();
        InvocationChain::new(
            invocation,
            proofs
                .map(|chain| chain.export().collect())
                .unwrap_or_default(),
        )
        .to_bytes()
        .unwrap()
    }

    #[dialog_common::test]
    async fn it_verifies_a_root_revocation_with_the_target_path() {
        let (root, _, path) = root_grant().await;
        let target = path.proof_cids()[0];
        let verified = verify(
            &mint_root_revocation(root.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.target_cid, target.to_string());
        assert_eq!(verified.issuer, root.did());
        assert_eq!(verified.authority, RevocationAuthority::PathIssuer);
    }

    #[dialog_common::test]
    async fn it_verifies_a_device_self_revocation_with_the_target_path() {
        let (_, device, path) = root_grant().await;
        let target = path.proof_cids()[0];
        let verified = verify(
            &mint_self_revocation(device.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.issuer, device.did());
        assert_eq!(verified.authority, RevocationAuthority::Delegated);
    }

    #[dialog_common::test]
    async fn it_verifies_an_invite_revocation_by_an_issuer_in_the_path() {
        let (_, member, _, path) = invite_path().await;
        let target = path.proof_cids()[1];
        let verified = verify(
            &mint_root_revocation(member.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.issuer, member.did());
        assert_eq!(verified.authority, RevocationAuthority::PathIssuer);
    }

    #[dialog_common::test]
    async fn it_rejects_a_decoy_path_that_omits_the_named_cid() {
        let (root, _, real_path) = root_grant().await;
        let (_, _, decoy_path) = {
            let decoy_device = signer(9).await;
            let decoy =
                crate::delegation::mint_device_delegation(root.clone(), &decoy_device.did())
                    .await
                    .unwrap();
            (root.clone(), decoy_device, decoy)
        };
        let bytes = raw_revocation(
            root,
            &decoy_path,
            real_path.proof_cids()[0].to_string(),
            None,
        )
        .await;

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Malformed(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_path_changed_after_signing() {
        let (space, member, _, original) = invite_path().await;
        let other_invite = signer(6).await;
        let replacement_leaf = DelegationBuilder::new()
            .issuer(member)
            .audience(&other_invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let changed = DelegationChain::new(original.proofs().next().unwrap().clone())
            .push(replacement_leaf)
            .unwrap();
        let target = original.proof_cids()[0];
        let mut bytes = mint_root_revocation(space, &original, &target)
            .await
            .unwrap();
        let old = hex::encode(original.to_bytes().unwrap());
        let new = hex::encode(changed.to_bytes().unwrap());
        assert_eq!(old.len(), new.len());
        let start = bytes
            .windows(old.len())
            .position(|window| window == old.as_bytes())
            .unwrap();
        bytes[start..start + old.len()].copy_from_slice(new.as_bytes());

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_an_unauthorized_issuer() {
        let (_, _, _, path) = invite_path().await;
        let outsider = signer(8).await;
        let bytes = raw_revocation(outsider, &path, path.proof_cids()[1].to_string(), None).await;

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_non_canonical_target_cid() {
        let (root, _, path) = root_grant().await;
        let bytes = raw_revocation(
            root,
            &path,
            path.proof_cids()[0].to_string().to_ascii_uppercase(),
            None,
        )
        .await;

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Malformed(_))
        ));
    }

    #[dialog_common::test]
    async fn it_reports_the_target_delegations_expiration() {
        let root = signer(1).await;
        let device = signer(2).await;
        let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(300)).unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(root.clone())
            .audience(&device.did())
            .subject(Subject::Any)
            .command(vec![])
            .expiration(expiration)
            .try_build()
            .await
            .unwrap();
        let path = DelegationChain::new(delegation);
        let target = path.proof_cids()[0];
        let verified = verify(&mint_root_revocation(root, &path, &target).await.unwrap())
            .await
            .unwrap();

        assert_eq!(verified.target_expires_at, Some(expiration.to_unix()));
    }
}
