//! The outcome of undoing one change.
//!
//! An undo names the change by the revision that made it. Its outcome is
//! recorded on the content branch, on an entity derived from that revision
//! ([`Undo::entity`]), so whatever showed the change (a receipt on a card,
//! a view) can read whether it was undone, or why not, from the branch it
//! already follows.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::undo::{Reason, Revision, Status};

/// Whether one change was undone. Declared as `tonk:undo` in `core.yaml`.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Undo {
    /// The outcome's entity: [`Undo::entity`] of the revision.
    pub this: Entity,
    /// The revision whose change was asked to be undone.
    pub revision: Revision,
    /// [`Undo::UNDONE`] or [`Undo::REFUSED`].
    pub status: Status,
}

/// Why an undo was refused. Stamped beside a refused [`Undo`], and
/// retracted when a later attempt undoes the change.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UndoReason {
    /// The outcome's entity.
    pub this: Entity,
    /// The reason, in words a person can read.
    pub reason: Reason,
}

impl Undo {
    /// The change was undone.
    pub const UNDONE: &'static str = "undone";
    /// The change was not undone; an [`UndoReason`] says why.
    pub const REFUSED: &'static str = "refused";

    /// The entity an undo of `revision` records its outcome on.
    pub fn entity(revision: &str) -> Entity {
        format!("tonk:undo/{revision}")
            .parse()
            .expect("a base58 revision makes a valid entity URI")
    }

    /// An outcome `status` for `revision`.
    pub fn new(revision: &str, status: &str) -> Self {
        Self {
            this: Self::entity(revision),
            revision: Revision(revision.to_owned()),
            status: Status(status.to_owned()),
        }
    }
}

impl UndoReason {
    /// Why an undo of `revision` was refused.
    pub fn new(revision: &str, reason: impl Into<String>) -> Self {
        Self {
            this: Undo::entity(revision),
            reason: Reason(reason.into()),
        }
    }
}

/// A revision's version as an undo names it: base58 of its key bytes,
/// which round-trip through
/// [`Version::from_key_bytes`](dialog_artifacts::history::Version::from_key_bytes).
pub fn encode_revision(version: &dialog_artifacts::history::Version) -> String {
    use base58::ToBase58 as _;

    version.key_bytes().to_base58()
}

/// The inverse of [`encode_revision`], or `None` when `encoded` names no
/// version.
pub fn decode_revision(encoded: &str) -> Option<dialog_artifacts::history::Version> {
    use base58::FromBase58 as _;

    let bytes = encoded.from_base58().ok()?;
    dialog_artifacts::history::Version::from_key_bytes(&bytes).ok()
}
