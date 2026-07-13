//! Profile-route wire DTOs.

use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

use crate::RepositoryInfo;

/// One space the profile owns, as listed by `GET /api/profile`.
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

/// Response body for `GET /api/profile`.
///
/// `profile` describes the profile "as a repository" (see
/// `bootstrap_profile`) so the UI can render it the same
/// way it renders any other space — populated by
/// `build_repository_info`, which reads the profile's meta
/// branch and surfaces its branches and remotes. `space` lists every
/// replica this profile owns — enough to populate the sidebar without
/// per-repo round-trips.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// [`RepositoryInfo`] for the profile itself — same shape as
    /// any other space, including the meta-branch entries for the
    /// profile's own branches and remotes.
    pub profile: RepositoryInfo,
    /// Every replica owned by this profile except the profile's
    /// own self-replica.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub space: Vec<SpaceEntry>,
    /// The member's effective display name (override, else petname).
    /// Lets the shell read identity without going through a space branch.
    pub display_name: String,
}
