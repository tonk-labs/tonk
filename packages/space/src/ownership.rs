use crate::delegation::Delegation;
use dialog_query::claim::{Attribute, Claim, Relation, Transaction};
use dialog_query::{Entity, Value};
use std::str::FromStr;

/// Represents space ownership - links a space DID to a delegation.
/// Implements `Claim` to be asserted via `Session::edit` API.
///
/// When asserted, this will:
/// 1. Assert the underlying delegation facts
/// 2. Assert the space/owner relation linking the space to the delegation
pub struct Ownership(pub Delegation);

impl From<Delegation> for Ownership {
    fn from(delegation: Delegation) -> Self {
        Ownership(delegation)
    }
}

impl Ownership {
    /// Returns the space DID (the subject of the underlying delegation)
    pub fn space_did(&self) -> &str {
        &self.0.subject
    }

    /// Returns a reference to the underlying delegation
    pub fn delegation(&self) -> &Delegation {
        &self.0
    }
}

impl Claim for Ownership {
    fn assert(self, transaction: &mut Transaction) {
        let delegation_cid = self.0.cid.clone();
        let space_did = self.0.subject.clone();

        // First, assert all the delegation facts
        self.0.assert(transaction);

        // Then, assert the ownership relation
        let entity = Entity::from_str(&space_did).expect("Space DID is not a valid entity");

        if let Ok(attr) = Attribute::from_str("space/owner") {
            Relation::new(attr, entity, Value::String(delegation_cid)).assert(transaction);
        }
    }

    fn retract(self, transaction: &mut Transaction) {
        let delegation_cid = self.0.cid.clone();
        let space_did = self.0.subject.clone();

        // First, retract all the delegation facts
        self.0.retract(transaction);

        // Then, retract the ownership relation
        let entity = match Entity::from_str(&space_did) {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Ok(attr) = Attribute::from_str("space/owner") {
            Relation::new(attr, entity, Value::String(delegation_cid)).retract(transaction);
        }
    }
}
