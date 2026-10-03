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

/// Invitation grant set acknowledged by a reported installation.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-connection")]
pub struct Grant(pub String);
/// Random local installation identifier. It is self-reported, not a device key.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-connection")]
pub struct Installation(pub String);
/// Bounded self-reported connection label, never an authority selector.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-connection")]
pub struct Name(pub String);
/// Completion status of an installation receipt, separate from legacy banners.
#[derive(Attribute, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[domain("xyz.tonk.agent-installation")]
pub struct InstallationStatus(pub String);
/// Additive receipt; old status-only confirmation queries remain valid.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentInstallationConfirmation {
    pub this: Entity,
    pub grant: Grant,
    pub installation: Installation,
    pub name: Name,
    pub status: InstallationStatus,
}

/// Labels allow Unicode and literal markup, but no controls or surrounding whitespace.
pub fn valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.trim() == name
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

/// Public random installation identifiers use 128 bits encoded as lowercase hex.
pub fn valid_installation_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
