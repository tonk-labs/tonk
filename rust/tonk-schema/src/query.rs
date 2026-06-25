//! On-the-wire shape for `/query` requests — the body workers
//! receive and browser clients send.
//!
//! A query names a [`Predicate`] plus its term bindings. The
//! predicate is either a concept descriptor (an object) or a named
//! formula (a string), invoked by name like dialog's own formulas
//! (`{ "assert": "math/sum", ... }`). Concept queries resolve
//! through dialog's planner as before; named formulas are
//! dispatched by the worker (the tree-introspection `tree/*` family
//! today, other formulas later).

use dialog_query::{ConceptDescriptor, ConceptQuery, Parameters};
use serde::{Deserialize, Serialize};

/// What a [`Query`] selects: a concept pattern or a named formula.
///
/// Discriminated by JSON type, mirroring how dialog's own
/// `Proposition` distinguishes a concept (object) from a formula
/// (string): a `ConceptDescriptor` always serializes as an object,
/// a formula name as a bare string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Predicate {
    /// A concept descriptor — resolved through dialog's planner.
    Concept(ConceptDescriptor),
    /// A named formula — dispatched by the worker by name (e.g.
    /// `"tree/node"`). Params travel in [`Query::terms`].
    Formula(String),
}

/// The body of a `/query` request and the canonical input to the
/// subscription hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// Term bindings for the query. Mirrors `ConceptQuery::terms`.
    pub terms: Parameters,
    /// What the query selects — a concept or a named procedure.
    pub predicate: Predicate,
}

impl From<&ConceptQuery> for Query {
    fn from(q: &ConceptQuery) -> Self {
        Self {
            terms: q.terms.clone(),
            predicate: Predicate::Concept(q.predicate.clone()),
        }
    }
}

impl Query {
    /// Borrow the formula name if this query selects one.
    pub fn formula(&self) -> Option<&str> {
        match &self.predicate {
            Predicate::Formula(name) => Some(name),
            Predicate::Concept(_) => None,
        }
    }

    /// Convert into a dialog [`ConceptQuery`], if this is a concept
    /// query. Returns the original [`Query`] back when it names a
    /// formula instead (which dialog's planner cannot resolve).
    pub fn into_concept_query(self) -> Result<ConceptQuery, Self> {
        match self.predicate {
            Predicate::Concept(predicate) => Ok(ConceptQuery {
                terms: self.terms,
                predicate,
            }),
            Predicate::Formula(_) => Err(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare-string `predicate` deserializes to a formula; an
    /// object `predicate` deserializes to a concept. This is the
    /// whole wire contract for the tagged union.
    #[test]
    fn it_discriminates_formula_from_concept_by_json_type() {
        let formula: Query =
            serde_json::from_value(serde_json::json!({ "predicate": "tree/node", "terms": {} }))
                .expect("string predicate parses");
        assert_eq!(formula.formula(), Some("tree/node"));
        assert!(
            formula.into_concept_query().is_err(),
            "a formula is not a concept query"
        );

        // A concept descriptor must declare at least one required
        // field (valid-by-construction), so the object carries one.
        let concept: Query = serde_json::from_value(serde_json::json!({
            "predicate": {
                "with": {
                    "name": { "the": "person/name", "as": "Text", "cardinality": "one" }
                }
            },
            "terms": {}
        }))
        .expect("object predicate parses");
        assert_eq!(concept.formula(), None);
        assert!(
            concept.into_concept_query().is_ok(),
            "an object predicate is a concept query"
        );
    }

    /// A formula `Query` round-trips through JSON keeping the bare
    /// string predicate (no enum tag leaks onto the wire).
    #[test]
    fn it_round_trips_a_formula_query() {
        let wire = serde_json::json!({ "predicate": "tree/child", "terms": {} });
        let query: Query = serde_json::from_value(wire.clone()).unwrap();
        let back = serde_json::to_value(&query).unwrap();
        assert_eq!(back["predicate"], "tree/child");
    }
}
