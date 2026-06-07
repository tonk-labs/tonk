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

use dialog_artifacts::{Entity, Value};
use dialog_query::{Concept, Predicate};
use tonk_core::claim::{
    Claim, ConceptDescriptor as ClaimConceptDescriptor, PredicateApplication, TransactRequest,
    ValueMap,
};

use crate::domain::command::SpaceName;

/// Request to create a new space (repository) by local name.
///
/// Asserted transiently when the user creates a space: the handler
/// records the replica (`status: blank`) so the Hub shows it
/// installing, then creates the repository, seeds the standard library,
/// and flips the status to `initialized`. Replaces the direct
/// `PUT /api/repository/{name}` call — the button asserts this command
/// and the handler does the rest.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreateSpace {
    /// The command entity (a fresh id per invocation).
    pub this: Entity,
    /// Local name for the new space, used in its `/space/{name}` URL.
    pub name: SpaceName,
}

impl CreateSpace {
    /// A `CreateSpace` command for the given entity and name.
    pub fn new(this: Entity, name: impl Into<String>) -> Self {
        Self {
            this,
            name: SpaceName(name.into()),
        }
    }

    /// Build a [`TransactRequest`] that asserts this command
    /// *transiently* — the shape the UI POSTs to `/transact` to fire
    /// the command. `this` is omitted from the parameters so the worker
    /// derives a fresh command entity from the predicate + payload (one
    /// invocation per request).
    ///
    /// The descriptor is wrapped [`Transient`](ClaimConceptDescriptor::Transient)
    /// so the reactor buckets it as a command: it triggers the handler
    /// and is swept from durable storage at the same commit.
    pub fn into_request(name: impl Into<String>) -> TransactRequest {
        let descriptor =
            dialog_query::ConceptDescriptor::from(<Self as Predicate>::Application::default());
        let mut parameters = ValueMap::new();
        parameters.insert("name".into(), Value::String(name.into()));
        TransactRequest {
            claims: vec![Claim::Assert(PredicateApplication {
                predicate: ClaimConceptDescriptor::Transient(descriptor),
                parameters,
                name: None,
            })],
        }
    }
}
