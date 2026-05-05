//! On-the-wire shape for query results — a serializable
//! projection of [`ConceptConclusion`] that workers emit and
//! browser clients deserialize.

use std::collections::BTreeMap;

use dialog_query::{Any, ConceptConclusion, Parameters, Term};
use serde::{Deserialize, Serialize};

/// Serializable projection of a [`ConceptConclusion`] — the
/// concept's entity plus the projected field values for every
/// term named by the originating query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conclusion {
    /// Entity URI of the matched concept (`did:key:…` etc.).
    pub this: String,
    /// Field values keyed by term name from the query. Each value
    /// is the raw `dialog_artifacts::Value` serialized via its
    /// `serde::Serialize` impl (untagged enum — strings, numbers,
    /// bools, entity URIs, byte buffers).
    pub fields: BTreeMap<String, serde_json::Value>,
}

impl Conclusion {
    /// Project a [`ConceptConclusion`] using the query's `terms`
    /// to discover which field names were bound.
    pub fn project(conclusion: &ConceptConclusion, terms: &Parameters) -> Self {
        let mut fields = BTreeMap::new();
        for (name, _) in terms.iter() {
            let lookup = conclusion
                .source()
                .lookup(&Term::<Any>::var(name.clone()))
                .ok()
                .and_then(|v| serde_json::to_value(&v).ok());
            if let Some(value) = lookup {
                fields.insert(name.clone(), value);
            }
        }
        Self {
            this: conclusion.entity().to_string(),
            fields,
        }
    }
}
