//! Engine-free mirror of the worker's `/evaluate` JSON response.
//!
//! The inspector POSTs to the branch's `/evaluate` endpoint and renders the
//! result. It deserializes the response into these local serde types rather than
//! importing `tonk_worker::EvaluateResponse` / `tonk_evaluator::*`, which would
//! link the whole query engine — the very thing the sealed guest avoids. The
//! field shapes match the worker's wire contract
//! (`tonk-worker/src/router/evaluate.rs`, `tonk-evaluator/src/evaluate.rs`); a
//! drift there must be mirrored here.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The `/evaluate` response body.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EvaluateResponse {
    /// Branch revision before the commit, if any.
    #[serde(default)]
    pub revision_before: Option<Revision>,
    /// Branch revision after the commit (equals `revision_before` for a dry run).
    #[serde(default)]
    pub revision_after: Option<Revision>,
    /// Per-source-expression query matches before the commit.
    #[serde(default)]
    pub matches_before: Vec<QueryMatchBlock>,
    /// Per-source-expression query matches after the commit.
    #[serde(default)]
    pub matches_after: Vec<QueryMatchBlock>,
}

/// A branch revision — only the `tree` reference is rendered (as a short badge).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Revision {
    /// The revision's tree reference (`#<base58>`); shown truncated.
    pub tree: String,
}

/// Matches for one source-expression query.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QueryMatchBlock {
    /// Display label for the source expression (`person ?alice:` → `"person"`).
    pub label: String,
    /// One entry per matched entity.
    pub results: Vec<QueryResult>,
}

/// One match — an entity plus its bound field values.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QueryResult {
    /// Canonical entity URI for the match.
    pub this: String,
    /// Field name → bound value.
    pub fields: BTreeMap<String, serde_json::Value>,
}
