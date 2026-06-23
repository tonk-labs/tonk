//! [`Membership`] — a profile's membership of a repository.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::membership::{Invitation, Member, Name, Role, Subject};
use crate::prelude::*;

/// A membership — a profile is a member of a repository.
///
/// The `this` entity is content-derived from the `(subject, member)`
/// pair, so re-asserting a membership (multi-device claims, repeated
/// joins) converges on the same entity. The repository's creator
/// asserts a bare membership at create time; invite claimers assert
/// one alongside an [`InvitedVia`] stamp. Lives on the repository's
/// meta branch.
///
/// `subject` and `member` repeat the hash inputs as queryable
/// attributes — same redundant-by-design rationale as
/// [`Replica`](crate::Replica).
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Membership {
    /// The membership's entity. Derived from `(subject, member)`.
    pub this: Entity,
    /// Reference to the repository.
    pub subject: Subject,
    /// Reference to the member profile.
    pub member: Member,
}

/// Hash input for [`Membership::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name so equal field data under a
/// different concept hashes differently.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    Membership { subject: &'a Did, member: &'a Did },
}

impl Membership {
    /// Build a membership from the member profile DID and the
    /// repository subject DID.
    pub fn new(member: Did, subject: Did) -> Self {
        Self {
            this: Entity::of(&This::Membership {
                subject: &subject,
                member: &member,
            }),
            subject: Subject(subject.this()),
            member: Member(member.this()),
        }
    }

    /// The membership's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for Membership {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

/// Provenance stamp: how a membership came to be — the invitation it
/// was claimed through. Follows the [`SpaceKind`] / [`SpaceStatus`]
/// stamp pattern: a standalone fact on an existing entity, so the
/// base [`Membership`] query stays uniform across founders (no stamp)
/// and claimers (stamped).
///
/// [`SpaceKind`]: crate::SpaceKind
/// [`SpaceStatus`]: crate::SpaceStatus
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvitedVia {
    /// The membership entity being stamped.
    pub this: Entity,
    /// The invitation the membership was claimed through.
    pub invitation: Invitation,
}

impl InvitedVia {
    /// Stamp `membership` as claimed through `invitation`.
    pub fn new(membership: Entity, invitation: Entity) -> Self {
        Self {
            this: membership,
            invitation: Invitation(invitation),
        }
    }
}

/// The role of a member in a space, stamped onto a [`Membership`]
/// entity. Follows the [`InvitedVia`] / [`SpaceStatus`] stamp pattern:
/// a standalone fact on an existing entity, so the base [`Membership`]
/// query stays uniform across all members.
///
/// The creator is stamped [`MemberRole::FOUNDER`] at space creation;
/// invite claimers are stamped [`MemberRole::MEMBER`] on join. Lives on
/// the repository's content branch alongside [`Membership`], so the
/// roster replicates to every member.
///
/// [`SpaceStatus`]: crate::SpaceStatus
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemberRole {
    /// The membership entity being stamped.
    pub this: Entity,
    /// The member's role — `tonk:founder` or `tonk:member`.
    pub role: Role,
}

impl MemberRole {
    /// The space creator's role URI.
    pub const FOUNDER: &'static str = "tonk:founder";
    /// An invite claimer's role URI.
    pub const MEMBER: &'static str = "tonk:member";

    /// Stamp `membership` with the founder role.
    pub fn founder(membership: Entity) -> Self {
        Self::stamp(membership, Self::FOUNDER)
    }

    /// Stamp `membership` with the member role.
    pub fn member(membership: Entity) -> Self {
        Self::stamp(membership, Self::MEMBER)
    }

    /// Stamp `membership` with the role at `role_uri`. The URIs
    /// ([`FOUNDER`](Self::FOUNDER) / [`MEMBER`](Self::MEMBER)) are
    /// constants, so the parse never fails in practice.
    fn stamp(membership: Entity, role_uri: &str) -> Self {
        let role = role_uri.parse().unwrap_or_else(|_| membership.clone());
        Self {
            this: membership,
            role: Role(role),
        }
    }
}

/// A member's self-asserted display name for this repository. A
/// standalone fact on the membership entity, mirroring [`InvitedVia`]
/// — the base [`Membership`] query stays uniform whether or not a
/// name was written. Last-wins (cardinality-one): a member may rename,
/// and the current value displays. Written by the member's own worker
/// alongside the membership, so the space meta needs no cross-repo
/// profile lookup to render a roster.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemberName {
    /// The membership entity being named.
    pub this: Entity,
    /// The member's display name.
    pub name: Name,
}

impl MemberName {
    /// Name `membership` with `name`.
    pub fn new(membership: Entity, name: String) -> Self {
        Self {
            this: membership,
            name: Name(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_same_entity_for_the_same_member_and_subject() {
        let a = Membership::new(did!("test:m"), did!("test:r"));
        let b = Membership::new(did!("test:m"), did!("test:r"));
        assert_eq!(a.this.to_string(), b.this.to_string());
    }

    #[dialog_common::test]
    fn it_derives_different_entities_for_different_members() {
        let a = Membership::new(did!("test:m1"), did!("test:r"));
        let b = Membership::new(did!("test:m2"), did!("test:r"));
        assert_ne!(a.this.to_string(), b.this.to_string());
    }

    #[dialog_common::test]
    fn it_derives_different_entities_for_different_subjects() {
        let a = Membership::new(did!("test:m"), did!("test:r1"));
        let b = Membership::new(did!("test:m"), did!("test:r2"));
        assert_ne!(a.this.to_string(), b.this.to_string());
    }

    #[dialog_common::test]
    fn it_reflects_member_and_subject_as_attributes() {
        let member = did!("test:member-x");
        let subject = did!("test:repo-y");
        let membership = Membership::new(member.clone(), subject.clone());
        assert_eq!(membership.member.0.to_string(), member.as_str());
        assert_eq!(membership.subject.0.to_string(), subject.as_str());
    }

    #[dialog_common::test]
    fn it_stamps_the_membership_entity() {
        let membership = Membership::new(did!("test:m"), did!("test:r"));
        let invitation_entity = Entity::of(&"some-invitation");
        let stamp = InvitedVia::new(membership.this().clone(), invitation_entity.clone());
        assert_eq!(stamp.this, *membership.this());
        assert_eq!(stamp.invitation.0, invitation_entity);
    }

    #[dialog_common::test]
    fn it_stamps_founder_and_member_roles() {
        let membership = Membership::new(did!("test:m"), did!("test:r"));
        let founder = MemberRole::founder(membership.this().clone());
        let member = MemberRole::member(membership.this().clone());
        assert_eq!(founder.this, *membership.this());
        assert_eq!(member.this, *membership.this());
        assert_eq!(founder.role.0.to_string(), MemberRole::FOUNDER);
        assert_eq!(member.role.0.to_string(), MemberRole::MEMBER);
        assert_ne!(founder.role.0, member.role.0);
    }

    #[dialog_common::test]
    fn it_stamps_the_membership_with_a_name() {
        let membership = Membership::new(did!("test:m"), did!("test:r"));
        let stamp = MemberName::new(membership.this().clone(), "Alice".to_string());
        assert_eq!(stamp.this, *membership.this());
        assert_eq!(stamp.name.0, "Alice");
    }
}
