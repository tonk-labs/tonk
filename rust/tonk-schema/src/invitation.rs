//! [`Invitation`] — the durable record of a minted invite.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::invitation::{Audience, Inviter, Subject};
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
/// It is deliberately NOT the certificate entity the retained
/// delegation facts hang off. That one is `blob:<blake3>` of the
/// envelope, and blake3 is the branch blob store's hash, not
/// something a claimer can derive from a URL it has only parsed.
/// Keying on it would make the record unwritable until the chain had
/// been retained into a branch, which the claim path has not done and
/// must not have to do.
///
/// `subject`, `inviter`, and `audience` repeat chain-derivable data
/// as queryable attributes — same redundant-by-design rationale as
/// [`Replica`](crate::Replica). The delegation itself is NOT repeated
/// here: retaining the chain decomposes every certificate into
/// `dialog.ucan/*` facts and an envelope blob on the branch, which is
/// the authoritative copy a proof search reads. A second hand-rolled
/// copy here could only drift from it, and did: it recorded the leaf
/// hop, which a re-delegating claimer never presents.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Invitation {
    /// The invitation's entity. Derived from the leaf delegation CID.
    pub this: Entity,
    /// Reference to the repository the invite grants access to.
    pub subject: Subject,
    /// Reference to the minting account bound to the leaf signer, before
    /// any claim redelegation extends it. Legacy invitations without signed
    /// attribution retain their first audience, which may be the original owner.
    pub inviter: Inviter,
    /// The chain's tail audience: the ephemeral key DID for open
    /// invites, the recipient DID for scoped ones. Revocation proves
    /// from this principal back toward the subject, so it is the handle
    /// onto the retained delegation facts.
    pub audience: Audience,
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
    /// Returns `None` when signed inviter metadata is invalid or the chain has
    /// no specific subject. `tonk_invite::Invite` rejects both at construction,
    /// so callers holding an unmodified `Invite` can expect `Some`.
    pub fn from_chain(chain: &DelegationChain) -> Option<Self> {
        let subject = chain.subject()?.clone();
        let leaf_cid = chain
            .proof_cids()
            .last()
            .expect("delegation chains are non-empty by construction");
        // New invitations bind the minting account to the leaf signer.
        // Preserve historical records for leaves without this metadata:
        // their first audience was used as the inviter, even on reshares.
        let inviter = tonk_invite::inviter(chain).ok()?.unwrap_or_else(|| {
            chain
                .proofs()
                .next()
                .expect("delegation chains are non-empty by construction")
                .audience()
                .clone()
        });
        let audience = chain.audience().clone();
        // The canonical CID string, not the raw bytes: it keeps the
        // hash input human-inspectable and independent of `Cid`'s
        // serde representation.
        let delegation = leaf_cid.to_string();
        Some(Self {
            this: Entity::of(&This::Invitation {
                delegation: &delegation,
            }),
            subject: Subject(subject.this()),
            inviter: Inviter(inviter.this()),
            audience: Audience(audience.this()),
        })
    }

    /// The invitation's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

