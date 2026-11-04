//! The library's component catalog and what came of installing from it.
//!
//! Declared in core.yaml as `library/component` and `library/outcome`;
//! these structs follow those declarations.

// The `#[derive(Concept)]` macro generates helper types without doc comments.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::library::{component, outcome};

/// A component a space can install (`library/component`), keyed by its
/// entity, `tonk:library/<name>`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LibraryComponent {
    /// `tonk:library/<name>`.
    pub this: Entity,
    /// The component's library file without `.yaml`.
    pub name: component::Name,
    /// What a person calls it.
    pub label: component::Label,
    /// Its release.
    pub version: component::Version,
}

/// The AGENTS.md lines a component adds when installed. A projection of
/// `library/component`'s optional `agents` field: a component may add none.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LibraryComponentAgents {
    /// `tonk:library/<name>`.
    pub this: Entity,
    /// One line per line of AGENTS.md.
    pub agents: component::Agents,
}

/// What came of one request to install or uninstall a component
/// (`library/outcome`), keyed by the request's entity.
#[derive(Concept, Debug, Clone, PartialEq, PartialOrd)]
pub struct LibraryOutcome {
    /// The request's entity.
    pub this: Entity,
    /// The component asked for.
    pub component: outcome::Component,
    /// "install" or "uninstall".
    pub action: outcome::Action,
    /// "installed", "current", "uninstalled" or "failed".
    pub status: outcome::Status,
    /// The `time` the request was asked with.
    pub time: outcome::Time,
    /// The component's version the request installed or removed.
    pub version: Option<outcome::Version>,
    /// The AGENTS.md lines it added or removed.
    pub lines: Option<outcome::Lines>,
    /// Why it failed.
    pub reason: Option<outcome::Reason>,
}
