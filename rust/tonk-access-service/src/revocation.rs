//! Revocation screening for presented UCAN containers.
//!
//! A presented container is parsed once into the CIDs, issuers, and
//! validity bounds that the expiry and revocation screens both need.
//! The revocation screen then decides whether the chain rests on a
//! delegation that one of its own issuers withdrew, by asking the
//! revocation index.

pub mod checker;
pub mod index;

use std::collections::BTreeSet;

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::AnySignature;

/// Credential CIDs and validity window presented to the presign endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCredentials {
    /// The invocation's subject — the space whose access the chain
    /// exercises. Carried so a refusal can name whose authority was
    /// withdrawn.
    pub subject: dialog_varsig::Did,
    /// CIDs of every referenced or carried delegation.
    pub delegation_cids: Vec<String>,
    /// Every principal that issued a delegation in this chain.
    ///
    /// A revocation applies here only when its subject is one of these:
    /// a principal who never issued into this chain held no authority
    /// over it, so their revocation was never about it. This is the
    /// spec's `delegators` set, with
    /// [ucan-wg/revocation#4](https://github.com/ucan-wg/revocation/pull/4)
    /// applied so the match binds on the revocation's subject rather
    /// than the invocation's issuer.
    pub delegators: BTreeSet<String>,
    /// Latest start bound in unix seconds.
    pub not_before: Option<u64>,
    /// Earliest expiration bound in unix seconds.
    pub expires_at: Option<u64>,
}

/// Parse a UCAN container once for both expiry and revocation screens.
#[cfg_attr(test, allow(dead_code))]
pub fn collect_presented(container_bytes: &[u8]) -> Result<PresentedCredentials, ContainerError> {
    let tokens = Container::from_bytes(container_bytes)?.into_tokens();
    let Some(invocation_bytes) = tokens.first() else {
        return Err(ContainerError::Invocation(
            "container must contain at least an invocation".to_string(),
        ));
    };
    let invocation: Invocation<AnySignature> = serde_ipld_dagcbor::from_slice(invocation_bytes)
        .map_err(|error| {
            ContainerError::Invocation(format!("failed to decode invocation: {error}"))
        })?;
    let mut delegation_cids = BTreeSet::new();
    let mut delegators = BTreeSet::new();
    delegation_cids.extend(invocation.proofs().iter().map(ToString::to_string));
    let mut not_before: Option<u64> = None;
    let mut expires_at = invocation.expiration().map(|stamp| stamp.to_unix());
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<AnySignature> =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|error| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {error}"))
            })?;
        delegation_cids.insert(delegation.to_cid().to_string());
        delegators.insert(delegation.issuer().to_string());
        if let Some(stamp) = delegation.not_before() {
            not_before = Some(not_before.map_or(stamp.to_unix(), |seen| seen.max(stamp.to_unix())));
        }
        if let Some(stamp) = delegation.expiration() {
            expires_at = Some(expires_at.map_or(stamp.to_unix(), |seen| seen.min(stamp.to_unix())));
        }
    }
    Ok(PresentedCredentials {
        subject: invocation.subject().clone(),
        delegation_cids: delegation_cids.into_iter().collect(),
        delegators,
        not_before,
        expires_at,
    })
}

