//! Operational metadata for a durable invitation.

#![allow(missing_docs)]

use dialog_corpus::Entity;
use dialog_query::Concept;

use crate::Invitation;
use crate::domain::invitation_execution::{Kind, ListedAt, MintedAt};

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

/// When an [`Invitation`] was minted, beside it so older invitation
/// records stay readable: an invitation minted before this was recorded
/// has no row here.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvitationMinted {
    /// The associated invitation entity.
    pub this: Entity,
    /// Unix milliseconds at mint.
    pub minted_at: MintedAt,
}

impl InvitationMinted {
    /// Record when `invitation` was minted.
    pub fn new(invitation: &Invitation, minted_at: u64) -> Self {
        Self {
            this: invitation.this.clone(),
            minted_at: MintedAt(minted_at),
        }
    }
}

/// Where an [`Invitation`] that replaced another lists: at the place of the
/// one it replaced, so a reset default passlink stays the default.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InvitationListed {
    /// The associated invitation entity.
    pub this: Entity,
    /// The replaced link's place, unix milliseconds.
    pub listed_at: ListedAt,
}

impl InvitationListed {
    /// List `invitation` at `place`.
    pub fn new(invitation: &Invitation, place: u64) -> Self {
        Self {
            this: invitation.this.clone(),
            listed_at: ListedAt(place),
        }
    }
}
