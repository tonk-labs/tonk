//! The two facts the search-tree inspector publishes for itself.
//!
//! These implement declarations in `tonk-core/assets/library/diagnose.yaml`,
//! which is the schema of record. Everything ELSE the inspector shows is
//! derived at query time from dialog's `tree/*` resolvers — node shape,
//! spans, entries, the components a key decodes to — so it needs nothing
//! stored and nothing published.
//!
//! What is left over is the two things a resolver cannot answer, and both
//! go into the branch's **session overlay**: folded into every read
//! (standing subscriptions included), never committed, never replicated,
//! gone when the worker is.
//!
//! - **Expansion** ([`DiagnoseOutline`]) is what this viewer chose to look
//!   at. Keeping it as a fact rather than as element state is what makes
//!   the lazy outline declarative: the children rule reads it as an
//!   ordinary premise, so revealing a subtree is a write and the view
//!   follows. Keeping it in the overlay rather than committed is what
//!   stops one person's browsing from replicating to everyone else.
//!
//! - **Locality** ([`DiagnoseStatus`]) is where the bytes happen to be.
//!   `dialog_artifacts::inspect` refuses to serve this through `Load` and
//!   is right to: a resolver's rows are a pure function of an immutable
//!   block, and "is this block here?" changes with no commit behind it.
//!   So it is published as a dated snapshot, the same shape the console
//!   publishes reactor state in — true when it was taken, and re-taken
//!   whenever the outline reveals more of the tree.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::diagnose::{Expanded, Local, Probed};

/// The inspector's expansion state: which nodes are showing their contents.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiagnoseOutline {
    /// The outline itself — the [`OUTLINE`](Self::OUTLINE) singleton.
    pub this: Entity,
    /// A node whose contents are shown. Cardinality-many, so expanding
    /// accumulates rather than replacing.
    pub expanded: Expanded,
}

impl DiagnoseOutline {
    /// The fixed entity the outline's expansion state is keyed on. A
    /// well-known singleton, like `state:here` for the sync chip: one
    /// branch is in scope per inspector page, so a singleton suffices and
    /// the view can subscribe without resolving anything first.
    pub const OUTLINE: &'static str = "diagnose:outline";

    /// Expansion state naming `node` as shown.
    pub fn expanding(node: Entity) -> Self {
        Self {
            this: Self::OUTLINE.parse().expect("diagnose:outline parses"),
            expanded: Expanded(node),
        }
    }
}

/// Whether a node's block is held on this device.
///
/// Keyed on the node's own entity (`tree:<base58>`, what the resolvers name
/// it), so the outline's row for a node joins its status with no lookup.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiagnoseStatus {
    /// The node this describes.
    pub this: Entity,
    /// True when the block is in this device's archive; false when reading
    /// it costs a round trip to the remote.
    pub local: Local,
    /// When the probe ran.
    pub probed: Probed,
}

impl DiagnoseStatus {
    /// A probe result for `node`, stamped at `probed`.
    pub fn new(node: Entity, local: bool, probed: String) -> Self {
        Self {
            this: node,
            local: Local(local),
            probed: Probed(probed),
        }
    }
}