/// Whether a presented chain rests on a delegation someone with
/// authority over it withdrew.
///
/// The spec's rule, per delegation: look the CID up, and if any subject
/// that revoked it is among this chain's issuers, ignore that
/// delegation. We hold one chain rather than a set of candidate paths,
/// so ignoring the only path we were given is a refusal.
///
/// An index failure is not a verdict. It surfaces as an error for the
/// caller to answer as its own unavailability, rather than as a claim
/// that anything was revoked.
pub async fn screen_revoked<I: index::RevocationIndex + dialog_common::ConditionalSync>(
    revocations: &I,
    presented: &PresentedCredentials,
) -> Result<Option<String>, index::IndexError> {
    if presented.delegators.is_empty() {
        // A chain with no delegations is rooted directly in its subject,
        // so there is no proof to withdraw.
        return Ok(None);
    }
    for cid in &presented.delegation_cids {
        if revocations
            .revoked_by_any(cid, &presented.delegators)
            .await?
        {
            return Ok(Some(cid.clone()));
        }
    }
    Ok(None)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::index::RevocationIndex as _;
    use super::*;

    /// A chain presenting `cids`, issued by `delegators`.
    fn presented(cids: &[&str], delegators: &[&str]) -> PresentedCredentials {
        PresentedCredentials {
            subject: "did:key:zSubject".parse().expect("test subject parses"),
            delegation_cids: cids.iter().map(|cid| (*cid).to_string()).collect(),
            delegators: delegators.iter().map(|did| (*did).to_string()).collect(),
            not_before: None,
            expires_at: None,
        }
    }

    /// The fork, end to end: real delegations, real containers, both
    /// chains screened.
    ///
    /// `space -> profile`, branching to Alice and to Bob. Alice revokes
    /// Bob's hop — which dialog's `validate` permits, since possession is
    /// the whole question there and Alice holds the capability.
    ///
    /// What must hold is that the recorded revocation reaches the chain
    /// Alice issued into and no other. The two halves of that live in
    /// different crates, so each looks correct read alone; this asserts
    /// them together against artifacts rather than stand-ins.
    #[dialog_common::test]
    async fn it_confines_a_revocation_to_chains_its_revoker_issued_into() {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_ucan_core::{DelegationBuilder, DelegationChain, InvocationBuilder};
        use dialog_varsig::Principal as _;

        async fn signer(seed: u8) -> Ed25519Signer {
            Ed25519Signer::import(&[seed; 32]).await.expect("a signer")
        }

        let space = signer(70).await;
        let profile = signer(71).await;
        let alice = signer(72).await;
        let bob = signer(73).await;

        let hop = async |issuer: &Ed25519Signer, audience: &Ed25519Signer| {
            DelegationBuilder::new()
                .issuer(dialog_credentials::Signer::from(issuer.clone()))
                .audience(&audience.did())
                .subject(UcanSubject::Specific(space.did()))
                .command(vec![])
                .try_build()
                .await
                .expect("a delegation")
        };

        let root = hop(&space, &profile).await;
        let to_alice = hop(&profile, &alice).await;
        let to_bob = hop(&profile, &bob).await;

        // The chain each guest presents when it syncs.
        let chain_of = |leaf: dialog_ucan_core::Delegation<AnySignature>| {
            DelegationChain::new(root.clone())
                .push(leaf)
                .expect("the hops connect")
        };
        let alices_chain = chain_of(to_alice.clone());
        let bobs_chain = chain_of(to_bob.clone());

        // Alice revokes BOB's hop. She is no party to it, but she holds
        // the capability, so the artifact itself is sound.
        let revocations = index::MemoryRevocationIndex::default();
        revocations
            .record(&to_bob.to_cid().to_string(), alice.did().as_ref())
            .await
            .expect("recorded");

        let present = async |chain: &DelegationChain, holder: &Ed25519Signer| {
            let invocation = InvocationBuilder::new()
                .issuer(dialog_credentials::Signer::from(holder.clone()))
                .audience(&space.did())
                .subject(&space.did())
                .command(vec!["test".to_string()])
                .arguments(std::collections::BTreeMap::new())
                .proofs(chain.proof_cids().to_vec())
                .try_build()
                .await
                .expect("an invocation");
            let mut tokens =
                vec![serde_ipld_dagcbor::to_vec(&invocation).expect("the invocation encodes")];
            for (_, delegation) in chain.export() {
                tokens.push(delegation.encoded().to_vec());
            }
            let bytes = Container::new(tokens).into_bytes().expect("a container");
            collect_presented(&bytes).expect("the container parses")
        };

        // Bob is untouched: Alice issued nothing into his chain, so her
        // revocation matches nothing in it.
        assert_eq!(
            screen_revoked(&revocations, &present(&bobs_chain, &bob).await)
                .await
                .unwrap(),
            None,
            "a sibling's revocation must not reach Bob"
        );

        // And the profile, which DID issue that hop, cuts Bob off — so the
        // assertion above is about authority, not an unmatchable CID.
        revocations
            .record(&to_bob.to_cid().to_string(), profile.did().as_ref())
            .await
            .expect("recorded");
        assert_eq!(
            screen_revoked(&revocations, &present(&bobs_chain, &bob).await)
                .await
                .unwrap(),
            Some(to_bob.to_cid().to_string()),
            "the issuer of the hop must be able to cut Bob off"
        );

        // Alice's own chain never named Bob's hop, so none of this touched
        // her access.
        assert_eq!(
            screen_revoked(&revocations, &present(&alices_chain, &alice).await)
                .await
                .unwrap(),
            None,
            "revoking Bob's hop must leave Alice's chain alone"
        );
    }

    /// A guest cannot cut off a sibling invite, even though the artifact
    /// verifies.
    ///
    /// Two layers answer this. Dialog's `validate` establishes only that the
    /// revoker HELD the capability, so a guest who proves possession may name
    /// any delegation of it — including one issued to somebody else. Nothing
    /// there ties the revoker to the hop it names.
    ///
    /// The screen is what makes that harmless: a revocation bites only where
    /// `revocation.sub` is among the issuers of the chain being presented. A
    /// guest never issued into a sibling's chain, so the row it wrote is
    /// inert — recorded, and matching nothing, forever.
    ///
    /// Pinned because the safety lives in the interaction between the two.
    /// Reading either alone suggests a guest can revoke a sibling's access.
    #[dialog_common::test]
    async fn it_ignores_a_revocation_by_a_principal_outside_the_chain() {
        let revocations = index::MemoryRevocationIndex::default();

        // space -> profile -> ephemeral-B is Bob's chain; Alice is the
        // audience of a sibling hop and issues into nothing here.
        revocations
            .record("bafyBobsInvite", "did:key:zAlice")
            .await
            .unwrap();

        let bobs_chain = presented(
            &["bafySpaceToProfile", "bafyBobsInvite"],
            &["did:key:zSpace", "did:key:zProfile"],
        );
        assert_eq!(
            screen_revoked(&revocations, &bobs_chain).await.unwrap(),
            None,
            "a revocation by a principal absent from the chain must not bite"
        );

        // And the same delegation revoked by someone who IS in the chain
        // does bite, so the assertion above is about authority rather than
        // about the CID being unmatchable.
        revocations
            .record("bafyBobsInvite", "did:key:zProfile")
            .await
            .unwrap();
        assert_eq!(
            screen_revoked(&revocations, &bobs_chain).await.unwrap(),
            Some("bafyBobsInvite".to_string()),
            "the profile issued the hop, so its revocation must bite"
        );
    }

    /// Revoking a powerline must cut off every chain it enables.
    ///
    /// `account -> profile` is `Subject::Any`: it is not scoped to a
    /// space, it is the hop that lets the profile act for the account
    /// everywhere. A chain like `space -> account -> profile` is only
    /// usable because that hop exists, so withdrawing it must deny the
    /// chain.
    ///
    /// The screen matches `revocation.sub` against the presented chain's
    /// ISSUER set. On `space -> account -> profile` the issuers are
    /// {space, account}, so a revocation recorded under `sub = account`
    /// matches and the chain is denied.
    ///
    /// Pinned because the recording side derives `sub` from a fallback
    /// (`path.issuer()` when the delegation is `Subject::Any`) rather
    /// than from anything the powerline itself states. The fallback lands
    /// on the right principal — the granting account issues into every
    /// chain its powerline enables — but a change to it would leave these
    /// chains live with nothing else failing.
    #[dialog_common::test]
    async fn it_denies_a_chain_running_through_a_revoked_powerline() {
        let revocations = index::MemoryRevocationIndex::default();
        // The powerline hop, revoked by the account that issued it.
        revocations
            .record("bafyPowerline", "did:key:zAccount")
            .await
            .unwrap();

        // space -> account -> profile: issuers {space, account}. The
        // powerline CID is carried as one of the presented proofs.
        let through = presented(
            &["bafySpaceToAccount", "bafyPowerline"],
            &["did:key:zSpace", "did:key:zAccount"],
        );
        assert_eq!(
            screen_revoked(&revocations, &through).await.unwrap(),
            Some("bafyPowerline".to_string()),
            "a chain resting on a revoked powerline must be denied"
        );
    }

    #[dialog_common::test]
    async fn it_passes_a_chain_nothing_revoked() {
        let revocations = index::MemoryRevocationIndex::default();
        let chain = presented(&["bafyA", "bafyB"], &["did:key:zAlice"]);
        assert_eq!(screen_revoked(&revocations, &chain).await.unwrap(), None);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_resting_on_a_withdrawn_delegation() {
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zAlice").await.unwrap();

        // Alice issued into this chain, so her revocation applies to it.
        let chain = presented(&["bafyA", "bafyB"], &["did:key:zAlice", "did:key:zBob"]);
        assert_eq!(
            screen_revoked(&revocations, &chain).await.unwrap(),
            Some("bafyB".to_string())
        );
    }

    #[dialog_common::test]
    async fn it_ignores_a_revocation_by_someone_outside_the_chain() {
        // The discriminating case. One target, one stored revocation,
        // two chains, two verdicts: `a` revoked `b`, so a chain `a` had
        // a hand in is refused, and a chain rooted elsewhere is not.
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zA").await.unwrap();

        let through_a = presented(&["bafyB"], &["did:key:zA", "did:key:zB"]);
        assert_eq!(
            screen_revoked(&revocations, &through_a).await.unwrap(),
            Some("bafyB".to_string())
        );

        let through_k = presented(&["bafyB"], &["did:key:zK", "did:key:zB"]);
        assert_eq!(
            screen_revoked(&revocations, &through_k).await.unwrap(),
            None,
            "a principal who never issued into this chain held no authority over it"
        );
    }

    #[dialog_common::test]
    async fn it_covers_every_delegation_the_chain_presents() {
        // Revoking the root of a chain refuses it just as revoking the
        // leaf does: the walk checks all of them, not only the last.
        let revocations = index::MemoryRevocationIndex::default();
        revocations
            .record("bafyRoot", "did:key:zAlice")
            .await
            .unwrap();

        let chain = presented(&["bafyRoot", "bafyLeaf"], &["did:key:zAlice"]);
        assert_eq!(
            screen_revoked(&revocations, &chain).await.unwrap(),
            Some("bafyRoot".to_string())
        );
    }

    #[dialog_common::test]
    async fn it_passes_a_chain_with_no_delegations_at_all() {
        // Rooted directly in its subject, so there is no proof to
        // withdraw and nothing to look up.
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zA").await.unwrap();

        let direct = presented(&[], &[]);
        assert_eq!(screen_revoked(&revocations, &direct).await.unwrap(), None);
    }
}
