//! [`Replica`] — this device's view of a repository.

// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types and associated functions without doc comments. Suppress
// the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_repository::SiteAddress;
use dialog_varsig::Did;
use serde::Serialize;

use crate::Branch;
use crate::Remote;
use crate::domain::remote::Address;
use crate::domain::replica::{Kind, Name, Profile, Status, Subject};
use crate::prelude::*;

/// A replica — this device's view of a specific repository.
///
/// The `this` entity is content-derived from the `(profile, subject)`
/// pair (see [`Origin`]), so:
///
/// - two devices holding the same profile converge on the same
///   replica entity for a given repository, and
/// - different profiles produce different replica entities even when
///   pointing at the same repository.
///
/// The concept lives on the repository's meta branch. It is
/// typically the first thing asserted when a repository is opened
/// locally: writing the replica record announces "this profile has
/// a local view of this repository" and anchors subsequent
/// per-replica facts (branches, upstream configuration, etc.).
///
/// # Redundant by design
///
/// The [`Subject`] and [`Profile`] attributes carry the same two DIDs
/// that went into hashing the entity. The redundancy is intentional:
/// the hash is a one-way function, so without these attributes it
/// would be impossible to answer queries like "find the replica this
/// profile has for subject X" — you would need to know both inputs
/// upfront and re-hash to locate the entity. The stored attributes
/// make the relationships discoverable through normal queries.
///
/// # Constructing
///
/// [`Replica::new`] takes the profile and subject DIDs plus a name
/// and derives every field consistently:
///
/// ```no_run
/// use dialog_varsig::Did;
/// use tonk_schema::Replica;
/// # fn example(profile: Did, subject: Did) -> Replica {
/// Replica::new(profile, subject, "home")
/// # }
/// ```
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Replica {
    /// The replica's entity. Derived from `(profile, subject)` via
    /// [`Origin::this`].
    pub this: Entity,
    /// Human-readable name for the repository on this replica.
    pub name: Name,
    /// Reference to the repository this replica is a view of.
    pub subject: Subject,
    /// Reference to the profile that owns this replica.
    pub profile: Profile,
    /// What this replica points at: [`Self::PROFILE`] for the
    /// profile's own self-replica (`subject == profile`),
    /// [`Self::REPOSITORY`] for a space the profile has joined or
    /// created. Lets a query select only real spaces (e.g. the
    /// Hub picker) without re-deriving the self-replica from the
    /// profile entity.
    pub kind: Kind,
}

/// A [`Replica`] as it was written before the [`Kind`] field
/// existed: `name` + `subject` + `profile`, no `kind`.
///
/// A `Replica` query requires every field, so it won't match a
/// record that predates `kind`. `LegacyReplica` drops `kind` and
/// therefore matches *every* replica — kinded or not. Used only by
/// the `repo-vs-profile` migration to enumerate records that still
/// need stamping; new code should query [`Replica`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LegacyReplica {
    /// The replica's entity.
    pub this: Entity,
    /// Human-readable name for the repository on this replica.
    pub name: Name,
    /// Reference to the repository this replica is a view of.
    pub subject: Subject,
    /// Reference to the profile that owns this replica.
    pub profile: Profile,
}

/// The [`Kind`] of a replica as a standalone fact: just `this` and
/// `kind`.
///
/// Asserting one stamps a `kind` onto an existing replica entity
/// without re-asserting the whole [`Replica`] (which would require
/// re-deriving every other field). The migration uses it to
/// backfill `kind` on legacy records.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceKind {
    /// The replica entity being stamped.
    pub this: Entity,
    /// What the replica points at — see [`Replica::kind`].
    pub kind: Kind,
}

impl SpaceKind {
    /// A `kind` stamp for the given replica entity.
    pub fn new(this: Entity, kind: Kind) -> Self {
        Self { this, kind }
    }
}

/// The seeding [`Status`] of a replica as a standalone fact: just
/// `this` and `status`.
///
/// A repository's content branch is seeded asynchronously after the
/// replica is recorded, so the replica is first stamped
/// [`Replica::BLANK`] and flipped to [`Replica::INITIALIZED`] once the
/// seed completes. Asserting one stamps the status onto an existing
/// replica entity (cardinality one, so a later assert supersedes)
/// without re-asserting the whole [`Replica`]. The Hub reads it to
/// reflect the install state on each card.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceStatus {
    /// The replica entity being stamped.
    pub this: Entity,
    /// The replica's seeding status — see [`Replica::BLANK`] /
    /// [`Replica::INITIALIZED`].
    pub status: Status,
}

impl SpaceStatus {
    /// A `status` stamp for the given replica entity.
    pub fn new(this: Entity, status: Status) -> Self {
        Self { this, status }
    }
}

