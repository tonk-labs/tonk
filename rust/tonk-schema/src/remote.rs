//! [`Remote`] — a named upstream for a replica.

// The `#[derive(Concept)]` macro generates helper types and
// associated functions without doc comments. Suppress the
// crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::prelude::*;
use crate::{Address, Branch, Name, Origin, Replica, Subject};

/// Hash input for [`Remote::this`].
///
/// `Remote` identity is `(replica, name)`: the remote named `"origin"`
/// on one replica is distinct from `"origin"` on a different replica.
/// The remote's target repository and address are stored as
/// attributes — they're queryable but not part of the hash, so
/// changing them reasserts attributes on the same entity rather than
/// minting a new one.
///
/// The single-variant enum shape tags the CBOR encoding with the
/// concept name, so a remote and a branch with the same
/// `(origin, name)` pair hash to different entities.
///
/// Not stored — constructed transiently inside [`Remote::new`] so
/// the hash can be computed.
#[derive(Serialize)]
enum This<'a> {
    Remote { origin: &'a Entity, name: &'a str },
}

/// A named upstream for a replica.
///
/// A `Remote` records "replica R calls server S 'origin', and the
/// repository there is subject X." The name is local to the replica,
/// the subject+address identify the target. Two replicas can each
/// have their own `"origin"` pointing at different targets.
///
/// # Redundant by design
///
/// [`Origin`], [`Subject`], and [`Address`] duplicate information
/// that either went into the hash (`origin`, via `name`) or describes
/// the target (`subject`, `address`). Keeping them as attributes
/// makes "which remotes live on replica X," "which remotes point at
/// repository Y," and "which remotes are at address Z" queryable
/// without re-hashing.
///
/// # Constructing
///
/// [`Remote::new`] takes the owning replica, the target repository
/// DID, the target address bytes, and the local name:
///
/// ```no_run
/// use dialog_varsig::did;
/// use tonk_schema::{Address, Name, Remote, Replica};
/// let replica = Replica::new(
///     did!("test:profile"),
///     did!("test:repo"),
///     Name("home".into()),
/// );
/// let remote = Remote::new(
///     &replica,
///     did!("test:repo"),
///     Address(vec![]),
///     Name("origin".into()),
/// );
/// ```
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct Remote {
    /// The remote's entity. Derived from `(replica, name)`.
    pub this: Entity,
    /// The remote's local name on this replica.
    pub name: Name,
    /// The replica that owns this remote.
    pub origin: Origin,
    /// The repository on the remote side.
    pub subject: Subject,
    /// The address of the remote site.
    pub address: Address,
}

impl Remote {
    /// Build a remote concept from a replica, target repository DID,
    /// address, and local name.
    ///
    /// Derives `this` from `(replica.this, name)` and fills in the
    /// `origin`, `subject`, and `address` attributes so every field
    /// is consistent with the entity hash. `name` takes anything
    /// convertible into [`Name`] — e.g. a `&str` — so callers
    /// don't have to wrap string literals.
    pub fn new(replica: &Replica, subject: Did, address: Address, name: impl Into<Name>) -> Self {
        let name = name.into();
        Self {
            this: Entity::of(&This::Remote {
                origin: replica.this(),
                name: &name.0,
            }),
            origin: Origin::from(replica.this().clone()),
            subject: Subject::from(subject.this()),
            address,
            name,
        }
    }

    /// Create a [`Branch`] concept on this remote.
    ///
    /// `name` accepts anything convertible into [`Name`],
    /// matching the [`Branch::new`] signature.
    pub fn branch(&self, name: impl Into<Name>) -> Branch {
        Branch::new(self, name)
    }
}

impl AsRef<Entity> for Remote {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::did;

    fn replica() -> Replica {
        Replica::new(did!("test:p"), did!("test:r"), Name("home".into()))
    }

    fn addr(bytes: &[u8]) -> Address {
        Address(bytes.to_vec())
    }

    #[test]
    fn same_replica_same_name_same_entity() {
        let r = replica();
        let a = Remote::new(&r, did!("test:repo"), addr(b"host1"), Name("origin".into()));
        let b = Remote::new(&r, did!("test:repo"), addr(b"host1"), Name("origin".into()));
        assert_eq!(a.this, b.this);
    }

    #[test]
    fn different_name_different_entity() {
        let r = replica();
        let a = Remote::new(&r, did!("test:repo"), addr(b"host1"), Name("origin".into()));
        let b = Remote::new(&r, did!("test:repo"), addr(b"host1"), Name("backup".into()));
        assert_ne!(a.this, b.this);
    }

    #[test]
    fn address_does_not_affect_entity() {
        // Address is an attribute, not part of the hash — pointing
        // "origin" at a different host rewrites the attribute on
        // the same entity.
        let r = replica();
        let a = Remote::new(&r, did!("test:repo"), addr(b"host1"), Name("origin".into()));
        let b = Remote::new(&r, did!("test:repo"), addr(b"host2"), Name("origin".into()));
        assert_eq!(a.this, b.this);
    }

    #[test]
    fn subject_does_not_affect_entity() {
        let r = replica();
        let a = Remote::new(&r, did!("test:repo-a"), addr(b"h"), Name("origin".into()));
        let b = Remote::new(&r, did!("test:repo-b"), addr(b"h"), Name("origin".into()));
        assert_eq!(a.this, b.this);
    }

    #[test]
    fn different_replica_different_entity() {
        let r1 = Replica::new(did!("test:p1"), did!("test:r"), Name("home".into()));
        let r2 = Replica::new(did!("test:p2"), did!("test:r"), Name("home".into()));
        let a = Remote::new(&r1, did!("test:repo"), addr(b"h"), Name("origin".into()));
        let b = Remote::new(&r2, did!("test:repo"), addr(b"h"), Name("origin".into()));
        assert_ne!(a.this, b.this);
    }

    #[test]
    fn branch_and_remote_with_same_inputs_differ() {
        // Branch and Remote both hash (replica, name), but the
        // concept tag in the enum variant makes their entities
        // distinct.
        use crate::Branch;
        let r = replica();
        let branch = Branch::new(&r, Name("shared".into()));
        let remote = Remote::new(&r, did!("test:repo"), addr(b"h"), Name("shared".into()));
        assert_ne!(
            branch.this, remote.this,
            "branch and remote collided on entity"
        );
    }

    #[test]
    fn remote_and_replica_branch_with_same_name_differ() {
        // Regression: a remote named "origin" on a replica and
        // a branch named "origin" on the same replica must
        // produce distinct entities. If they didn't, querying
        // `Branch` would also return the remote (every entity
        // has the same `origin` + `name` attribute shape).
        use crate::Branch;
        let r = replica();
        let remote_origin = Remote::new(&r, did!("test:repo"), addr(b"h"), Name("origin".into()));
        let branch_origin = Branch::new(&r, Name("origin".into()));
        assert_ne!(
            remote_origin.this, branch_origin.this,
            "a replica's remote 'origin' and branch 'origin' collided on entity"
        );
    }

    #[test]
    fn attributes_reflect_inputs() {
        let r = replica();
        let remote = Remote::new(
            &r,
            did!("test:repo-x"),
            addr(b"addr"),
            Name("origin".into()),
        );
        assert_eq!(remote.origin.0, r.this);
        assert_eq!(remote.subject.0.to_string(), "did:test:repo-x");
        assert_eq!(remote.address.0, b"addr");
        assert_eq!(remote.name.0, "origin");
    }
}
