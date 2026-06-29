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

/// A branch revision. Only the `tree` reference is rendered (a short badge).
///
/// The wire `tree` is a `TreeReference` — a Blake3 hash that serializes as a
/// **byte sequence**, not a string (the typed Rust value's `Display` is the
/// `#<base58>` form, which never reaches the wire). Captured as a raw
/// [`serde_json::Value`] and base58-encoded for the badge in `render`, so the
/// other `Revision` fields (subject/issuer/authority/cause/period/moment) are
/// simply ignored rather than mis-typed. `#[serde(default)]` tolerates a future
/// shape where `tree` is absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Revision {
    /// The revision's tree reference, as it arrives on the wire (a byte array).
    #[serde(default)]
    pub tree: serde_json::Value,
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
