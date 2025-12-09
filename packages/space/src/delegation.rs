use dialog_query::claim::{Attribute, Claim, Relation, Transaction};
use dialog_query::{Entity, Value};
use std::str::FromStr;

/// Represents a delegation to be stored as facts in the space.
/// Implements `Claim` to be asserted via `Session::edit` API.
pub struct Delegation {
    /// The CID of the delegation (used as entity)
    pub cid: String,
    /// The raw bytes of the delegation
    pub bytes: Vec<u8>,
    /// The issuer DID
    pub issuer: String,
    /// The audience DID
    pub audience: String,
    /// The subject DID (the space DID)
    pub subject: String,
    /// The command being delegated
    pub command: String,
}

impl Claim for Delegation {
    fn assert(self, transaction: &mut Transaction) {
        // Parse entity from CID - if this fails, we skip (can't assert without valid entity)
        let entity = match Entity::from_str(&self.cid) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Helper to create relations - skip if attribute parsing fails
        let mut add_relation = |attr_str: &str, value: Value| {
            if let Ok(attr) = Attribute::from_str(attr_str) {
                Relation::new(attr, entity.clone(), value).assert(transaction);
            }
        };

        // Store the raw delegation bytes
        add_relation("db/blob", Value::Bytes(self.bytes));

        // Store the issuer DID
        add_relation("ucan/issuer", Value::String(self.issuer));

        // Store the audience DID
        add_relation("ucan/audience", Value::String(self.audience));

        // Store the subject DID
        add_relation("ucan/subject", Value::String(self.subject));

        // Store the command
        add_relation("ucan/cmd", Value::String(self.command));
    }

    fn retract(self, transaction: &mut Transaction) {
        // Parse entity from CID
        let entity = match Entity::from_str(&self.cid) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Helper to create relations for retraction
        let mut remove_relation = |attr_str: &str, value: Value| {
            if let Ok(attr) = Attribute::from_str(attr_str) {
                Relation::new(attr, entity.clone(), value).retract(transaction);
            }
        };

        remove_relation("db/blob", Value::Bytes(self.bytes));
        remove_relation("ucan/issuer", Value::String(self.issuer));
        remove_relation("ucan/audience", Value::String(self.audience));
        remove_relation("ucan/subject", Value::String(self.subject));
        remove_relation("ucan/cmd", Value::String(self.command));
    }
}
