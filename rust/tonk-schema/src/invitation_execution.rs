//! Operational metadata for a durable invitation.

#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::Invitation;
use crate::domain::invitation_execution::Kind;

/// Audience metadata stored beside an [`Invitation`].
///
/// `this` is exactly the invitation entity, so old invitation records remain
/// readable and the companion can be joined without a second identifier.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvitationExecution {
    /// The associated invitation entity.
    pub this: Entity,
    /// Stable audience mode: `open` or `scoped`.
    pub kind: Kind,
}

impl InvitationExecution {
    /// Build execution metadata for an invitation.
    pub fn new(invitation: &Invitation, kind: impl Into<String>) -> Self {
        Self {
            this: invitation.this.clone(),
            kind: Kind(kind.into()),
        }
    }
}
