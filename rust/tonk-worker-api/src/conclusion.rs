//! Query-result conclusions and subscription frames — the on-the-wire
//! shape workers emit and browser clients deserialize.
//!
//! A [`Conclusion`] is one row of a result: an entity id plus a map of
//! projected field values. Its fields are engine-free (`String` +
//! `BTreeMap<String, Ipld>`); only the *projection* of a raw engine
//! `ConceptConclusion` into this shape needs the datalog engine, so
//! that constructor (`tonk_core::conclusion::project`) stays in
//! `tonk-core` while the type itself lives here. `tonk-core` re-exports
//! [`Conclusion`] and [`Frame`] at their historical paths.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use serde::{Deserialize, Serialize};

/// Serializable projection of a query match — the concept's entity
/// plus the projected field values for every term named by the query.
///
/// `PartialEq` by value (entity + fields) so a delta's `retracted`
/// rows can be matched against a retained snapshot to remove them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conclusion {
    /// Entity URI of the matched concept (`did:key:…` etc.).
    pub this: String,
    /// Field values keyed by term name from the query. Each value is
    /// the raw `dialog_artifacts::Value` serialized into the IPLD data
    /// model — strings, integers, floats, bools, byte buffers — so the
    /// wire encoding stays codec-agnostic (dag-json on the browser hop,
    /// dag-cbor for storage).
    pub fields: BTreeMap<String, Ipld>,
}

/// One subscription update on the wire.
///
/// A subscriber's first frame (and any frame after a reconnect) is a
/// [`Snapshot`](Frame::Snapshot) — the full current result set, so a
/// fresh consumer needs no prior state. Every subsequent frame is a
/// [`Delta`](Frame::Delta): the rows that entered
/// ([`asserted`](Frame::Delta::asserted)) and left
/// ([`retracted`](Frame::Delta::retracted)) the result since the
/// last frame. The consumer applies the delta to its retained set,
/// keyed by conclusion identity, and re-renders.
///
/// Serialized internally-tagged so the browser can match on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    /// The full current result set. First frame per subscriber and
    /// after a reconnect.
    Snapshot {
        /// Every row currently in the result.
        conclusions: Vec<Conclusion>,
    },
    /// The change since the previous frame.
    Delta {
        /// Rows that entered the result.
        asserted: Vec<Conclusion>,
        /// Rows that left the result.
        retracted: Vec<Conclusion>,
    },
}
