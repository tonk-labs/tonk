//! [`HostContext`] — a tab's per-request routing context.
//!
//! The service worker stamps this onto the per-tab **host-id entity** (the
//! `X-Tonk-Session` header value, parsed to an [`Entity`]) in the
//! Level-0-resolved branch's overlay, exactly the `state:here` pattern the sync
//! chip uses but keyed per tab instead of a singleton. Multiple tabs coexist as
//! distinct entities in the same overlay; a view scoped to a tab's host-id
//! entity reads only that tab's context.

// The `#[derive(Concept)]` macro generates helper types without doc comments.
// Suppress the crate-level `missing_docs` lint for this module so the macros
// compile under `-D warnings`.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::host::{Hash, Path};

/// A tab's request context: the location (`path`, `hash`) the host stamped,
/// keyed on the tab's host-id entity.
///
/// Both attributes are cardinality one, so re-stamping on navigation supersedes
/// the prior values rather than accumulating — the entity always reflects the
/// tab's latest location.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HostContext {
    /// The per-tab host-id entity (`host:<uuid>`), the `X-Tonk-Session` value.
    pub this: Entity,
    /// The document path the request came from.
    pub path: Path,
    /// The document fragment (may be empty when the location has none).
    pub hash: Hash,
}

impl HostContext {
    /// A context stamp for the given host-id entity.
    pub fn new(this: Entity, path: String, hash: String) -> Self {
        Self {
            this,
            path: Path(path),
            hash: Hash(hash),
        }
    }
}
