//! Serializable projections of `dialog_query::ConceptQuery` and
//! `dialog_query::ConceptConclusion`. The dialog types don't
//! derive serde directly — `WireQuery` and `WireConclusion`
//! cover both directions of the wire (request body in, broadcast
//! event out).

use std::collections::BTreeMap;

use dialog_query::{ConceptConclusion, ConceptDescriptor, ConceptQuery, Parameters};
use serde::{Deserialize, Serialize};

/// Serializable projection of a [`ConceptQuery`] for the
/// `/query` endpoint and the subscription hash input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireQuery {
    /// Term bindings for the query. Mirrors `ConceptQuery::terms`.
    pub terms: Parameters,
    /// Concept predicate. Mirrors `ConceptQuery::predicate`.
    pub predicate: ConceptDescriptor,
}

impl From<&ConceptQuery> for WireQuery {
    fn from(q: &ConceptQuery) -> Self {
        Self {
            terms: q.terms.clone(),
            predicate: q.predicate.clone(),
        }
    }
}

impl From<WireQuery> for ConceptQuery {
    fn from(w: WireQuery) -> Self {
        ConceptQuery {
            terms: w.terms,
            predicate: w.predicate,
        }
    }
}

/// Serializable projection of a [`ConceptConclusion`] — the
/// concept's entity plus a placeholder for field values. The
/// route layer can do a richer projection if needed; this is
/// the minimum viable wire shape.
#[derive(Debug, Clone, Serialize)]
pub struct WireConclusion {
    /// Entity URI of the matched concept (`did:key:…` etc.).
    pub this: String,
    /// Per-field values projected from the conclusion. Empty for
    /// the v1 wire format — extend when consumers need it.
    pub fields: BTreeMap<String, serde_json::Value>,
}

impl From<&ConceptConclusion> for WireConclusion {
    fn from(c: &ConceptConclusion) -> Self {
        Self {
            this: c.entity().to_string(),
            fields: BTreeMap::new(),
        }
    }
}
