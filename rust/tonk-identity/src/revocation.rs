//! Self-contained signed revocation artifacts.
//!
//! Every artifact names the delegation it withdraws and carries the ordered
//! delegation path that witnesses that target. Consumers can therefore verify
//! revocation authority without consulting an account provider or registry.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use dialog_credentials::{DidKeyResolver, Signer};
use dialog_ucan_core::crypto::nonce::Nonce;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::{
    Container, Delegation, DelegationChain, InvocationBuilder, InvocationChain,
};
use dialog_varsig::AnySignature;
use dialog_varsig::{Did, Principal};
use ipld_core::cid::Cid;

/// The command a revocation invokes.
pub const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// The spec sets `nonce` to the empty byte string, because revocation is
/// idempotent: revoking the same delegation twice is one fact, so the
/// two invocations share a CID rather than being distinct acts. A random
/// nonce would make every replay a new invocation to store and bill.
fn nonce() -> Nonce {
    Nonce::Custom(Vec::new())
}

/// The argument naming the withdrawn delegation.
///
/// `rev`, not `revoke`: the spec's IPLD schema is normative and uses the
/// abbreviated wire names, matching `cmd` / `nnc` / `prf` elsewhere in
/// the envelope. The prose in the spec README spells them out, which is
/// where the longer names came from.
pub const REVOKE_ARGUMENT: &str = "rev";

/// The argument carrying the delegation-path witness.
pub const PATH_ARGUMENT: &str = "pth";

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
    ///
    /// Who performed the act. Distinct from [`subject`](Self::subject)
    /// when the revocation was minted under delegated authority.
    pub issuer: Did,
    /// DID whose authority the revocation exercises.
    ///
    /// What a validator matches against the issuers of a presented
    /// chain: the spec's pseudocode tests the invocation's issuer, which
    /// contradicts its own delegated-revocation section, since a
    /// delegate is never in the chain whose authority they borrowed.
    /// Fixed upstream in ucan-wg/revocation#4; we implement the
    /// corrected form.
    pub subject: Did,
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

