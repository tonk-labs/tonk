//! [`Plan`]: the query a branch's stack evaluates for a subscription.

use std::sync::Arc;

use dialog_corpus::Entity;
use dialog_query::concept::descriptor::ConceptConclusion;
use dialog_query::{
    Application, ConceptDescriptor, ConceptQuery, EvaluationError, Match, Restriction, Scope,
    Selection,
};
use serde::{Serialize, Serializer};
use tonk_schema::concept::QueryPlan;

use crate::Query;

/// A subscribed query: the [`QueryPlan`] the stack evaluates, with the
/// wire [`Query`] it was planned from, which is what the stack hashes to
/// tell one query from another ([`QueryHash`](dialog_repository::QueryHash)).
///
/// The plan, not the raw [`ConceptQuery`]: `QueryPlan::from` dispatches a
/// concept-of-concept, command or rule metadata query to the anonymous
/// enumeration that surfaces those built-in rows, where the raw query
/// would scan for `dialog.meta/*` facts that are not stored and find
/// nothing.
///
/// The identity is the wire projection a page sends, so two requests for
/// the same query share one subscription however their maps happened to
/// be laid out in memory: the stack hashes it through IPLD, whose maps
/// are sorted.
#[derive(Clone, Debug)]
pub struct Plan {
    plan: QueryPlan,
    identity: Arc<Query>,
}

impl Plan {
    /// The plan of `query`.
    pub fn new(query: ConceptQuery) -> Self {
        let identity = Arc::new(Query::from(&query));
        Self {
            plan: QueryPlan::from(query),
            identity,
        }
    }

    /// The same identity over another plan: a restriction of this one.
    fn with(&self, plan: QueryPlan) -> Self {
        Self {
            plan,
            identity: self.identity.clone(),
        }
    }
}

impl Serialize for Plan {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.identity.serialize(serializer)
    }
}

impl Application for Plan {
    type Conclusion = ConceptConclusion;

    fn evaluate<'a, Env, M: Selection + 'a>(self, selection: M, env: &'a Env) -> impl Selection + 'a
    where
        Env: Scope<'a>,
    {
        self.plan.evaluate(selection, env)
    }

    fn realize(&self, input: Match) -> Result<Self::Conclusion, EvaluationError> {
        self.plan.realize(input)
    }

    fn restrict(&self, entity: &Entity) -> Restriction<Self> {
        match self.plan.restrict(entity) {
            Restriction::Scoped(plan) => Restriction::Scoped(self.with(plan)),
            Restriction::Unaffected => Restriction::Unaffected,
            Restriction::Unsupported => Restriction::Unsupported,
        }
    }

    fn adopt(&self, conclusion: ConceptConclusion) -> Result<ConceptConclusion, EvaluationError> {
        self.plan.adopt(conclusion)
    }

    fn concept(&self) -> Option<&ConceptDescriptor> {
        self.plan.concept()
    }
}

#[cfg(test)]
mod tests {
    #![allow(missing_docs)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use dialog_repository::QueryHash;

    use super::*;

    /// A query decoded from the wire, as the worker decodes a request.
    fn wire_query(json: serde_json::Value) -> ConceptQuery {
        serde_json::from_value::<Query>(json)
            .expect("query decodes")
            .into_concept_query()
            .expect("concept query")
    }

    /// Two requests for the same query share one subscription, however
    /// their maps happened to be laid out in memory: the stack hashes
    /// them alike.
    #[dialog_common::test]
    fn it_hashes_equal_queries_equally() {
        let json = serde_json::json!({
            "predicate": { "with": {
                "name": { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" },
                "age": { "the": "xyz.tonk.person/age", "as": "UnsignedInteger", "cardinality": "one" },
                "email": { "the": "xyz.tonk.person/email", "as": "Text", "cardinality": "one" }
            } },
            "terms": {
                "this": { "?": { "name": "this" } },
                "name": { "?": { "name": "name" } },
                "email": { "?": { "name": "email" } },
                "age": 41
            }
        });
        let first = QueryHash::of(&Plan::new(wire_query(json.clone()))).expect("a hash");
        for _ in 0..32 {
            assert_eq!(
                QueryHash::of(&Plan::new(wire_query(json.clone()))),
                Some(first.clone())
            );
        }
    }

    /// Two different queries hash apart, so they do not share an
    /// evaluation.
    #[dialog_common::test]
    fn it_hashes_different_queries_apart() {
        let query = |term: &str| {
            wire_query(serde_json::json!({
                "predicate": { "with": {
                    term: { "the": "xyz.tonk.person/name", "as": "Text", "cardinality": "one" }
                } },
                "terms": { "this": { "?": { "name": "this" } }, term: { "?": { "name": term } } }
            }))
        };
        assert_ne!(
            QueryHash::of(&Plan::new(query("name"))),
            QueryHash::of(&Plan::new(query("label")))
        );
    }
}
