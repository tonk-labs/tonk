//! On-the-wire shape for `/transact` requests — typed claims that
//! carry concept-level transient/durable classification through to
//! the reactor's transaction builder.
//!
//! See `plan/transact-endpoint.md` for the design. The short
//! version: every assertion or retraction names a predicate
//! ([`ConceptDescriptor`]) along with its parameter bindings
//! ([`PredicateApplication`]); the predicate wrapper carries
//! whether the concept is durable (carries forward across
//! commits) or transient (one-timestep lifetime, retracted
//! before durable write). The reactor reads this classification
//! to bucket transients without re-querying the schema.

use dialog_query::{ConceptDescriptor as DialogConceptDescriptor, ConceptQuery, Parameters};
use serde::{Deserialize, Serialize};

use crate::transact::ApplicationPlan;

/// A concept predicate plus its durability classification.
///
/// `Durable` is the default — facts of the concept carry forward
/// across commits until retracted (the implicit-persistence
/// rule). `Transient` means the facts exist only at the
/// timestep they're submitted in; the reactor's commit pipeline
/// asserts them so effects can read them, then retracts them
/// inside the same transaction so they never reach durable
/// storage.
///
/// The wrapper lives here, on the wire side, rather than as a
/// `transient: bool` field on [`DialogConceptDescriptor`] upstream.
/// Validating the end-to-end mechanism this way means we can
/// keep dialog's descriptor untouched until we're sure of the
/// design.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "concept", rename_all = "lowercase")]
pub enum ConceptDescriptor {
    /// Facts of this concept persist across commits until
    /// retracted.
    Durable(DialogConceptDescriptor),
    /// Facts of this concept exist only at the current
    /// timestep. The reactor strips them before the durable
    /// commit.
    Transient(DialogConceptDescriptor),
}

impl ConceptDescriptor {
    /// Borrow the inner [`DialogConceptDescriptor`], discarding the
    /// durability wrapper.
    pub fn concept(&self) -> &DialogConceptDescriptor {
        match self {
            Self::Durable(c) | Self::Transient(c) => c,
        }
    }

    /// `true` if this descriptor names a transient concept.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

/// A predicate applied to parameter bindings — the claim
/// counterpart of `tonk_schema::query::Query`. `parameters` mirrors
/// the dialog-query [`Parameters`] shape (`terms` in
/// [`dialog_query::ConceptQuery`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredicateApplication {
    /// The predicate, with its durability classification.
    pub predicate: ConceptDescriptor,
    /// Term bindings for this application. The `"this"` slot
    /// names the subject entity; other slots bind the
    /// predicate's attribute fields.
    pub parameters: Parameters,
}

impl PredicateApplication {
    /// `true` if the predicate names a transient concept.
    pub fn is_transient(&self) -> bool {
        self.predicate.is_transient()
    }

    /// Project into the [`ApplicationPlan`] the existing
    /// [`crate::transact`] emitter consumes — same EAV-emission
    /// machinery whether the claim ultimately lands in the
    /// durable or transient bucket.
    pub fn into_plan(self) -> ApplicationPlan {
        ApplicationPlan {
            statement: ConceptQuery {
                terms: self.parameters,
                predicate: match self.predicate {
                    ConceptDescriptor::Durable(c) | ConceptDescriptor::Transient(c) => c,
                },
            },
            name: None,
        }
    }
}

/// One assertion or retraction in a [`TransactRequest`] — the typed
/// write-unit shared by the structured-transaction path and the
/// notation path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "application", rename_all = "lowercase")]
pub enum Claim {
    /// Assert the facts produced by this predicate application.
    Assert(PredicateApplication),
    /// Retract the facts produced by this predicate
    /// application.
    Retract(PredicateApplication),
}

impl Claim {
    /// Borrow the inner [`PredicateApplication`], regardless of
    /// variant.
    pub fn application(&self) -> &PredicateApplication {
        match self {
            Self::Assert(a) | Self::Retract(a) => a,
        }
    }
}

/// Body of a `POST /api/repository/{repo}/branch/{branch}/transact`
/// (and profile counterpart) request — a list of [`Claim`]s applied
/// in order under one dialog commit.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransactRequest {
    /// In document order. Each claim contributes facts to the
    /// transaction; the reactor buckets transient applications
    /// separately so they can be retracted before the durable
    /// write.
    pub claims: Vec<Claim>,
}
