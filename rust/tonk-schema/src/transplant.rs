//! [`Transplant`] — the scar a re-rooted space carries.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::transplant::{Origin, Revision, Tree};
use crate::prelude::*;

/// A transplant — this space adopted its history from another subject.
///
/// Written by `tonk space transplant` in the first commit minted under
/// the fresh subject, so the record itself sits at the origin boundary:
/// everything before it in the log belongs to the origin's key,
/// everything after to the new one. Nothing is rewritten — the point of
/// a transplant is that the seam stays visible.
///
/// The `this` entity is content-derived from the origin subject, so a
/// space transplanted from two different origins keeps both scars while
/// re-running the same transplant converges on one record.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transplant {
    /// The transplant's entity. Derived from the origin subject.
    pub this: Entity,
    /// The subject the history was adopted from.
    pub origin: Origin,
    /// The origin's last published head record, byte-exact.
    pub revision: Revision,
    /// The tree root the transplant adopted.
    pub tree: Tree,
}

/// Hash input for [`Transplant::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name so equal field data under a
/// different concept hashes differently.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    Transplant { origin: &'a Did },
}

impl Transplant {
    /// Record the adoption of `origin`'s history, ending at the head
    /// whose encoded record is `revision` and whose tree root is `tree`.
    pub fn new(origin: &Did, revision: Vec<u8>, tree: impl Into<String>) -> Self {
        Self {
            this: Entity::of(&This::Transplant { origin }),
            origin: Origin(origin.this()),
            revision: Revision(revision),
            tree: Tree(tree.into()),
        }
    }

    /// The transplant's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for Transplant {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did(seed: &str) -> Did {
        format!("did:key:{seed}").parse().unwrap()
    }

    #[test]
    fn it_derives_the_entity_from_the_origin() {
        let origin = did("z6MkOriginA");
        let first = Transplant::new(&origin, vec![1], "tree-a");
        let again = Transplant::new(&origin, vec![2], "tree-b");
        assert_eq!(first.this(), again.this());

        let other = Transplant::new(&did("z6MkOriginB"), vec![1], "tree-a");
        assert_ne!(first.this(), other.this());
    }
}
