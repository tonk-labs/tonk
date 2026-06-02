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

use crate::meta::AnchorName;
use dialog_artifacts::Value;
use dialog_query::ConceptDescriptor as DialogConceptDescriptor;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Parameter bindings carried on the wire. Each entry is a
/// concrete [`Value`] (entity URI, scalar, ref) — the wire format
/// has no representation for logic variables or blanks. The
/// dialog-query [`Term`](dialog_query::Term)-flavoured `Parameters`
/// is used downstream after [`crate::claim`]-time lift; on the
/// wire we keep the surface narrow so the worker never has to
/// defend against terms that don't make sense for an assertion.
pub type ValueMap = IndexMap<String, Value>;

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
/// counterpart of `tonk_schema::query::Query`.
///
/// Each entry in `parameters` is a concrete [`Value`]; the wire
/// format intentionally does not support logic variables or
/// blanks (a `/transact` caller is writing facts, not querying).
/// The `"this"` slot, when present, names the subject entity;
/// when absent, the worker derives it from `(predicate, parameters)`
/// so callers never have to mint an arbitrary URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredicateApplication {
    /// The predicate, with its durability classification.
    pub predicate: ConceptDescriptor,
    /// Value bindings for this application. Omitting `"this"`
    /// asks the worker to derive the subject from the predicate
    /// and the remaining payload.
    #[serde(default)]
    pub parameters: ValueMap,
    /// Published name (`&anchor` in notation), if any. When
    /// present, applying the claim also asserts the desugared
    /// `dialog.name/referent` fact on `id:<name>` pointing at the
    /// `this` entity so the concept resolves by name. Mirrors
    /// the `name` slot on `tonk_schema::transact::Application`.
    ///
    /// An [`AnchorName`], so the `id:<name>` entity is validated when
    /// the claim is built or deserialized — a malformed name fails at
    /// that boundary rather than being silently dropped downstream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<AnchorName>,
}

impl PredicateApplication {
    /// `true` if the predicate names a transient concept.
    pub fn is_transient(&self) -> bool {
        self.predicate.is_transient()
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

impl TransactRequest {
    /// Reconstruct a request from its canonical DAG-JSON encoding.
    /// Used by the `claim!` macro, which serializes the lowered
    /// request at compile time and embeds the bytes; the generated
    /// code calls this at runtime. The bytes are always produced by
    /// `serde_ipld_dagjson` from this same type, so a decode
    /// failure is a build-time bug, not a user error.
    pub fn from_dagjson_bytes(bytes: &[u8]) -> Self {
        serde_ipld_dagjson::from_slice(bytes)
            .expect("claim!: compiled bootstrap is not valid DAG-JSON")
    }
}
