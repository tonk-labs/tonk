//! Self-contained signed revocation artifacts.
//!
//! Every artifact names the delegation it withdraws and carries the ordered
//! delegation path that witnesses that target. Consumers can therefore verify
//! revocation authority without consulting an account provider or registry.

use std::collections::HashMap;

use anyhow::Result;
use dialog_credentials::{DidKeyResolver, Signer};
use dialog_ucan_core::container::revocation::{RevocationChain, RevocationError};
use dialog_ucan_core::revocation::action::Revocation;
use dialog_ucan_core::revocation::builder::RevocationBuilder;
use std::sync::Arc;
// Mirrors dialog's own target split: WASM Ed25519 keys carry a `JsValue`
// and are `!Send`, so futures are local there and boxed elsewhere.
#[cfg(target_arch = "wasm32")]
use dialog_ucan_core::future::Local as Runtime;
#[cfg(not(target_arch = "wasm32"))]
use dialog_ucan_core::future::Sendable as Runtime;
use dialog_ucan_core::revocation::UnverifiedRevocations;
use dialog_ucan_core::verification::{Environment, VerificationContext};
use dialog_ucan_core::{Delegation, DelegationChain, InvocationChain};
use dialog_varsig::AnySignature;
use dialog_varsig::Did;
use ipld_core::cid::Cid;

/// The command a revocation invokes.
pub const REVOKE_COMMAND: [&str; 2] = ["ucan", "revoke"];

/// The argument naming the withdrawn delegation.
///
/// `rev`, not `revoke`: the spec's IPLD schema is normative and uses the
/// abbreviated wire names, matching `cmd` / `nnc` / `prf` elsewhere in
/// the envelope. The prose in the spec README spells them out, which is
/// where the longer names came from.
pub const REVOKE_ARGUMENT: &str = "rev";

/// The argument carrying the delegation-path witness.
pub const PATH_ARGUMENT: &str = "pth";

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
    /// Subject of the delegation being withdrawn — what capability it
    /// granted, as against [`subject`](Self::subject), which is who is
    /// withdrawing it.
    ///
    /// A powerline (`Subject::Any`) names no subject of its own, so this
    /// reports the principal that issued it — the authority the grant
    /// actually conveys, and the one a chain resting on it descends from.
    /// Device grants are powerlines, so this is the common case rather
    /// than an edge one.
    pub revoked_subject: Did,
    /// Every delegation CID this revocation rests on: the witnessed path
    /// and any attached proof chain.
    ///
    /// Whether the revoker's OWN authority still stands is a question
    /// about the revocation index, not about these bytes — a delegation
    /// that was validly issued and later withdrawn still verifies here.
    /// So the recorder screens these against the index before accepting,
    /// the same way the presign path screens a chain it is handed.
    pub path_cids: Vec<String>,
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

/// Pack a built revocation plus the blocks it names into a container.
///
/// Assembly and serialization are `RevocationChain`'s: it knows that the
/// witness is named by `args.pth` rather than `prf`, which is exactly the
/// distinction a generic invocation writer misses.
fn package(
    revocation: Revocation<AnySignature>,
    path: &DelegationChain,
    proofs: Option<&DelegationChain>,
) -> Result<Vec<u8>> {
    let blocks: HashMap<Cid, Arc<Delegation<AnySignature>>> = path
        .export()
        .chain(proofs.into_iter().flat_map(|chain| chain.export()))
        .collect();

    RevocationChain::assemble(revocation, blocks)
        .map_err(|err| anyhow::anyhow!("failed to assemble the revocation: {err}"))?
        .to_bytes()
        .map_err(|err| anyhow::anyhow!("failed to serialize the revocation: {err}"))
}

/// Mint a proofless revocation signed by a principal in the witnessed path.
///
/// The signer is the revocation's subject: this artifact is about *their*
/// withdrawal, not about the capability being withdrawn. Whether they were
/// entitled to withdraw it is [`verify`]'s question, answered against the
/// witness path.
pub async fn mint_root_revocation(
    root: impl Into<Signer>,
    path: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    let root: Signer = root.into();
    let revocation = RevocationBuilder::new(root, *target)
        .path(path.proof_cids().to_vec())
        .try_build()
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;
    package(revocation, path, None)
}

