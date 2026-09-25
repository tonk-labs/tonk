//! Profile wire DTOs.

use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

/// One space the profile owns, as the CLI's inventory lists it.
///
/// A repository's identity is its credential's `did:key` (`subject`);
/// the routing/storage key is the DID suffix (`key`). The membership
/// index carries no display name: the space's name lives in its own
/// `tonk/repository` concept on its content branch, so the UI resolves
/// the label from the space's own repo (per-space `<tonk-display
/// model=tonk:repository>`), not from this listing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpaceEntry {
    /// Routing/storage key — the `subject` DID suffix. The URL segment
    /// the UI links by.
    pub key: String,
    /// The space's identity DID.
    pub subject: Did,
}
