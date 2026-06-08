// The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate
// helper types without doc comments; suppress `missing_docs` here.
#![allow(missing_docs)]

//! Command concepts — transient effect triggers dispatched to
//! typed-Rust handlers in `tonk-worker` after a commit.
//!
//! A command is an ordinary [`Concept`] that is *asserted transiently*:
//! it triggers a handler and is swept from durable storage at the same
//! commit, so it fires exactly once and leaves no trace. The worker's
//! command registry matches a committed transient against these
//! concepts and runs the corresponding handler.
//!
//! These types only define the *shape* a command carries. The handler
//! that reacts to one lives in `tonk-worker`; the transient-ness is a
//! property of how the command is asserted (the
//! `dialog.concept/transient` marker), not of the type.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::command::Value as SpaceName;

/// Request to create a new space (repository) by local name.
///
/// Asserted transiently when the user submits the Add Space form (a
/// `<form onsubmit=space/create>` defined in `profile.yaml`). The
/// notation event layer reads `name` from the form's
/// `elements.name.value` and POSTs the transient claim; the handler
/// records the replica (`status: blank`) so the Hub shows it
/// installing, then creates the repository, seeds the standard library,
/// and flips the status to `initialized`.
///
/// `name`'s attribute is a `dom.event.*` read-path so the same concept
/// the form asserts is the one the worker handler decodes — see
/// [`crate::domain::command::Value`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateSpace {
    /// The command entity (a fresh id per invocation, derived by the
    /// worker from the predicate + payload).
    pub this: Entity,
    /// Local name for the new space, read from the form's `name` input.
    pub name: SpaceName,
}
