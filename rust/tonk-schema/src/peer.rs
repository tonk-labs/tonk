//! The local `tonk`, as this device currently sees it.
//!
//! Every other remote on the network page is a durable fact: a space's
//! configured upstream, true whether or not anything answers today. The
//! CLI is not. It is a process on this machine that may be running or
//! not, and "running" is an observation with a lifetime measured in the
//! length of a session.
//!
//! So it is stamped into the overlay rather than committed, keyed on the
//! [`Peer::STATE_CLI`] singleton exactly as the sync chip keys its live
//! status on `state:here`. Committing it would replicate "my laptop had
//! a CLI up" to every other device that syncs this profile, which is
//! false everywhere but here.
//!
//! The row is always rendered. A CLI that is not running is a status,
//! not an absence — a page that simply omitted it could not distinguish
//! "no CLI" from "not looked yet".

use crate::domain::peer::{Spaces, Status, Subject};
use dialog_artifacts::Entity;
use dialog_query::Concept;

/// This device's live view of the local `tonk` process.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Peer {
    /// The singleton entity this observation is keyed on.
    pub this: Entity,
    /// Whether the CLI answered.
    pub status: Status,
}

/// The same observation with what the CLI said about itself.
///
/// Separate from [`Peer`] because the identity fields only exist while
/// reachable: a concept whose required field is sometimes absent makes
/// the whole row unresolvable, which is the `sheet_missing_field_empty`
/// failure. Asserting this alongside [`Peer`] adds the detail without
/// making the status row depend on it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerIdentity {
    /// The singleton entity this observation is keyed on.
    pub this: Entity,
    /// The authenticated endpoint DID, not a claim of space authority.
    pub subject: Subject,
    /// How many spaces it is serving.
    pub spaces: Spaces,
}

/// User-facing detail for the latest connect attempt, including retry input.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerDetail {
    /// The live observation singleton.
    pub this: Entity,
    /// The exact URI entered by the user.
    pub address: crate::domain::peer::Address,
    /// A useful result or failure reason.
    pub detail: crate::domain::peer::Detail,
}

/// A space disclosed by the peer, not permission to read or join it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerOffer {
    /// An entity scoped to the current observation overlay.
    pub this: Entity,
    /// The offered space's DID.
    pub subject: crate::domain::peer_offer::Subject,
    /// The listener's local name for it.
    pub name: crate::domain::peer_offer::Name,
    /// The route observed along with this offer, not a grant of authority.
    pub address: crate::domain::peer_offer::Address,
}

/// Result of selecting an offer, scoped to its observation's lifetime.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerOfferDetail {
    /// The offer entity in the live inventory overlay.
    pub this: Entity,
    /// Success or refusal of the explicit upstream selection.
    pub detail: crate::domain::peer_offer::Detail,
}

impl Peer {
    /// The fixed entity the live CLI observation is keyed on.
    ///
    /// A well-known singleton, like `state:here` for sync: there is one
    /// local `tonk` per device, so the page subscribes without resolving
    /// anything first.
    pub const STATE_CLI: &'static str = "state:cli";

    /// `status` URI: the CLI answered.
    pub const REACHABLE: &'static str = "peer:reachable";

    /// `status` URI: nothing answered. The ordinary state, not a fault —
    /// a CLI is only running when someone ran it.
    pub const UNREACHABLE: &'static str = "peer:unreachable";

    /// The singleton entity, parsed.
    pub fn entity() -> Entity {
        Self::STATE_CLI.parse().expect("state:cli parses")
    }

    /// A stamp saying the CLI answered.
    pub fn reachable() -> Self {
        Self {
            this: Self::entity(),
            status: Status(Self::REACHABLE.parse().expect("peer:reachable parses")),
        }
    }

    /// A stamp saying nothing answered.
    pub fn unreachable() -> Self {
        Self {
            this: Self::entity(),
            status: Status(Self::UNREACHABLE.parse().expect("peer:unreachable parses")),
        }
    }
}

impl PeerIdentity {
    /// What the CLI said about itself.
    pub fn new(subject: impl Into<String>, spaces: u64) -> Self {
        Self {
            this: Peer::entity(),
            subject: Subject(subject.into()),
            spaces: Spaces(spaces),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both stamps key on the same singleton, so asserting one
    /// supersedes the other rather than leaving two rows.
    #[test]
    fn it_keys_every_observation_on_one_singleton() {
        assert_eq!(Peer::reachable().this, Peer::unreachable().this);
        assert_eq!(PeerIdentity::new("did:key:zAbc", 2).this, Peer::entity());
    }

    /// The two statuses must be distinguishable; a view selects on them.
    #[test]
    fn it_distinguishes_reachable_from_unreachable() {
        assert_ne!(Peer::reachable().status, Peer::unreachable().status);
    }
}
