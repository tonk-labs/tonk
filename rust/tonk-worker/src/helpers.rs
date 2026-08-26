//! Test helpers for `tonk-worker` consumers.
//!
//! Auto-enabled in this crate's test builds via `cfg(test)`; opt-in
//! for downstream crates that want the same fixtures via the
//! `helpers` cargo feature.

use dialog_query::{ConceptQuery, Query as ConceptPattern};
use tonk_schema::meta::Name;
use tonk_schema::query::Query as WireQuery;
use tonk_schema::{Remote, Replica};

/// Wire-form `/query` body that selects every published [`Name`]
/// on a branch — `this` (the `id:<n>` name entity) plus `entity`
/// (the target it currently points at).
///
/// Derived from the [`Name`] concept's own
/// [`dialog_query::ConceptDescriptor`] rather than hand-rolled
/// JSON, so the wire shape stays in lockstep with the Rust type
/// definition. Tests that need a stable query with bound
/// variables — projection tests, subscription tests — use this so
/// they pick up Name-shape changes automatically.
pub fn named_concept_wire_query() -> serde_json::Value {
    let pattern = ConceptPattern::<Name>::default();
    let query = ConceptQuery::from(pattern);
    let wire = WireQuery::from(&query);
    serde_json::to_value(&wire).expect("Name query serializes")
}

/// Wire-form `/query` body selecting every [`Remote`] on a branch.
///
/// Derived from the concept the way [`named_concept_wire_query`] is, so
/// a shape change reaches subscribers without editing hand-rolled JSON.
/// Tests wait on this to learn that a remote ATTACHED — the handler
/// wires it after navigating, so the fact arriving is the signal, and
/// subscribing means waiting on that notification rather than asking
/// again on a timer.
pub fn remote_concept_wire_query() -> serde_json::Value {
    let pattern = ConceptPattern::<Remote>::default();
    let query = ConceptQuery::from(pattern);
    let wire = WireQuery::from(&query);
    serde_json::to_value(&wire).expect("Remote query serializes")
}

/// Wire-form `/query` body selecting every [`Replica`] on a branch.
///
/// A created space lands here as a replica row on profile main, so a
/// test that dispatched `space/create` waits on this rather than asking
/// the profile listing again on a timer.
pub fn replica_concept_wire_query() -> serde_json::Value {
    let pattern = ConceptPattern::<Replica>::default();
    let query = ConceptQuery::from(pattern);
    let wire = WireQuery::from(&query);
    serde_json::to_value(&wire).expect("Replica query serializes")
}

/// Wire-form `/query` body selecting the email-status overlay row.
///
/// Re-exported from [`crate::query`], where it lives so the account
/// panel can ask the same question a test does.
pub use crate::query::email_status_wire_query;