/// The account that issued an invite: whoever signed the chain's leaf,
/// lifted to the account behind it when the signer is a device.
///
/// A device acts for its account through a powerline grant
/// (`account -> device`, any subject), so a leaf signed by a device whose
/// grant is the hop before it is the account's invite. Otherwise the
/// leaf's signer issued it directly.
///
/// Unlike [`Invitation::inviter`], which records the first hop's
/// audience, this follows the chain to its end: a member who re-shares
/// the space extends the chain past the founder, and the invite is the
/// member's, not the founder's. Takes the chain as minted, before a
/// claim redelegates it.
pub fn inviting_account(chain: &DelegationChain) -> Did {
    let hops: Vec<_> = chain.proofs().collect();
    let leaf = hops
        .last()
        .expect("delegation chains are non-empty by construction");
    let signer = leaf.issuer();
    match hops.iter().rev().nth(1) {
        Some(grant)
            if grant.audience() == signer
                && *grant.subject() == dialog_ucan_core::subject::Subject::Any =>
        {
            grant.issuer().clone()
        }
        _ => signer.clone(),
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

    /// `issuer` grants `audience` over `subject`.
    async fn hop(
        issuer: &Ed25519Signer,
        audience: &Did,
        subject: UcanSubject,
    ) -> dialog_ucan_core::Delegation<dialog_varsig::AnySignature> {
        DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer.clone()))
            .audience(audience)
            .subject(subject)
            .command(vec![])
            .try_build()
            .await
            .unwrap()
    }

    /// `space -> root -> device (any subject) -> ephemeral`: what an
    /// account's device mints.
    async fn device_minted(
        space: &Ed25519Signer,
        root: &Ed25519Signer,
        device: &Ed25519Signer,
        ephemeral: &Did,
    ) -> DelegationChain {
        let specific = UcanSubject::Specific(space.did());
        DelegationChain::new(hop(space, &root.did(), specific.clone()).await)
            .push(hop(root, &device.did(), UcanSubject::Any).await)
            .unwrap()
            .push(hop(device, ephemeral, specific).await)
            .unwrap()
    }

    #[dialog_common::test]
    async fn it_attributes_a_device_minted_invite_to_the_device_account() {
        let space = signer(&SUBJECT_SEED).await;
        let root = signer(&INVITER_SEED).await;
        let device = signer(&CLAIMER_SEED).await;
        let ephemeral = signer(&EPHEMERAL_SEED).await.did();
        let chain = device_minted(&space, &root, &device, &ephemeral).await;

        assert_eq!(inviting_account(&chain), root.did());
    }

    #[dialog_common::test]
    async fn it_attributes_a_reshared_invite_to_the_member_who_reshared_it() {
        // The founder's link, claimed by a member, who then mints their
        // own: the chain runs through the founder and on to the member.
        let space = signer(&SUBJECT_SEED).await;
        let founder = signer(&INVITER_SEED).await;
        let founder_device = signer(&[5u8; 32]).await;
        let first_link = signer(&EPHEMERAL_SEED).await;
        let member = signer(&[6u8; 32]).await;
        let member_device = signer(&[7u8; 32]).await;
        let second_link = signer(&[8u8; 32]).await.did();
        let specific = UcanSubject::Specific(space.did());
        let chain = device_minted(&space, &founder, &founder_device, &first_link.did())
            .await
            .push(hop(&first_link, &member.did(), specific.clone()).await)
            .unwrap()
            .push(hop(&member, &member_device.did(), UcanSubject::Any).await)
            .unwrap()
            .push(hop(&member_device, &second_link, specific).await)
            .unwrap();

        assert_eq!(inviting_account(&chain), member.did());
        assert_eq!(
            Invitation::from_chain(&chain)
                .unwrap()
                .inviter
                .0
                .to_string(),
            founder.did().as_str(),
            "the first-hop audience still names the founder"
        );
    }

    #[dialog_common::test]
    async fn it_attributes_an_invite_signed_by_an_account_to_that_account() {
        let space = signer(&SUBJECT_SEED).await;
        let root = signer(&INVITER_SEED).await;
        let ephemeral = signer(&EPHEMERAL_SEED).await.did();
        let specific = UcanSubject::Specific(space.did());
        let chain = DelegationChain::new(hop(&space, &root.did(), specific.clone()).await)
            .push(hop(&root, &ephemeral, specific).await)
            .unwrap();

        assert_eq!(inviting_account(&chain), root.did());
    }

    #[dialog_common::test]
    async fn it_attributes_a_one_hop_invite_to_its_signer() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let inviter = signer(&INVITER_SEED).await.did();

        assert_eq!(inviting_account(&minted_chain(&subject).await), inviter);
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

    #[dialog_common::test]
    async fn it_projects_the_bound_resharer_and_rejects_owner_impersonation() {
        let alice = signer(&INVITER_SEED).await;
        let bob = signer(&CLAIMER_SEED).await;
        let device = signer(&[5; 32]).await;
        let carol = signer(&EPHEMERAL_SEED).await;
        let space = signer(&SUBJECT_SEED).await;
        let first = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space.clone()))
            .audience(&alice.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let share = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(alice.clone()))
            .audience(&bob.did())
            .subject(UcanSubject::Specific(space.did()))
            .command(vec![])
            .meta(tonk_invite::inviter_meta(&alice.did()))
            .try_build()
            .await
            .unwrap();
        let identity = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(bob.clone()))
            .audience(&device.did())
            .subject(UcanSubject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let prefix = DelegationChain::new(first)
            .push(share)
            .unwrap()
            .push(identity)
            .unwrap();
        for (declared, expected) in [
            (Some(bob.did()), Some(bob.did())),
            (Some(alice.did()), None),
            (None, Some(alice.did())),
        ] {
            let leaf = DelegationBuilder::new()
                .issuer(dialog_credentials::Signer::from(device.clone()))
                .audience(&carol.did())
                .subject(UcanSubject::Specific(space.did()))
                .command(vec![])
                .meta(
                    declared
                        .as_ref()
                        .map(tonk_invite::inviter_meta)
                        .unwrap_or_default(),
                )
                .try_build()
                .await
                .unwrap();
            let chain = prefix.clone().push(leaf).unwrap();
            let record = Invitation::from_chain(&chain);
            assert_eq!(
                record.as_ref().map(|record| record.inviter.0.to_string()),
                expected.map(|did| did.to_string())
            );
            if let Some(record) = record {
                assert_eq!(record.subject.0.to_string(), space.did().to_string());
                assert_eq!(record.audience.0.to_string(), carol.did().to_string());
            }
        }
    }
}