/// Build the spec's arguments: the target as a link, and the witness as
/// a list of links.
///
/// Both are IPLD links rather than strings. The spec types `revoke` as
/// `&Delegation` and `path` as `[&Delegation]`, so a revocation minted
/// here decodes as one anywhere else that implements the spec, and the
/// CIDs stay addressable rather than being opaque text.
fn arguments(target: &Cid, path: &DelegationChain) -> Result<BTreeMap<String, Promised>> {
    let index = target_index(path, target)?;
    let mut arguments = BTreeMap::new();
    arguments.insert(REVOKE_ARGUMENT.to_string(), Promised::Link(*target));
    // The witness runs from the root through the target: enough to show
    // the revoker issued something on the way, and no more.
    arguments.insert(
        PATH_ARGUMENT.to_string(),
        Promised::List(
            path.proof_cids()
                .iter()
                .take(index + 1)
                .map(|cid| Promised::Link(*cid))
                .collect(),
        ),
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
    issuer: impl Into<Signer>,
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
        .issuer(issuer.into())
        .audience(&subject)
        .subject(&subject)
        .command(command())
        .arguments(arguments(target, path)?)
        .proofs(proof_cids)
        .nonce(nonce())
        .try_build()
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;
    // Assemble the container directly rather than through
    // `InvocationChain::to_bytes`, which emits only delegations named in
    // `invocation.proofs()`. The witness is referenced from `args.path`
    // rather than from proofs, so that writer would drop it and leave a
    // verifier unable to resolve the links it was handed.
    let mut tokens = vec![
        serde_ipld_dagcbor::to_vec(&invocation)
            .context("failed to serialize the revocation invocation")?,
    ];
    let mut seen: BTreeSet<Cid> = BTreeSet::new();
    for (cid, delegation) in path
        .export()
        .chain(proofs.into_iter().flat_map(|c| c.export()))
    {
        if seen.insert(cid) {
            tokens.push(delegation.encoded().to_vec());
        }
    }

    Container::new(tokens)
        .into_bytes()
        .map_err(|err| anyhow::anyhow!("failed to serialize the revocation: {err}"))
}

/// Mint a proofless revocation signed by an issuer in the witnessed path.
pub async fn mint_root_revocation(
    root: impl Into<Signer>,
    path: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    let root: Signer = root.into();
    let index = target_index(path, target)?;
    if !is_path_issuer(path, index, &root.did()) {
        anyhow::bail!("revocation signer is not an issuer in the target path");
    }
    mint(root, path, target, None).await
}

/// Mint a device-signed revocation of the device's own grant.
pub async fn mint_self_revocation(
    device: impl Into<Signer>,
    grant: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    mint(device, grant, target, Some(grant)).await
}

/// Mint a revocation signed under an attached delegation proof chain.
pub async fn mint_delegated_revocation(
    issuer: impl Into<Signer>,
    path: &DelegationChain,
    target: &Cid,
    proofs: &DelegationChain,
) -> Result<Vec<u8>> {
    mint(issuer, path, target, Some(proofs)).await
}

/// Index every delegation the container carries, keyed by canonical CID.
fn carried_delegations(
    bytes: &[u8],
) -> std::result::Result<HashMap<Cid, Delegation<AnySignature>>, VerifyError> {
    let tokens = Container::from_bytes(bytes)
        .map_err(|err| VerifyError::Malformed(format!("bad container: {err}")))?
        .into_tokens();
    let mut carried = HashMap::new();
    // Token 0 is the invocation; the rest are delegations.
    for token in tokens.iter().skip(1) {
        let delegation: Delegation<AnySignature> = serde_ipld_dagcbor::from_slice(token)
            .map_err(|err| VerifyError::Malformed(format!("bad delegation token: {err}")))?;
        carried.insert(delegation.to_cid(), delegation);
    }
    Ok(carried)
}

/// Read a link argument, per the spec's `&Delegation`.
fn link_argument(
    chain: &InvocationChain<AnySignature>,
    name: &str,
) -> std::result::Result<Cid, VerifyError> {
    match chain.arguments().get(name) {
        Some(Promised::Link(cid)) => Ok(*cid),
        _ => Err(VerifyError::Malformed(format!("{name} must be a link"))),
    }
}

/// Read a list-of-links argument, per the spec's `[&Delegation]`.
fn link_list_argument(
    chain: &InvocationChain<AnySignature>,
    name: &str,
) -> std::result::Result<Vec<Cid>, VerifyError> {
    let Some(Promised::List(items)) = chain.arguments().get(name) else {
        return Err(VerifyError::Malformed(format!(
            "{name} must be a list of links"
        )));
    };
    if items.is_empty() {
        return Err(VerifyError::Malformed(format!("{name} must not be empty")));
    }
    items
        .iter()
        .map(|item| match item {
            Promised::Link(cid) => Ok(*cid),
            _ => Err(VerifyError::Malformed(format!(
                "{name} must contain only links"
            ))),
        })
        .collect()
}

/// Parse and verify a self-contained revocation artifact.
pub async fn verify(bytes: &[u8]) -> std::result::Result<VerifiedRevocation, VerifyError> {
    let chain = InvocationChain::<AnySignature>::try_from(bytes)
        .map_err(|err| VerifyError::Malformed(format!("bad invocation container: {err}")))?;

    let actual_command: Vec<&str> = chain.command().0.iter().map(String::as_str).collect();
    if actual_command.as_slice() != REVOKE_COMMAND {
        return Err(VerifyError::Malformed(format!(
            "expected command {REVOKE_COMMAND:?}, got {actual_command:?}"
        )));
    }

    let target = link_argument(&chain, REVOKE_ARGUMENT)?;

    // The witness names its delegations by link; resolve each against
    // the container that carried them. `InvocationChain` keeps its
    // delegation map private, so index the tokens we already parsed.
    let carried = carried_delegations(bytes)?;
    let path_cids = link_list_argument(&chain, PATH_ARGUMENT)?;
    let mut path_delegations = Vec::with_capacity(path_cids.len());
    for cid in &path_cids {
        let delegation = carried.get(cid).ok_or_else(|| {
            VerifyError::Malformed(format!("witness delegation {cid} is not in the container"))
        })?;
        delegation
            .verify_signature(&DidKeyResolver)
            .await
            .map_err(|err| {
                VerifyError::Unauthorized(format!("path signature failed to verify: {err}"))
            })?;
        path_delegations.push(delegation);
    }

    // A witness is a delegation PATH, not a bag of delegations. Signatures
    // alone prove each hop was issued; they prove nothing about reach. So
    // require the hops to link: every issuer after the first must be the
    // previous hop's audience. Without this, any principal could staple an
    // unrelated (validly signed) grant onto a real prefix and claim the
    // authority the prefix carries.
    for pair in path_delegations.windows(2) {
        if pair[0].audience() != pair[1].issuer() {
            return Err(VerifyError::Unauthorized(format!(
                "witness hops do not connect: {} does not follow {}",
                pair[1].issuer(),
                pair[0].audience()
            )));
        }
    }

    let mut matches = path_cids
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
        .verify_signature(&DidKeyResolver)
        .await
        .map_err(|err| {
            VerifyError::Unauthorized(format!("invocation signature failed to verify: {err}"))
        })?;

    let issuer = chain.issuer().clone();
    let authority = if path_delegations
        .iter()
        .take(target_index + 1)
        .any(|delegation| delegation.issuer() == &issuer)
    {
        RevocationAuthority::PathIssuer
    } else {
        chain.verify(&DidKeyResolver).await.map_err(|err| {
            VerifyError::Unauthorized(format!("delegated authority failed to verify: {err}"))
        })?;
        if !chain.proofs().contains(&target) {
            return Err(VerifyError::Unauthorized(
                "delegated revocation does not attach the target as a proof".to_string(),
            ));
        }
        // Attaching the target is not the same as holding authority over
        // it. Passing the whole witnessed path as its own proof satisfies
        // the check above for EVERY hop, which would let any principal on
        // a chain revoke the hops above its own. So require that the
        // revoker is a principal of the target hop itself: its issuer
        // (revoking what it granted) or its audience (declining what it
        // was granted).
        let target_delegation = path_delegations
            .get(target_index)
            .ok_or_else(|| VerifyError::Malformed("target index out of range".to_string()))?;
        if target_delegation.issuer() != &issuer && target_delegation.audience() != &issuer {
            return Err(VerifyError::Unauthorized(format!(
                "{issuer} is neither issuer nor audience of the revoked delegation"
            )));
        }
        RevocationAuthority::Delegated
    };

    // Re-encode and compare, so a container carrying extra tokens, a
    // different order, or a re-serialized delegation is refused rather
    // than silently accepted. Rebuilt the way `mint` builds it:
    // `InvocationChain::to_bytes` would drop the witness, since the
    // witness is named by `args.path` rather than by proofs.
    let mut expected = vec![
        serde_ipld_dagcbor::to_vec(&chain.invocation).map_err(|err| {
            VerifyError::Malformed(format!("failed to re-encode the invocation: {err}"))
        })?,
    ];
    let mut seen: BTreeSet<Cid> = BTreeSet::new();
    for cid in path_cids.iter().chain(chain.proofs().iter()) {
        if !seen.insert(*cid) {
            continue;
        }
        let delegation = carried.get(cid).ok_or_else(|| {
            VerifyError::Malformed(format!("delegation {cid} is not in the container"))
        })?;
        expected.push(delegation.encoded().to_vec());
    }
    let canonical_bytes = Container::new(expected).into_bytes().map_err(|err| {
        VerifyError::Malformed(format!(
            "failed to canonicalize invocation container: {err}"
        ))
    })?;
    if canonical_bytes != bytes {
        return Err(VerifyError::Malformed(
            "invocation container is not canonical".to_string(),
        ));
    }

    let target_expires_at = path_delegations
        .get(target_index)
        .and_then(|delegation| delegation.expiration())
        .map(|expiration| expiration.to_unix());

    Ok(VerifiedRevocation {
        target_cid: target.to_string(),
        artifact_cid: chain.invocation.to_cid().to_string(),
        target_expires_at,
        issuer,
        subject: chain.subject().clone(),
        authority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
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
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&member.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let second = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(member.clone()))
            .audience(&invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let path = DelegationChain::new(first).push(second).unwrap();
        (space, member, invite, path)
    }

    /// Build a revocation container by hand, so tests can inject a
    /// malformed `revoke` argument that `mint` would never produce.
    async fn raw_revocation(
        issuer: impl Into<Signer>,
        path: &DelegationChain,
        named: Promised,
        proofs: Option<&DelegationChain>,
    ) -> Vec<u8> {
        let mut args = BTreeMap::new();
        args.insert(REVOKE_ARGUMENT.into(), named);
        args.insert(
            PATH_ARGUMENT.into(),
            Promised::List(
                path.proof_cids()
                    .iter()
                    .map(|cid| Promised::Link(*cid))
                    .collect(),
            ),
        );
        let subject = subject(path);
        let invocation = InvocationBuilder::new()
            .issuer(issuer.into())
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
        let mut tokens = vec![serde_ipld_dagcbor::to_vec(&invocation).unwrap()];
        let mut seen: BTreeSet<Cid> = BTreeSet::new();
        for (cid, delegation) in path
            .export()
            .chain(proofs.into_iter().flat_map(|chain| chain.export()))
        {
            if seen.insert(cid) {
                tokens.push(delegation.encoded().to_vec());
            }
        }
        Container::new(tokens).into_bytes().unwrap()
    }

    /// Build a revocation whose `pth` names an arbitrary list of
    /// delegations, connected or not.
    ///
    /// `DelegationChain::push` refuses a hop that does not follow its
    /// predecessor, so a disconnected witness cannot be expressed through
    /// the normal builders — but nothing stops a hostile client from
    /// emitting the CBOR directly, which is what this reproduces.
    async fn raw_revocation_with_path(
        issuer: impl Into<Signer>,
        subject_did: &Did,
        witness: &[&Delegation<AnySignature>],
        named: Promised,
    ) -> Vec<u8> {
        let mut args = BTreeMap::new();
        args.insert(REVOKE_ARGUMENT.into(), named);
        args.insert(
            PATH_ARGUMENT.into(),
            Promised::List(
                witness
                    .iter()
                    .map(|delegation| Promised::Link(delegation.to_cid()))
                    .collect(),
            ),
        );
        let invocation = InvocationBuilder::new()
            .issuer(issuer.into())
            .audience(subject_did)
            .subject(subject_did)
            .command(command())
            .arguments(args)
            .proofs(Vec::new())
            .nonce(nonce())
            .try_build()
            .await
            .unwrap();
        let mut tokens = vec![serde_ipld_dagcbor::to_vec(&invocation).unwrap()];
        for delegation in witness {
            tokens.push(delegation.encoded().to_vec());
        }
        Container::new(tokens).into_bytes().unwrap()
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
            Promised::Link(real_path.proof_cids()[0]),
            None,
        )
        .await;

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Malformed(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_witness_swapped_after_signing() {
        // The witness is named by link, so tampering means swapping the
        // carried block for a different delegation. Its CID no longer
        // matches what the signed arguments name, and the container
        // stops being canonical.
        let (space, member, _, original) = invite_path().await;
        let other_invite = signer(6).await;
        let replacement_leaf = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(member))
            .audience(&other_invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let target = original.proof_cids()[0];
        let bytes = mint_root_revocation(space, &original, &target)
            .await
            .unwrap();

        // Replace the last carried delegation with the replacement.
        let mut tokens = Container::from_bytes(&bytes).unwrap().into_tokens();
        *tokens.last_mut().unwrap() = replacement_leaf.encoded().to_vec();
        let tampered = Container::new(tokens).into_bytes().unwrap();

        assert!(matches!(
            verify(&tampered).await,
            Err(VerifyError::Malformed(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegated_revocation_naming_another_subject() {
        // Borrowing the right to revoke on one subject does not grant it
        // on another. ucanto covers the same escape as "with field must
        // match"; here the delegation names the space, and the
        // revocation is minted against an unrelated one.
        let (space, _, invite, path) = invite_path().await;
        let other_space = signer(9).await;
        let borrowed = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(&invite.did())
            .subject(Subject::Specific(other_space.did()))
            .command(command())
            .try_build()
            .await
            .unwrap();
        let borrowed = DelegationChain::new(borrowed);
        let target = path.proof_cids()[1];

        // Mint by hand: `mint_delegated_revocation` would refuse before
        // producing anything, and the point is that a hand-rolled
        // artifact does not verify either.
        let bytes = raw_revocation(invite, &path, Promised::Link(target), Some(&borrowed)).await;
        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_rejects_a_delegated_revocation_without_the_target_attached() {
        // The delegation that grants revocation authority pins what may
        // be revoked. Detaching the target and naming a different one is
        // ucanto's "nb.delegation field must match".
        let (space, _, invite, path) = invite_path().await;
        let granted = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&invite.did())
            .subject(Subject::Specific(space.did()))
            .command(command())
            .try_build()
            .await
            .unwrap();
        // The grant is attached, but the target delegation is not, so
        // nothing ties this authority to the delegation being withdrawn.
        let detached = DelegationChain::new(granted);
        let target = path.proof_cids()[1];

        let bytes = raw_revocation(invite, &path, Promised::Link(target), Some(&detached)).await;
        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_lets_the_root_revoke_a_grandchild_it_never_issued() {
        // The spec's reach rule: "any UCAN that contains a proof where
        // the revoker matches the `iss` field, even transitively, MAY be
        // revoked." The space issued only the first hop, yet may revoke
        // the second.
        let (space, member, _, path) = invite_path().await;
        let grandchild = path.proof_cids()[1];
        assert_ne!(path.proofs().next().unwrap().issuer(), &member.did());

        let verified = verify(
            &mint_root_revocation(space.clone(), &path, &grandchild)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.target_cid, grandchild.to_string());
        assert_eq!(verified.subject, space.did());
        assert_eq!(verified.authority, RevocationAuthority::PathIssuer);
    }

    #[dialog_common::test]
    async fn it_lets_the_audience_revoke_the_hop_granted_to_it() {
        // "Revoke a delegation made to you." The invite key is the
        // AUDIENCE of the hop it withdraws, never its issuer, so the
        // path-issuer branch cannot carry this. It goes through the
        // delegated branch instead, attaching the grant as proof — which
        // is the stronger route: the grant is verified rather than
        // matched by name.
        //
        // No separate evidence chain is needed. The witnessed path IS the
        // proof, so `pth` carries the hop and nothing else has to be
        // supplied.
        let (_, _, invite, path) = invite_path().await;
        let target = path.proof_cids()[1];

        let verified = verify(
            &mint_self_revocation(invite.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.issuer, invite.did());
        assert_eq!(verified.target_cid, target.to_string());
        assert_eq!(verified.authority, RevocationAuthority::Delegated);
    }

    #[dialog_common::test]
    async fn it_refuses_an_audience_revoking_a_hop_above_its_own() {
        // The audience allowance reaches its own hop, not the ones above
        // it. The invite key received hop 1; hop 0 (space -> member) is
        // not its to withdraw, and attaching the path as proof does not
        // make it so.
        let (_, _, invite, path) = invite_path().await;
        let above = path.proof_cids()[0];

        let bytes = mint_self_revocation(invite, &path, &above).await.unwrap();
        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_a_revoker_further_down_the_chain_than_the_target() {
        // Authority runs downward only. The member issued the second
        // hop, so it may not revoke the first one above it.
        let (_, member, _, path) = invite_path().await;
        let above = path.proof_cids()[0];

        assert!(
            mint_root_revocation(member.clone(), &path, &above)
                .await
                .is_err()
        );
        let bytes = raw_revocation(member, &path, Promised::Link(above), None).await;
        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_records_the_subject_rather_than_the_signer() {
        // What a validator matches against a presented chain's issuers.
        // For a self-revocation the device signs, but the authority
        // exercised is the space's, and the space is what appears in
        // chains rooted there.
        let (space, _, invite, path) = invite_path().await;
        let target = path.proof_cids()[1];
        let verified = verify(
            &mint_self_revocation(invite.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.issuer, invite.did(), "the device signed it");
        assert_eq!(
            verified.subject,
            space.did(),
            "the space's authority is what a chain check matches"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_witness_that_is_not_a_connected_chain() {
        // A witness is a delegation path, not a bag of delegations: two
        // unrelated grants prove nothing about reach.
        let (space, _, invite, path) = invite_path().await;
        let unrelated_space = signer(11).await;
        let unrelated = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(unrelated_space.clone()))
            .audience(&invite.did())
            .subject(Subject::Specific(unrelated_space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let disjoint = DelegationChain::new(unrelated);
        let target = path.proof_cids()[1];

        // The target is not in the presented witness at all.
        let bytes = raw_revocation(space, &disjoint, Promised::Link(target), None).await;
        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Malformed(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_a_witness_whose_hops_do_not_connect() {
        // The attack: staple a real prefix onto an unrelated hop and
        // present the pair as ONE witness. Every signature verifies and
        // the target is present, but `member` never delegated to
        // `stranger`, so the second hop hangs off nothing and the space's
        // authority does not reach it.
        //
        // Without a linkage check the stranger's own hop sits at index 1
        // with `space -> member` in front of it, so the prefix scan finds
        // the stranger as an issuer and grants authority over a path the
        // space never authorized.
        let space = signer(3).await;
        let member = signer(4).await;
        let stranger = signer(12).await;
        let invite = signer(5).await;

        let first = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&member.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let orphan = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(stranger.clone()))
            .audience(&invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let orphan_cid = orphan.to_cid();
        // `pth` = [space -> member, stranger -> invite]. Both signed,
        // target at index 1, and the stranger IS an issuer within that
        // prefix — so the prefix scan alone would grant authority.
        let bytes = raw_revocation_with_path(
            stranger,
            &space.did(),
            &[&first, &orphan],
            Promised::Link(orphan_cid),
        )
        .await;
        assert!(
            verify(&bytes).await.is_err(),
            "a witness whose hops do not connect must not establish authority"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_an_unauthorized_issuer() {
        let (_, _, _, path) = invite_path().await;
        let outsider = signer(8).await;
        let bytes =
            raw_revocation(outsider, &path, Promised::Link(path.proof_cids()[1]), None).await;

        assert!(matches!(
            verify(&bytes).await,
            Err(VerifyError::Unauthorized(_))
        ));
    }

    #[dialog_common::test]
    async fn it_encodes_arguments_as_native_dag_cbor_links() {
        // The spec types `revoke` as `&Delegation` and `path` as
        // `[&Delegation]`. Decoding the invocation straight to Ipld is
        // what proves they are real links: a stringified or hex-encoded
        // CID would surface as Ipld::String here, not Ipld::Link.
        use ipld_core::ipld::Ipld;

        let (root, _, path) = root_grant().await;
        let target = path.proof_cids()[0];
        let bytes = mint_root_revocation(root, &path, &target).await.unwrap();

        let tokens = Container::from_bytes(&bytes).unwrap().into_tokens();
        let invocation: Ipld = serde_ipld_dagcbor::from_slice(&tokens[0]).unwrap();

        // Walk to the arguments without assuming the envelope's shape.
        fn find<'a>(node: &'a Ipld, key: &str) -> Option<&'a Ipld> {
            match node {
                Ipld::Map(map) => map
                    .get(key)
                    .or_else(|| map.values().find_map(|v| find(v, key))),
                Ipld::List(items) => items.iter().find_map(|v| find(v, key)),
                _ => None,
            }
        }

        let args = find(&invocation, "args").expect("invocation carries arguments");
        match find(args, REVOKE_ARGUMENT).expect("revoke argument is present") {
            Ipld::Link(cid) => assert_eq!(cid, &target),
            other => panic!("revoke must be a link, got {other:?}"),
        }
        match find(args, PATH_ARGUMENT).expect("path argument is present") {
            Ipld::List(items) => {
                assert!(!items.is_empty(), "path must not be empty");
                for item in items {
                    assert!(
                        matches!(item, Ipld::Link(_)),
                        "path must contain only links, got {item:?}"
                    );
                }
            }
            other => panic!("path must be a list, got {other:?}"),
        }
    }

    #[dialog_common::test]
    async fn it_mints_the_same_bytes_for_a_repeated_revocation() {
        // The empty nonce is what makes revocation idempotent: the same
        // revoker withdrawing the same delegation twice produces one
        // artifact, so a replay is recognizably the same fact rather
        // than a second one to store and bill.
        let (root, _, path) = root_grant().await;
        let target = path.proof_cids()[0];
        let first = mint_root_revocation(root.clone(), &path, &target)
            .await
            .unwrap();
        let second = mint_root_revocation(root, &path, &target).await.unwrap();
        assert_eq!(first, second);
    }

    #[dialog_common::test]
    async fn it_rejects_a_target_that_is_not_a_link() {
        let (root, _, path) = root_grant().await;
        // The spec types `revoke` as `&Delegation`. A stringified CID is
        // the shape the previous encoding used, so it is exactly what a
        // stale client would send.
        let bytes = raw_revocation(
            root,
            &path,
            Promised::String(path.proof_cids()[0].to_string()),
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
            .issuer(dialog_credentials::Signer::from(root.clone()))
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
