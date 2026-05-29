//! On-the-wire shape for query results — a serializable
//! projection of [`ConceptConclusion`] that workers emit and
//! browser clients deserialize.

use std::collections::BTreeMap;

use dialog_query::{Any, ConceptConclusion, Parameters, Term};
use ipld_core::ipld::Ipld;
use ipld_core::serde::to_ipld;
use serde::{Deserialize, Serialize};

/// Serializable projection of a [`ConceptConclusion`] — the
/// concept's entity plus the projected field values for every
/// term named by the originating query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conclusion {
    /// Entity URI of the matched concept (`did:key:…` etc.).
    pub this: String,
    /// Field values keyed by term name from the query. Each value
    /// is the raw `dialog_artifacts::Value` serialized into the
    /// IPLD data model — strings, integers, floats, bools, byte
    /// buffers — so the wire encoding stays codec-agnostic
    /// (dag-json on the browser hop, dag-cbor for storage).
    pub fields: BTreeMap<String, Ipld>,
}

impl Conclusion {
    /// Project a [`ConceptConclusion`] using the query's `terms`
    /// to discover which field names were bound.
    ///
    /// Variable terms read their value from the match's bindings;
    /// constant terms emit the constant directly. Constants
    /// matter for filter use-cases — `terms.name = "Alice"`
    /// means the caller already knows the value and the engine
    /// won't bind a variable for it; without surfacing the
    /// constant here, `fields["name"]` would be missing for every
    /// row even though every row's `name` is, by construction,
    /// `"Alice"`.
    pub fn project(conclusion: &ConceptConclusion, terms: &Parameters) -> Self {
        let mut fields = BTreeMap::new();
        for (name, term) in terms.iter() {
            let value = match term {
                Term::Constant(value) => to_ipld(value).ok(),
                Term::Variable { .. } => conclusion
                    .source()
                    .lookup(&Term::<Any>::var(name.clone()))
                    .ok()
                    .and_then(|v| to_ipld(&v).ok()),
            };
            if let Some(value) = value {
                fields.insert(name.clone(), value);
            }
        }
        Self {
            this: conclusion.entity().to_string(),
            fields,
        }
    }
}
