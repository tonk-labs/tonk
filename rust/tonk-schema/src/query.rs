//! On-the-wire shape for `/query` requests — a serializable
//! projection of [`ConceptQuery`] that workers receive and
//! browser clients send.

use dialog_query::{ConceptDescriptor, ConceptQuery, Parameters};
use serde::{Deserialize, Serialize};

/// Serializable projection of a [`ConceptQuery`] — the body of a
/// `/query` request and the canonical input to the subscription
/// hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// Term bindings for the query. Mirrors `ConceptQuery::terms`.
    pub terms: Parameters,
    /// Concept predicate. Mirrors `ConceptQuery::predicate`.
    pub predicate: ConceptDescriptor,
}

impl From<&ConceptQuery> for Query {
    fn from(q: &ConceptQuery) -> Self {
        Self {
            terms: q.terms.clone(),
            predicate: q.predicate.clone(),
        }
    }
}

impl From<Query> for ConceptQuery {
    fn from(w: Query) -> Self {
        ConceptQuery {
            terms: w.terms,
            predicate: w.predicate,
        }
    }
}
