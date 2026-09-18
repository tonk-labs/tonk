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