/// Hash input for [`Replica::this`].
///
/// The single-variant enum shape tags the CBOR encoding with the
/// concept name: two inputs with the same data but different
/// concepts (e.g. a replica and a branch that happened to share
/// field shapes) produce distinct hashes.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    Replica { subject: &'a Did, profile: &'a Did },
}

impl Replica {
    /// Build a replica concept from a profile DID, a subject DID,
    /// and a name.
    ///
    /// Derives `this` from `(profile, subject)` and fills in the
    /// `subject` and `profile` attributes from the same DIDs so
    /// every field is consistent with the entity hash. `name`
    /// takes anything convertible into [`Name`] — e.g. a `&str`
    /// — so callers don't have to wrap string literals.
    pub fn new(profile: Did, subject: Did, name: impl Into<Name>) -> Self {
        let kind = if subject == profile {
            Self::profile_kind()
        } else {
            Self::repository_kind()
        };
        Self {
            this: Entity::of(&This::Replica {
                subject: &subject,
                profile: &profile,
            }),
            subject: Subject(subject.this()),
            profile: Profile(profile.this()),
            name: name.into(),
            kind,
        }
    }

    /// `kind` URI for the profile's own self-replica.
    pub const PROFILE: &'static str = "tonk:profile";

    /// `kind` URI for a space (a repository the profile joined or
    /// created).
    pub const REPOSITORY: &'static str = "tonk:repository";

    /// The [`Kind`] for the profile's own self-replica.
    pub fn profile_kind() -> Kind {
        Kind(Self::PROFILE.parse().expect("tonk:profile parses"))
    }

    /// The [`Kind`] for a space.
    pub fn repository_kind() -> Kind {
        Kind(Self::REPOSITORY.parse().expect("tonk:repository parses"))
    }

    /// `status` URI for a freshly created replica whose content
    /// branch has not been seeded yet.
    pub const BLANK: &'static str = "tonk:blank";

    /// `status` URI for a replica whose content branch has been
    /// seeded.
    pub const INITIALIZED: &'static str = "tonk:initialized";

    /// The [`Status`] for a not-yet-seeded replica.
    pub fn blank_status() -> Status {
        Status(Self::BLANK.parse().expect("tonk:blank parses"))
    }

    /// The [`Status`] for a seeded replica.
    pub fn initialized_status() -> Status {
        Status(Self::INITIALIZED.parse().expect("tonk:initialized parses"))
    }

    /// The replica's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }

    /// Create a [`Branch`] concept on this replica.
    ///
    /// `name` is anything convertible into a [`branch::Name`],
    /// matching the [`Branch::new`] signature.
    ///
    /// [`branch::Name`]: crate::domain::branch::Name
    pub fn branch(&self, name: impl Into<crate::domain::branch::Name>) -> Branch {
        Branch::new(self, name)
    }

    /// Create a [`Remote`] concept on this replica.
    ///
    /// `name` accepts anything convertible into [`Name`]; the
    /// [`SiteAddress`] is encoded into an [`Address`] internally
    /// (we can't surface that as a `From` impl without clashing
    /// with the blanket one the `Attribute` derive emits — see
    /// [`Address::encode`]).
    pub fn remote(
        &self,
        name: impl Into<crate::domain::remote::Name>,
        subject: Did,
        address: &SiteAddress,
    ) -> Remote {
        Remote::new(self, subject, Address::encode(address), name)
    }
}

impl AsRef<Entity> for Replica {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::did;

    fn named(tag: &str) -> Name {
        Name(tag.into())
    }

    #[test]
    fn same_origin_same_entity() {
        let a = Replica::new(did!("test:p"), did!("test:r"), named("home"));
        let b = Replica::new(did!("test:p"), did!("test:r"), named("home"));
        assert_eq!(a.this.to_string(), b.this.to_string());
    }

    #[test]
    fn different_profile_different_entity() {
        let a = Replica::new(did!("test:p1"), did!("test:r"), named("home"));
        let b = Replica::new(did!("test:p2"), did!("test:r"), named("home"));
        assert_ne!(a.this.to_string(), b.this.to_string());
    }

    #[test]
    fn name_does_not_affect_entity() {
        // The entity is derived from (profile, subject) alone, so
        // renaming a replica does not produce a new entity — it
        // produces a new name attribute on the existing one.
        let a = Replica::new(did!("test:p"), did!("test:r"), named("home"));
        let b = Replica::new(did!("test:p"), did!("test:r"), named("pictures"));
        assert_eq!(a.this.to_string(), b.this.to_string());
    }

    #[test]
    fn subject_and_profile_reflect_inputs() {
        let profile = did!("test:profile-x");
        let subject = did!("test:repo-y");
        let replica = Replica::new(profile.clone(), subject.clone(), named("n"));
        assert_eq!(replica.profile.0.to_string(), profile.as_str());
        assert_eq!(replica.subject.0.to_string(), subject.as_str());
    }
}
