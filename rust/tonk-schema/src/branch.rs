//! [`Branch`] — a branch within a replica of a repository.

// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types and associated functions without doc comments. Suppress
// the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use serde::Serialize;

use crate::prelude::*;
use crate::{Name, Origin};

/// Hash input for [`Branch::this`].
///
/// `Branch` identity is `(replica, name)`: the branch named `"main"`
/// on one replica is distinct from the branch named `"main"` on a
/// different replica (whether that's a different profile's view of
/// the same repository, or an entirely different repository).
///
/// The single-variant enum shape tags the CBOR encoding with the
/// concept name, so a branch and a remote with the same
/// `(origin, name)` pair hash to different entities.
///
/// Not stored — constructed transiently inside [`Branch::new`] so
/// the hash can be computed.
#[derive(Serialize)]
enum This<'a> {
    Branch { origin: &'a Entity, name: &'a str },
}

/// A branch within a replica.
///
/// The `this` entity is content-derived from the replica's entity
/// and the branch name, so:
///
/// - the same replica + the same branch name always yields the same
///   `Branch` entity (devices sharing a profile converge on the same
///   `Replica.this`, and therefore the same `Branch.this`), and
/// - different replicas — or different names within one replica —
///   yield different entities.
///
/// # Redundant by design
///
/// [`Subject`] and [`Origin`] duplicate information that went into
/// the hash. The hash is one-way, so without these attributes there
/// would be no way to answer "which branches are on this replica"
/// or "which branches belong to this repository" without knowing
/// the inputs upfront.
///
/// # Constructing
///
/// [`Branch::new`] takes a reference to the replica and a name,
/// reads the replica's `this` and `subject` attributes, and derives
/// every field consistently:
///
/// ```no_run
/// use dialog_varsig::did;
/// use tonk_schema::{Branch, Replica, Name};
/// let replica = Replica::new(
///     did!("test:profile"),
///     did!("test:repo"),
///     Name("home".into()),
/// );
/// let main = Branch::new(&replica, Name("main".into()));
/// ```
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(missing_docs)]
pub struct Branch {
    /// The branch's entity. Derived from `(replica, name)`.
    pub this: Entity,
    /// The branch's name on this replica.
    pub name: Name,
    /// The replica this branch lives on.
    pub origin: Origin,
}

impl AsRef<Entity> for Branch {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

impl Branch {
    /// Build a branch concept from an owning entity and a name.
    ///
    /// The `origin` argument can be anything that views as an
    /// [`Entity`] — a [`Replica`] (for a local branch) or a
    /// [`Remote`] (for a remote-side branch) both work via their
    /// `AsRef<Entity>` impls. Derives `this` from `(origin, name)`
    /// and stores `origin` as an attribute so every field is
    /// consistent with the entity hash.
    ///
    /// [`Remote`]: crate::Remote
    pub fn new(origin: impl AsRef<Entity>, name: Name) -> Self {
        let origin = origin.as_ref();
        Self {
            this: Entity::of(&This::Branch {
                origin,
                name: &name.0,
            }),
            origin: Origin::from(origin.clone()),
            name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Name, Replica};
    use dialog_varsig::did;

    fn branch_name(tag: &str) -> Name {
        Name(tag.into())
    }

    #[test]
    fn same_replica_same_name_same_entity() {
        let r = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let a = Branch::new(&r, branch_name("main"));
        let b = Branch::new(&r, branch_name("main"));
        assert_eq!(a.this, b.this);
    }

    #[test]
    fn different_name_different_entity() {
        let r = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let a = Branch::new(&r, branch_name("main"));
        let b = Branch::new(&r, branch_name("meta"));
        assert_ne!(a.this, b.this);
    }

    #[test]
    fn different_replica_different_entity() {
        let r1 = Replica::new(did!("test:p1"), did!("test:r"), Name("home".into()));
        let r2 = Replica::new(did!("test:p2"), did!("test:r"), Name("home".into()));
        let a = Branch::new(&r1, branch_name("main"));
        let b = Branch::new(&r2, branch_name("main"));
        assert_ne!(a.this, b.this);
    }

    #[test]
    fn different_repo_different_entity() {
        let r1 = Replica::new(did!("test:p"), did!("test:r1"), Name("home".into()));
        let r2 = Replica::new(did!("test:p"), did!("test:r2"), Name("home".into()));
        let a = Branch::new(&r1, branch_name("main"));
        let b = Branch::new(&r2, branch_name("main"));
        assert_ne!(a.this, b.this);
    }

    #[test]
    fn attributes_reflect_replica() {
        let r = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let b = Branch::new(&r, branch_name("main"));
        assert_eq!(b.origin.0, r.this);
    }

    #[test]
    fn replica_name_does_not_affect_branch_entity() {
        // The replica's display name is not part of Replica.this, so
        // renaming the replica doesn't change the branch entity.
        let home = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let pics = Replica::new(did!("test:p"), did!("test:r"), Name("pictures".into()));
        let a = Branch::new(&home, branch_name("main"));
        let b = Branch::new(&pics, branch_name("main"));
        assert_eq!(a.this, b.this);
    }

    #[test]
    fn branch_on_replica_and_remote_differ() {
        // A `Branch` is polymorphic over its origin — the same name
        // on a replica vs. on a remote still produces distinct
        // entities because the origin entities themselves differ.
        use crate::{Address, Remote};
        let replica = Replica::new(did!("test:p"), did!("test:r"), Name("home".into()));
        let remote = Remote::new(
            &replica,
            did!("test:repo"),
            Address(b"addr".to_vec()),
            Name("origin".into()),
        );
        let local = Branch::new(&replica, branch_name("main"));
        let tracking = Branch::new(&remote, branch_name("main"));
        assert_ne!(local.this, tracking.this);
        assert_eq!(local.origin.0, replica.this);
        assert_eq!(tracking.origin.0, remote.this);
    }
}
