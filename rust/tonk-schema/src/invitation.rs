//! [`Invitation`] — the durable record of a minted invite.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_ucan_core::DelegationChain;
use serde::Serialize;

use crate::domain::invitation::{Audience, Inviter, PathHex, Subject, TargetCid};
use crate::prelude::*;

/// An invitation — the durable record of a minted invite to a
/// repository. Lives on the repository's meta branch.
///
/// The `this` entity is content-derived from the CID of the **leaf
/// delegation of the chain as serialized into the invite URL**. The
/// minter holds that chain because it built it; a claimer holds it
/// because it parsed the URL — so both sides derive the same entity
/// independently, and a claimer can assert (or reference) the record
/// without any lookup. Open-invite claims push a redelegation onto
/// the chain, so claim paths must derive the invitation from the
/// chain *as parsed*, before claiming.
///
/// `subject`, `inviter`, and `audience` repeat chain-derivable data
/// as queryable attributes — same redundant-by-design rationale as
/// [`Replica`](crate::Replica).
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Invitation {
    /// The invitation's entity. Derived from the leaf delegation CID.
    pub this: Entity,
    /// Reference to the repository the invite grants access to.
    pub subject: Subject,
    /// Reference to the minting profile's root identity on the chain as
    /// minted, before any claim redelegation extends it.
    pub inviter: Inviter,
    /// The chain's tail audience: the ephemeral key DID for open
    /// invites, the recipient DID for scoped ones.
    pub audience: Audience,
    /// Canonical CID of the invitation delegation that closes this route.
    pub target_cid: TargetCid,
    /// Exact public delegation path through the target, hex encoded.
    pub path_hex: PathHex,
}

/// Hash input for [`Invitation::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    Invitation { delegation: &'a str },
}

impl Invitation {
    /// Build the invitation record for a delegation chain — the chain
    /// exactly as serialized into the invite URL.
    ///
    /// Returns `None` when the chain has no specific subject
    /// (`Subject::Any` chains are not valid invites; `tonk_invite::
    /// Invite` already rejects them at construction, so callers
    /// holding an `Invite` can expect `Some`).
    pub fn from_chain(chain: &DelegationChain) -> Option<Self> {
        let subject = chain.subject()?.clone();
        let leaf_cid = chain
            .proof_cids()
            .last()
            .expect("delegation chains are non-empty by construction");
        // Root-first chains begin `space → root → device`. Membership and
        // provenance are account semantics, so attribute the invite to the
        // first hop's audience (the root), never the device that happened to
        // sign the final invite hop. Legacy one-hop chains naturally retain
        // their original audience as the best available identity.
        let inviter = chain
            .proofs()
            .next()
            .expect("delegation chains are non-empty by construction")
            .audience()
            .clone();
        let audience = chain.audience().clone();
        // The canonical CID string, not the raw bytes: it keeps the
        // hash input human-inspectable and independent of `Cid`'s
        // serde representation.
        let delegation = leaf_cid.to_string();
        let path_hex = hex::encode(chain.to_bytes().ok()?);
        Some(Self {
            this: Entity::of(&This::Invitation {
                delegation: &delegation,
            }),
            subject: Subject(subject.this()),
            inviter: Inviter(inviter.this()),
            audience: Audience(audience.this()),
            target_cid: TargetCid(delegation),
            path_hex: PathHex(path_hex),
        })
    }

    /// The invitation's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for Invitation {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_varsig::{Did, Principal};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const INVITER_SEED: [u8; 32] = [1u8; 32];
    const EPHEMERAL_SEED: [u8; 32] = [2u8; 32];
    const CLAIMER_SEED: [u8; 32] = [3u8; 32];
    const SUBJECT_SEED: [u8; 32] = [4u8; 32];

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    /// Single-hop chain `inviter -> ephemeral`, scoped to `subject` —
    /// the shape a minted open invite carries in its URL.
    async fn minted_chain(subject: &Did) -> DelegationChain {
        let inviter = signer(&INVITER_SEED).await;
        let ephemeral = signer(&EPHEMERAL_SEED).await;
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(inviter))
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    #[dialog_common::test]
    async fn it_derives_the_same_entity_from_minted_and_reparsed_chains() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let chain = minted_chain(&subject).await;
        let reparsed = DelegationChain::try_from(chain.to_bytes().unwrap().as_slice()).unwrap();

        let minted = Invitation::from_chain(&chain).unwrap();
        let claimed = Invitation::from_chain(&reparsed).unwrap();
        assert_eq!(minted.this, claimed.this);
        assert_eq!(minted, claimed);
    }

    #[dialog_common::test]
    async fn it_keys_execution_metadata_to_the_invitation_entity() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let invitation = Invitation::from_chain(&minted_chain(&subject).await).unwrap();
        let execution = crate::InvitationExecution::new(&invitation, "open");

        assert_eq!(execution.this, invitation.this);
        assert_eq!(execution.kind.0, "open");
    }

    #[dialog_common::test]
    async fn it_derives_a_different_entity_after_a_claim_redelegation() {
        // Open-invite claims push `ephemeral -> claimer` onto the
        // chain; the leaf changes, so the derived entity changes.
        // Claim paths must therefore derive from the chain as parsed,
        // before claiming.
        let subject = signer(&SUBJECT_SEED).await.did();
        let chain = minted_chain(&subject).await;
        let before = Invitation::from_chain(&chain).unwrap();

        let ephemeral = signer(&EPHEMERAL_SEED).await;
        let claimer = signer(&CLAIMER_SEED).await.did();
        let redelegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(ephemeral))
            .audience(&claimer)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let extended = chain.push(redelegation).unwrap();
        let after = Invitation::from_chain(&extended).unwrap();

        assert_ne!(before.this, after.this);
    }

    #[dialog_common::test]
    async fn it_reads_subject_inviter_and_audience_from_the_chain() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral = signer(&EPHEMERAL_SEED).await.did();
        let chain = minted_chain(&subject).await;

        let invitation = Invitation::from_chain(&chain).unwrap();
        assert_eq!(invitation.subject.0.to_string(), subject.as_str());
        assert_eq!(invitation.inviter.0.to_string(), ephemeral.as_str());
        assert_eq!(invitation.audience.0.to_string(), ephemeral.as_str());
    }

    #[dialog_common::test]
    async fn it_attributes_a_multi_hop_invite_to_the_root_not_the_device() {
        let space = signer(&SUBJECT_SEED).await;
        let root = signer(&INVITER_SEED).await;
        let device = signer(&CLAIMER_SEED).await;
        let ephemeral = signer(&EPHEMERAL_SEED).await;
        let first = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&root.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let root_to_device = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(root.clone()))
            .audience(&device.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let device_to_invite = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(device.clone()))
            .audience(&ephemeral.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(first)
            .push(root_to_device)
            .unwrap()
            .push(device_to_invite)
            .unwrap();

        let invitation = Invitation::from_chain(&chain).unwrap();
        assert_eq!(invitation.inviter.0.to_string(), root.did().as_str());
        assert_ne!(invitation.inviter.0.to_string(), device.did().as_str());
    }

    #[dialog_common::test]
    async fn it_returns_none_for_a_subjectless_chain() {
        let inviter = signer(&INVITER_SEED).await;
        let ephemeral = signer(&EPHEMERAL_SEED).await.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(inviter))
            .audience(&ephemeral)
            .subject(UcanSubject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(delegation);
        assert!(Invitation::from_chain(&chain).is_none());
    }
}