/// Mint a device-signed revocation of the device's own grant.
pub async fn mint_self_revocation(
    device: impl Into<Signer>,
    grant: &DelegationChain,
    target: &Cid,
) -> Result<Vec<u8>> {
    let device: Signer = device.into();
    let subject = grant
        .subject()
        .cloned()
        .unwrap_or_else(|| grant.issuer().clone());
    let revocation = RevocationBuilder::new(device, *target)
        .path(grant.proof_cids().to_vec())
        .try_build_with_proofs(grant.proof_cids().to_vec(), &subject)
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;
    package(revocation, grant, Some(grant))
}

/// Mint a revocation signed under an attached delegation proof chain.
///
/// `prf` answers "may this principal invoke at all"; `pth` answers "why may
/// they revoke *this*". They can rest on different grants.
pub async fn mint_delegated_revocation(
    issuer: impl Into<Signer>,
    path: &DelegationChain,
    target: &Cid,
    proofs: &DelegationChain,
) -> Result<Vec<u8>> {
    let issuer: Signer = issuer.into();
    let subject = proofs
        .subject()
        .cloned()
        .unwrap_or_else(|| proofs.issuer().clone());
    let revocation = RevocationBuilder::new(issuer, *target)
        .path(path.proof_cids().to_vec())
        .try_build_with_proofs(proofs.proof_cids().to_vec(), &subject)
        .await
        .map_err(|err| anyhow::anyhow!("failed to mint the revocation: {err}"))?;
    package(revocation, path, Some(proofs))
}

