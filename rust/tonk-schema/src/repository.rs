//! [`RepositoryName`] — a repository's own self-describing name.
//!
//! Unlike the meta-branch [`Replica`](crate::Replica) index (which only
//! records *that* a profile holds a repository), this concept lives on
//! the repository's *content* branch and carries the repository's
//! display name. Because it travels with the repository, every device
//! that syncs the content branch sees the current name — there is no
//! per-profile cache to fall stale when another device renames it.
//!
//! The concept is pinned to the `tonk:repository` URI so a
//! `<tonk-display concept=tonk:repository>` can resolve its view, but its
//! *instances* are keyed by the repository's subject DID (the entity the
//! name attaches to). The standard-library `tonk/rename-repository` rule
//! writes it; the banner and the Hub card read it.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` for this module so it compiles
// under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::repo::Name;

/// A repository's own display name, stored on its content branch and
/// keyed by the subject DID.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepositoryName {
    /// The repository's subject DID — the entity the name attaches to.
    pub this: Entity,
    /// The repository's display name.
    pub name: Name,
}
