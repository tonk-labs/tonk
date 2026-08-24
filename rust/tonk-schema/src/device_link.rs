//! [`DeviceLink`] — an `account -> profile` powerline, as a device list
//! presents it.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::device::{CreatedAt, Reason, Title};

/// The reason recorded on a link minted for a device.
pub const DEVICE_LINK: &str = "device-link";

/// A device authorization: the label and creation time of an
/// `account -> profile` delegation.
///
/// # Why this has no identifying fields
///
/// Every other concept derives `this` from the data that identifies it.
/// This one takes the entity as given, because the identity already
/// exists: dialog stores a retained delegation under
/// `Entity::from_blob(hash)` and decomposes issuer, audience, subject,
/// command, and expiration onto it. This concept adds the fields dialog
/// does not carry, onto the entity dialog already made.
///
/// That is deliberate. The delegation IS the authorization — it is what
/// confers the authority and it is signed — so a separate record keyed
/// by device DID would be a second source of truth that could disagree
/// with the proof. It also means revoking the delegation takes this row
/// with it: a device cannot linger in a list after losing its authority.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceLink {
    /// The delegation's entity — its blob hash, as dialog keyed it.
    pub this: Entity,
    /// When the link was minted, unix seconds.
    pub created_at: CreatedAt,
    /// Human label for the device.
    pub title: Title,
    /// Why the delegation exists — [`DEVICE_LINK`] for a device.
    pub reason: Reason,
}

impl DeviceLink {
    /// Describe the delegation stored at `entity` as a device link.
    ///
    /// `entity` comes from retaining the chain — dialog returns the
    /// entities it wrote — so this never derives a hash of its own and
    /// cannot describe a delegation that was never stored.
    pub fn new(entity: Entity, title: impl Into<String>, created_at: u64) -> Self {
        Self {
            this: entity,
            created_at: CreatedAt(created_at),
            title: Title(title.into()),
            reason: Reason(DEVICE_LINK.to_string()),
        }
    }
}
