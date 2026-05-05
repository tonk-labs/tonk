//! Wire shape for [`dialog_query::ConceptConclusion`] — the
//! dialog type doesn't derive serde directly, so the route and
//! broadcast layers project into [`Conclusion`] before
//! serializing.

use std::collections::BTreeMap;

use dialog_query::ConceptConclusion;
use serde::Serialize;

/// Serializable projection of a [`ConceptConclusion`] — the
/// concept's entity plus a placeholder for field values. The
/// route layer can do a richer projection if needed; this is
/// the minimum viable wire shape.
#[derive(Debug, Clone, Serialize)]
pub struct Conclusion {
    /// Entity URI of the matched concept (`did:key:…` etc.).
    pub this: String,
    /// Per-field values projected from the conclusion. Empty for
    /// the v1 wire format — extend when consumers need it.
    pub fields: BTreeMap<String, serde_json::Value>,
}

impl From<&ConceptConclusion> for Conclusion {
    fn from(c: &ConceptConclusion) -> Self {
        Self {
            this: c.entity().to_string(),
            fields: BTreeMap::new(),
        }
    }
}
