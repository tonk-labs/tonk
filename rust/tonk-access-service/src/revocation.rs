//! Revocation: what was withdrawn, and who withdrew it.
//!
//! The index records the facts, and [`checker::IndexedRevocations`]
//! answers dialog's question from them. Nothing here screens a chain:
//! the authorizer carries the checker, so revocation is asked inside
//! the chain walk, per link, against the principals entitled to revoke
//! that link.
//!
//! That per-link scoping is why it belongs there rather than here. A
//! screen outside the walk sees one flat set of issuers and must apply
//! it to every hop, which for `a -> b -> c -> d` would let `d`'s issuer
//! revoke `c`: a principal that merely *received* authority revoking
//! the grant it depends on. Authority to revoke flows downward, and
//! only the walk knows which direction is down.

pub mod checker;
pub mod index;

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;

    use dialog_credentials::{DidKeyResolver, Ed25519Signer, Signer};
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{
        Container, Delegation, DelegationBuilder, DelegationChain, Environment, InvocationBuilder,
        InvocationChain, VerificationContext,
    };
    use dialog_varsig::{AnySignature, Principal as _};

    use super::checker::IndexedRevocations;
    use super::index::{MemoryRevocationIndex, RevocationIndex as _};

    async fn signer(seed: u8) -> Ed25519Signer {
        Ed25519Signer::import(&[seed; 32]).await.expect("a signer")
    }

    /// Verify a presented chain against `revocations`, exactly as the
    /// authorizer does: same environment, same checker, same walk.
    ///
    /// Answers whether the chain was accepted, so a test reads as the
    /// verdict a presign would get.
    async fn accepted(
        revocations: &MemoryRevocationIndex,
        container_bytes: &[u8],
    ) -> Result<bool, String> {
        let chain = InvocationChain::<AnySignature>::try_from(container_bytes)
            .map_err(|error| error.to_string())?;
        let checker = IndexedRevocations(revocations);
        let environment = Environment::new(chain.proof_store(), DidKeyResolver, &checker);
        let context = VerificationContext::new(&environment);
        match chain.verify(&context).await {
            Ok(_) => Ok(true),
            Err(dialog_ucan_core::ContainerError::Revoked { .. }) => Ok(false),
            Err(other) => Err(other.to_string()),
        }
    }

    /// The container a holder presents when it syncs.
    async fn present(
        chain: &DelegationChain,
        holder: &Ed25519Signer,
        space: &Ed25519Signer,
    ) -> Vec<u8> {
        let invocation = InvocationBuilder::new()
            .issuer(Signer::from(holder.clone()))
            .audience(&space.did())
            .subject(&space.did())
            .command(vec!["test".to_string()])
            .arguments(BTreeMap::new())
            .proofs(chain.proof_cids().to_vec())
            .try_build()
            .await
            .expect("an invocation");
        let mut tokens =
            vec![serde_ipld_dagcbor::to_vec(&invocation).expect("the invocation encodes")];
        for (_, delegation) in chain.export() {
            tokens.push(delegation.encoded().to_vec());
        }
        Container::new(tokens).into_bytes().expect("a container")
    }

    /// The fork, end to end: real delegations, real containers, both
    /// chains verified.
    ///
    /// `space -> profile`, branching to Alice and to Bob. Alice revokes
    /// Bob's hop, which dialog's `validate` permits: possession is the
    /// whole question there, and Alice holds the capability.
    ///
    /// What must hold is that the recorded revocation reaches the chain
    /// its revoker had authority over and no other. The two halves of
    /// that live in different crates, so each looks correct read alone;
    /// this asserts them together against artifacts rather than
    /// stand-ins.
    #[dialog_common::test]
    async fn it_confines_a_revocation_to_chains_its_revoker_had_authority_over() {
        let space = signer(70).await;
        let profile = signer(71).await;
        let alice = signer(72).await;
        let bob = signer(73).await;

        let hop = async |issuer: &Ed25519Signer, audience: &Ed25519Signer| {
            DelegationBuilder::new()
                .issuer(Signer::from(issuer.clone()))
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

        let chain_of = |leaf: Delegation<AnySignature>| {
            DelegationChain::new(root.clone())
                .push(leaf)
                .expect("the hops connect")
        };
        let alices_chain = chain_of(to_alice.clone());
        let bobs_chain = chain_of(to_bob.clone());

        let bobs_container = present(&bobs_chain, &bob, &space).await;
        let alices_container = present(&alices_chain, &alice, &space).await;

        // Alice revokes BOB's hop. She is no party to it, but she holds
        // the capability, so the artifact itself is sound.
        let revocations = MemoryRevocationIndex::default();
        revocations
            .record(&to_bob.to_cid().to_string(), alice.did().as_ref())
            .await
            .expect("recorded");

        assert!(
            accepted(&revocations, &bobs_container).await.unwrap(),
            "a sibling's revocation must not reach Bob"
        );

        // And the profile, which DID issue that hop, cuts Bob off, so the
        // assertion above is about authority rather than an unmatchable CID.
        revocations
            .record(&to_bob.to_cid().to_string(), profile.did().as_ref())
            .await
            .expect("recorded");
        assert!(
            !accepted(&revocations, &bobs_container).await.unwrap(),
            "the issuer of the hop must be able to cut Bob off"
        );

        // Alice's own chain never named Bob's hop, so none of this touched
        // her access.
        assert!(
            accepted(&revocations, &alices_container).await.unwrap(),
            "revoking Bob's hop must leave Alice's chain alone"
        );
    }

    /// Authority to revoke flows downward, and only downward.
    ///
    /// The revocation spec's pseudocode computes one `delegators` set for
    /// the whole chain, which applied uniformly lets a principal revoke
    /// the grant it depends on: for `space -> profile -> bob`, Bob could
    /// withdraw the hop that gave the profile anything at all. Dialog
    /// scopes the candidates per link instead, so Bob is not a candidate
    /// when the hop above him is checked.
    ///
    /// Pinned because a flat screen passes every other test here and
    /// fails only this one.
    #[dialog_common::test]
    async fn it_refuses_to_let_a_recipient_revoke_the_grant_above_it() {
        let space = signer(80).await;
        let profile = signer(81).await;
        let bob = signer(82).await;

        let hop = async |issuer: &Ed25519Signer, audience: &Ed25519Signer| {
            DelegationBuilder::new()
                .issuer(Signer::from(issuer.clone()))
                .audience(&audience.did())
                .subject(UcanSubject::Specific(space.did()))
                .command(vec![])
                .try_build()
                .await
                .expect("a delegation")
        };

        let root = hop(&space, &profile).await;
        let to_bob = hop(&profile, &bob).await;
        let chain = DelegationChain::new(root.clone())
            .push(to_bob)
            .expect("the hops connect");
        let container = present(&chain, &bob, &space).await;

        // Bob names the ROOT hop, which he received authority through but
        // was never a party to.
        let revocations = MemoryRevocationIndex::default();
        revocations
            .record(&root.to_cid().to_string(), bob.did().as_ref())
            .await
            .expect("recorded");
        assert!(
            accepted(&revocations, &container).await.unwrap(),
            "a recipient must not revoke the grant its own authority rests on"
        );

        // The space issued that hop, so its revocation does bite. Same
        // target, same chain, different revoker.
        revocations
            .record(&root.to_cid().to_string(), space.did().as_ref())
            .await
            .expect("recorded");
        assert!(
            !accepted(&revocations, &container).await.unwrap(),
            "the issuer of the root hop must be able to withdraw it"
        );
    }

    /// A recipient may always disclaim what it was given.
    ///
    /// The mirror of the rule above: Bob cannot revoke the hop his
    /// authority rests on, but he can revoke the hop naming him, because
    /// its audience is entitled to hand it back.
    #[dialog_common::test]
    async fn it_lets_an_audience_disclaim_its_own_grant() {
        let space = signer(90).await;
        let bob = signer(91).await;

        let to_bob = DelegationBuilder::new()
            .issuer(Signer::from(space.clone()))
            .audience(&bob.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .expect("a delegation");
        let chain = DelegationChain::new(to_bob.clone());
        let container = present(&chain, &bob, &space).await;

        let revocations = MemoryRevocationIndex::default();
        assert!(
            accepted(&revocations, &container).await.unwrap(),
            "the chain must verify before anything is revoked"
        );

        revocations
            .record(&to_bob.to_cid().to_string(), bob.did().as_ref())
            .await
            .expect("recorded");
        assert!(
            !accepted(&revocations, &container).await.unwrap(),
            "the audience of a grant may withdraw it"
        );
    }

    /// Revoking a hop the chain never presents leaves it alone.
    #[dialog_common::test]
    async fn it_passes_a_chain_nothing_it_presents_was_revoked_in() {
        let space = signer(100).await;
        let bob = signer(101).await;

        let to_bob = DelegationBuilder::new()
            .issuer(Signer::from(space.clone()))
            .audience(&bob.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .expect("a delegation");
        let chain = DelegationChain::new(to_bob);
        let container = present(&chain, &bob, &space).await;

        let revocations = MemoryRevocationIndex::default();
        revocations
            .record("bafySomethingElse", space.did().as_ref())
            .await
            .expect("recorded");
        assert!(
            accepted(&revocations, &container).await.unwrap(),
            "a revocation naming a hop this chain never presents must not bite"
        );
    }
}
