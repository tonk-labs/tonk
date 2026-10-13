//! What a branch runs.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::boot::{Pin, Src};

/// The release a branch pins, as the branch itself records it.
///
/// What a replica acts on is the statement on the branch's head, which is
/// signed and is read without the branch's tree. This is the same pin said
/// in the tree: a commit that moves it is what makes the head move, so the
/// statement travels on the next sync, and the branch's history shows what
/// it ran and when.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Release {
    /// The one entity a branch's release is recorded on.
    pub this: Entity,
    /// The line of releases the pin was taken from.
    pub src: Src,
    /// The release pinned.
    pub pin: Pin,
}

impl Release {
    /// The entity a branch's release is recorded on.
    pub const HERE: &'static str = "tonk:boot";

    /// The record of `pin`, taken from `src`.
    pub fn new(src: impl Into<String>, pin: impl Into<String>) -> Self {
        Self {
            this: Self::HERE.parse().expect("tonk:boot parses"),
            src: Src(src.into()),
            pin: Pin(pin.into()),
        }
    }
}
