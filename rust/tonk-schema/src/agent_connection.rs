//! Public invitation management facts on the issuing account's profile main.
#![allow(missing_docs)]
use dialog_artifacts::Entity;
use dialog_query::{Attribute, Concept};

#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-grant")]
pub struct Account(pub String);
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-grant")]
pub struct Subject(pub String);
/// Versioned public envelope with exact signed chains, label and original issue time.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-grant")]
pub struct PublicRecord(pub String);

#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentGrantGroup {
    pub this: Entity,
    pub account: Account,
    pub subject: Subject,
    pub public_record: PublicRecord,
}

#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-revocation")]
pub struct Target(pub String);
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-revocation")]
pub struct Receipt(pub String);
/// A service-acknowledged target; retained even when sibling revocations fail.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentGrantRevocation {
    pub this: Entity,
    pub target: Target,
    pub receipt: Receipt,
}

#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-connection")]
pub struct Status(pub String);
/// Untrusted setup acknowledgement in the shared space, not an authority grant.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentConnectionConfirmation {
    pub this: Entity,
    pub status: Status,
}

#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-revocation")]
pub struct RequestedAt(pub u64);
/// Durable withdrawal intent; survives even when no service call succeeds.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentGrantRevocationIntent {
    pub this: Entity,
    pub requested_at: RequestedAt,
}

/// Attributes of the rows the settings page lists invite groups from.
///
/// OVERLAY ONLY, on the profile's active branch. What a group shows —
/// whether its chains still validate, how many of its grants the service
/// has acknowledged as withdrawn, whether the space confirmed setup — is
/// worked out from the durable records above and the space's own facts,
/// so the worker republishes it here each time it is asked rather than
/// writing a second copy that could disagree with them.
pub mod listed {
    use dialog_query::Attribute;

    /// The group's id, what a withdrawal names.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Id(pub String);
    /// What the invite was issued for, as its issuer named it.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Label(pub String);
    /// The space the grants cover, as a DID.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Space(pub String);
    /// Who the grants were issued to, as a DID.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Recipient(pub String);
    /// When the grants expire, unix seconds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct ExpiresAt(pub u64);
    /// `active`, `expired`, `partial` or `revoked`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Status(pub String);
    /// Whether the space recorded that the terminal finished setting up.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Confirmed(pub bool);
    /// How many of the group's grants the service acknowledged withdrawn.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Removed(pub u64);
    /// How many grants the group holds.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Grants(pub u64);
    /// Whether the last withdrawal left a grant the service did not
    /// acknowledge.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Failed(pub bool);
    /// The terminal request the group answered, empty for an invite link.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connection-state")]
    #[cardinality(one)]
    pub struct Request(pub String);
    /// Where the last ask got to: `ready`, `unavailable` (invitations are
    /// not enabled in this build) or `failed`.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connections")]
    #[cardinality(one)]
    pub struct Outcome(pub String);
    /// When the ask this answers was made — the asking page's own stamp,
    /// so it can tell its answer from an earlier one.
    #[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
    #[domain("xyz.tonk.agent-connections")]
    #[cardinality(one)]
    pub struct AnsweredAt(pub u64);
}

/// One invite group, as the settings page lists it. Overlay-only; see
/// [`listed`].
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentConnectionState {
    pub this: Entity,
    pub id: listed::Id,
    pub label: listed::Label,
    pub space: listed::Space,
    pub recipient: listed::Recipient,
    pub expires_at: listed::ExpiresAt,
    pub status: listed::Status,
    pub confirmed: listed::Confirmed,
    pub removed: listed::Removed,
    pub grants: listed::Grants,
    pub failed: listed::Failed,
    pub request: listed::Request,
}

impl AgentConnectionState {
    /// The overlay entity a group's row lives on.
    pub fn entity(id: &str) -> String {
        format!("{}:{id}", Self::PREFIX)
    }

    /// What every group row's entity starts with.
    pub const PREFIX: &str = "state:agent-connection";
}

/// Where the settings page's last ask about invite groups got to: the
/// single `state:agent-connections` row, written after the group rows so a
/// page that sees its own `answered_at` sees the groups that answer it.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentConnectionsState {
    pub this: Entity,
    pub outcome: listed::Outcome,
    pub answered_at: listed::AnsweredAt,
}

impl AgentConnectionsState {
    /// The single entity the answer row lives on.
    pub const ENTITY: &str = "state:agent-connections";
}
