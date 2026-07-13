//! Engine-free mirror of `tonk_schema::query::Query`.
//!
//! A query is the body of a `/query` request and the canonical input
//! to the subscription hash: term bindings plus a predicate (a concept
//! descriptor object, or a formula name string). The real type
//! (`tonk-schema`) types these fields with `dialog_query::{Parameters,
//! ConceptDescriptor}`, which pulls the datalog engine.
//!
//! Clients that only *build and forward* a query (construct it from
//! JSON, serialize it onward to the worker) never resolve the
//! predicate or run the planner — they need only the wire shape. This
//! mirror reproduces it with plain serde types:
//!
//! - `terms` mirrors `Parameters(HashMap<String, Term<Any>>)`, which
//!   serializes as a JSON object. `IndexMap<String, Value>` matches
//!   that shape and additionally preserves author insertion order.
//! - `predicate` mirrors the untagged `Predicate` enum (a descriptor
//!   object or a formula string) as an opaque `Value`, which holds
//!   either losslessly.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The body of a `/query` request. Mirrors `tonk_schema::query::Query`
/// for clients that build and forward queries without resolving them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// Term bindings for the query, keyed by variable name.
    pub terms: IndexMap<String, Value>,
    /// What the query selects: a concept descriptor object, or a
    /// formula name string.
    pub predicate: Value,
}
