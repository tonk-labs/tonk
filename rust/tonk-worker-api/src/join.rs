//! Join-route wire DTOs.

use serde::{Deserialize, Serialize};

use crate::RepositoryInfo;

/// Stable terminal classification for a failed join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JoinFailureKind {
    /// The URL is not an invite this build can read.
    Malformed,
    /// The invite is addressed to a different identity.
    AudienceMismatch,
    /// The remote refused the invite's authority.
    Revoked,
    /// The remote could not be reached, or could not serve the space.
    Unavailable,
    /// The remote evaluated the invite's authority and declined it on
    /// policy grounds — the delegation is well-formed and unexpired, but
    /// its conditions do not hold for this space (an unprovisioned
    /// subject, a lapsed plan). Distinct from [`Self::ClaimFailed`],
    /// which means something on THIS device went wrong: retrying or
    /// re-inviting changes nothing here, the space's owner has to act.
    Refused,
    /// A local failure stopped the join.
    ClaimFailed,
}

impl JoinFailureKind {
    /// Stable value written to transient join state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::AudienceMismatch => "audience-mismatch",
            Self::Revoked => "revoked",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
            Self::ClaimFailed => "claim-failed",
        }
    }

    /// Fixed recipient-facing message.
    pub const fn message(self) -> &'static str {
        match self {
            Self::Malformed => "This share link is invalid.",
            Self::AudienceMismatch => "This invite was issued to a different identity.",
            Self::Revoked => "This invite has been revoked.",
            Self::Unavailable => "Tonk could not reach this space. Try again.",
            Self::Refused => {
                "This space's host declined the invite. Its owner needs to check the space's plan."
            }
            Self::ClaimFailed => "Tonk could not join this space.",
        }
    }

    /// Whether retrying the same in-memory URL is useful.
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Unavailable)
    }
}

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
