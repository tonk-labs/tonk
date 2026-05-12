//! Test helpers for `tonk-worker` consumers.
//!
//! Auto-enabled in this crate's test builds via `cfg(test)`; opt-in
//! for downstream crates that want the same fixtures via the
//! `helpers` cargo feature.

use dialog_query::{ConceptQuery, Query as ConceptPattern};
use tonk_schema::meta::Name;
use tonk_schema::query::Query as WireQuery;

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