/// Parse and verify a self-contained revocation artifact.
pub async fn verify(bytes: &[u8]) -> std::result::Result<VerifiedRevocation, VerifyError> {
    let chain = InvocationChain::<AnySignature>::try_from(bytes)
        .map_err(|err| VerifyError::Malformed(format!("bad invocation container: {err}")))?;

    // Shape: the command, the empty nonce, `rev` as a link, `pth` as a list
    // of links, and every named block present in the container.
    let revocation = RevocationChain::try_from(chain.clone())
        .map_err(|err| VerifyError::Malformed(err.to_string()))?;

    // Everything else is dialog's: the invocation's own validity (signature,
    // `prf` chain, time bounds) and the witness path's (linkage, rooting,
    // expiry, and that the revoker held what it revokes). This module used to
    // hand-roll those, and each hand-rolled copy was missing something the
    // shared implementation already did.
    //
    // `UnverifiedRevocations` because this entry point answers "is this
    // artifact sound", not "does the revoker's own authority still stand" —
    // the latter needs an index, which the access service supplies where it
    // screens.
    let environment = Environment::new(
        revocation.chain().proof_store(),
        DidKeyResolver,
        UnverifiedRevocations,
    );
    let context = VerificationContext::new(&environment);
    revocation
        .verify::<Runtime, _, _, _>(&context)
        .await
        .map_err(|err| match err {
            RevocationError::Invalid(_) | RevocationError::Denied(_) => {
                VerifyError::Unauthorized(err.to_string())
            }
            // We could not establish a finding; that is our reach, not a
            // statement about their material.
            RevocationError::Unavailable { .. } => VerifyError::Malformed(err.to_string()),
        })?;

    let target = *revocation.revocation().revoked();
    let target_expires_at = revocation
        .revoked()
        .expiration()
        .map(|expiration| expiration.to_unix());

    Ok(VerifiedRevocation {
        target_cid: target.to_string(),
        artifact_cid: chain.invocation.to_cid().to_string(),
        target_expires_at,
        issuer: chain.issuer().clone(),
        subject: revocation.revocation().revoker().clone(),
        revoked_subject: match revocation.revoked().subject() {
            dialog_ucan_core::subject::Subject::Specific(did) => did.clone(),
            dialog_ucan_core::subject::Subject::Any => revocation.revoked().issuer().clone(),
        },
        path_cids: revocation
            .revocation()
            .path()
            .iter()
            .chain(chain.proofs().iter())
            .map(ToString::to_string)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_ucan_core::Container;
    use dialog_ucan_core::crypto::nonce::Nonce;
    use dialog_ucan_core::promise::Promised;
    use std::collections::BTreeMap;

    /// The command a revocation carries, per the spec's `cmd "/ucan/revoke"`.
    fn command() -> Vec<String> {
        vec!["ucan".to_string(), "revoke".to_string()]
    }

    /// `nnc ""`: revocation is idempotent, so the same withdrawal by the
    /// same principal is the same artifact.
    fn nonce() -> Nonce {
        Nonce::Custom(Vec::new())
    }

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::time::timestamp::{Duration, SystemTime, Timestamp};
    use dialog_ucan_core::{DelegationBuilder, InvocationBuilder};
    use dialog_varsig::Principal as _;
    use std::collections::BTreeSet;

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
        // The revoker is the subject: this artifact is about their
        // withdrawal, not about the capability being withdrawn. Built by
        // hand so a test can inject a `rev` or `pth` the builder would
        // never produce, but faithful to the builder in every other way —
        // otherwise these cases fail at the shape gate and never reach the
        // authority question they exist to ask.
        let signer: Signer = issuer.into();
        let revoker = signer.did();
        let invocation = InvocationBuilder::new()
            .issuer(signer)
            .audience(&revoker)
            .subject(&revoker)
            .command(command())
            .arguments(args)
            .proofs(
                proofs
                    .map(|chain| chain.proof_cids().to_vec())
                    .unwrap_or_default(),
            )
            .nonce(nonce())
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
        let outcome = verify(&bytes).await;
        eprintln!("OUTSIDER => {outcome:?}");
        assert!(matches!(outcome, Err(VerifyError::Unauthorized(_))));
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
    }

    #[dialog_common::test]
    async fn it_lets_a_holder_revoke_a_hop_it_descends_from() {
        // Verification establishes POSSESSION, not a relationship to the
        // hop being revoked: holding the capability, the member could
        // always have issued the hop itself, so its absence proves
        // nothing. So the member may name the hop above its own.
        //
        // What keeps that from cutting off anyone else is the presign
        // screen, not this function: a revocation bites only where its
        // revoker issued into the chain being presented. See
        // `it_ignores_a_revocation_by_a_principal_outside_the_chain` in
        // tonk-access-service, which pins the other half.
        let (_, member, _, path) = invite_path().await;
        let above = path.proof_cids()[0];

        let bytes = raw_revocation(member, &path, Promised::Link(above), None).await;
        assert!(
            verify(&bytes).await.is_ok(),
            "a holder of the capability may withdraw a hop it descends from"
        );
    }

    #[dialog_common::test]
    async fn it_records_the_revoker_as_subject() {
        // `sub` is WHO IS REVOKING, not what the revoked delegation was
        // about. Those are different questions, and filling the field from
        // the second meant nothing downstream could tell them apart.
        //
        // The screen matches this against the issuers of a presented chain:
        // a revocation bites where its revoker issued, and nowhere else.
        let (_, _, invite, path) = invite_path().await;
        let target = path.proof_cids()[1];
        let verified = verify(
            &mint_self_revocation(invite.clone(), &path, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(verified.issuer, invite.did(), "the invite key signed it");
        assert_eq!(
            verified.subject,
            invite.did(),
            "and it is the revoker, so `sub` names it too"
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
    async fn it_refuses_a_witness_that_does_not_start_at_the_subject() {
        // Linkage makes the hops connect; it does not say WHERE they
        // start. A witness rooted in a principal the subject never
        // delegated to is a well-formed chain about somebody else's
        // authority, so it must not authorize a revocation whose subject
        // is this space.
        let space = signer(3).await;
        let stranger = signer(13).await;
        let accomplice = signer(14).await;
        let victim = signer(5).await;

        // stranger -> accomplice -> victim: connected, every signature
        // valid, and it names the space as subject. But the space never
        // issued into it.
        let first = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(stranger.clone()))
            .audience(&accomplice.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let second = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(accomplice.clone()))
            .audience(&victim.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let target = second.to_cid();
        let bytes = raw_revocation_with_path(
            accomplice,
            &space.did(),
            &[&first, &second],
            Promised::Link(target),
        )
        .await;
        assert!(
            verify(&bytes).await.is_err(),
            "a witness that does not start at the subject must not authorize \
             a revocation against that subject"
        );
    }

    #[dialog_common::test]
    async fn it_records_the_issuer_as_subject_for_a_powerline_revocation() {
        // A powerline (`Subject::Any`) has no subject of its own, so the
        // revocation's `sub` falls back to the path's issuer — the
        // account that granted it. Verification is unaffected: the screen
        // matches `sub` against the victim chain's issuer set, and the
        // account issues into every chain the powerline enables
        // (`space -> account -> profile`), so the match lands.
        //
        // It is however a LOSSY encoding. "scoped to subject X" and
        // "about a powerline, which has no subject, and X merely issued
        // it" both come out as `sub = X`, so nothing downstream can tell
        // them apart without knowing the fallback exists. The evidence
        // record keys powerlines under `_` precisely to keep that
        // distinction somewhere.
        //
        // Pinned because it is a default rather than a stated fact.
        let account = signer(15).await;
        let profile = signer(16).await;
        let powerline = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(account.clone()))
            .audience(&profile.did())
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(powerline);
        let target = chain.proof_cids()[0];

        let verified = verify(
            &mint_root_revocation(account.clone(), &chain, &target)
                .await
                .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            verified.subject,
            account.did(),
            "a powerline revocation must be recorded under the granting \
             account, which is the issuer every enabled chain carries"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_revocation_from_a_principal_outside_the_chain() {
        // The attacker holds a real keypair and can sign anything it
        // likes — but it never appears in the chain leading to the
        // target, so it holds no authority to withdraw it. Signing
        // validly is not the same as being authorized.
        let (_, _, _, path) = invite_path().await;
        let attacker = signer(21).await;
        let target = path.proof_cids()[1];

        // The attacker can mint whatever it likes — signing is not
        // authorization — so the refusal has to come from verification.
        let bytes = raw_revocation(attacker, &path, Promised::Link(target), None).await;
        assert!(
            matches!(verify(&bytes).await, Err(VerifyError::Unauthorized(_))),
            "a validly signed revocation from outside the chain must not verify"
        );
    }

    #[dialog_common::test]
    async fn it_refuses_a_revocation_witnessed_by_an_expired_delegation() {
        // Authority that has lapsed is not authority. An attacker who
        // once held a delegation, or who presents one whose window has
        // closed, must not be able to withdraw anything with it.
        let space = signer(3).await;
        let member = signer(4).await;
        let invite = signer(5).await;
        let past = Timestamp::new(SystemTime::now() - Duration::from_secs(3600)).unwrap();

        let expired = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&member.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .expiration(past)
            .try_build()
            .await
            .unwrap();
        let onward = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(member.clone()))
            .audience(&invite.did())
            .subject(Subject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(expired).push(onward).unwrap();
        // The invite revokes the EXPIRED hop, which it is neither issuer
        // nor audience of — so authority does not settle outright and the
        // walk has to run, over a hop whose window has closed.
        let target = chain.proof_cids()[0];

        let bytes = raw_revocation(invite, &chain, Promised::Link(target), None).await;
        assert!(
            verify(&bytes).await.is_err(),
            "a witness whose hop has expired must not establish authority"
        );
    }

    #[dialog_common::test]
    async fn it_rejects_an_unauthorized_issuer() {
        let (_, _, _, path) = invite_path().await;
        let outsider = signer(8).await;
        let bytes =
            raw_revocation(outsider, &path, Promised::Link(path.proof_cids()[1]), None).await;

        let outcome = verify(&bytes).await;
        eprintln!("TONKPROBE outsider => {outcome:?}");
        assert!(matches!(outcome, Err(VerifyError::Unauthorized(_))));
        #[allow(unreachable_code)]
        {}
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
