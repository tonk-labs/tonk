//! Repository description carried on the content branch.
//!
//! Unlike the meta-branch [`Replica`](crate::Replica) index (which only
//! records *that* a profile holds a repository), these concepts live on the
//! repository's *content* branch. Because they travel with the repository,
//! every device that syncs the content branch sees the same display name and
//! agent context.
//!
//! [`RepositoryName`] is pinned to the `tonk:repository` URI so a
//! `<tonk-display model=tonk:repository>` can resolve its view. Both concepts'
//! *instances* are keyed by the repository's subject DID: the stable entity the
//! name and agent context attach to.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` for this module so it compiles
// under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::repo::{Agents, Name};

/// A repository's own display name, stored on its content branch and
/// keyed by the subject DID.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepositoryName {
    /// The repository's subject DID — the entity the name attaches to.
    pub this: Entity,
    /// The repository's display name.
    pub name: Name,
}

/// A repository's synced agent context, keyed by the same subject DID as its
/// self-describing name.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RepositoryAgents {
    /// The repository's subject DID — the entity the context attaches to.
    pub this: Entity,
    /// Markdown projected as `AGENTS.md` by compatible agent launchers.
    pub agents: Agents,
}
