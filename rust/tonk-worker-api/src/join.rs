//! Join-route wire DTOs.

use serde::{Deserialize, Serialize};

use crate::RepositoryInfo;

/// Body of `POST /api/profile/join`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JoinRequest {
    /// Full invite URL including any `#fragment`.
    ///
    /// Audience-open invites carry the ephemeral seed in the URL
    /// fragment; browsers never send fragments with `fetch`, so the
    /// caller must read `window.location.href` client-side and
    /// forward the complete string.
    pub url: String,
}

/// Body of a successful `POST /api/profile/join` response.
///
/// The `outcome` discriminator splits "we created a new local
/// replica for you" from "you already had one; we just refreshed
/// your access." UIs can navigate to `repository.name` either
/// way — only the toast / banner copy differs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum JoinResponse {
    /// A new local replica was created for the invited subject
    /// under the requested name. Status 201.
    Joined {
        /// Repository info for the freshly created replica.
        repository: RepositoryInfo,
    },
    /// The recipient already had a replica for this subject. The
    /// invite's delegation chain was saved (renewing access if
    /// the invite carried fresh delegations) but no new replica
    /// was created and the requested name is ignored. Status 200.
    Renewed {
        /// Repository info for the existing replica the recipient
        /// will land in.
        repository: RepositoryInfo,
    },
}

/// Guest visit uses the same secret-bearing request shape as durable join.
pub type VisitRequest = JoinRequest;

/// Guest visit returns the same mounted-replica outcome shape as durable join.
pub type VisitResponse = JoinResponse;

/// Active local membership mode for a mounted repository.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MembershipResponse {
    /// `guest` or `durable`.
    pub status: String,
}
