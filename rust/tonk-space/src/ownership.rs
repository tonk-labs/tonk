//! Space ownership claims.
//!
//! This module provides the `Ownership` type which wraps a `Delegation` and
//! adds an additional `space/owner` relation when asserted. This links a
//! space DID to the delegation that grants ownership over it.

use crate::delegation::Delegation;
use crate::relation::the;
use dialog_query::claim::{Claim, Relation, Transaction};
use dialog_query::{Entity, Value};
use std::str::FromStr;
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::Ed25519Did;

/// Represents space ownership - links a space DID to a delegation.
///
/// When asserted as a `Claim`, this will:
/// 1. Assert all the underlying delegation facts (issuer, audience, subject, etc.)
/// 2. Assert a `space/owner` relation linking the space DID to the delegation CID
///
/// This allows querying "who owns this space?" by looking up the `space/owner`
/// relation on the space's DID entity.
pub struct Ownership(pub Delegation);

impl From<Delegation> for Ownership {
    fn from(delegation: Delegation) -> Self {
        Ownership(delegation)
    }
}

impl Ownership {
    /// Returns the subject of the underlying delegation.
    ///
    /// The subject specifies what the delegation applies to:
    /// - `DelegatedSubject::Specific(did)` - applies to a specific DID (the space)
    /// - `DelegatedSubject::Any` - a "powerline" delegation that applies to any subject
    pub fn subject(&self) -> &DelegatedSubject<Ed25519Did> {
        self.0.subject()
    }

    /// Returns a reference to the underlying delegation.
    pub fn delegation(&self) -> &Delegation {
        &self.0
    }
}

impl Claim for Ownership {
    /// Asserts the ownership claim as facts in the transaction.
    ///
    /// This first asserts all delegation facts, then adds the `space/owner`
    /// relation linking the space DID to the delegation CID.
    fn assert(self, transaction: &mut Transaction) {
        // Get entity and subject before consuming the delegation
        let delegation_entity = self.0.this();
        let subject = self.subject().clone();

        // Assert all the delegation facts first
        self.0.assert(transaction);

        // Then assert the ownership relation: subject -> space/owner -> delegation_entity
        let subject_str = match &subject {
            DelegatedSubject::Specific(did) => did.to_string(),
            DelegatedSubject::Any => "*".to_string(),
        };

        // subject_entity is the space DID as an Entity
        let subject_entity =
            Entity::from_str(&subject_str).expect("DID strings are valid Entity URIs");

        Relation::new(
            the!("space/owner"),
            subject_entity,
            Value::String(delegation_entity.to_string()),
        )
        .assert(transaction);
    }

    /// Retracts the ownership claim from the transaction.
    fn retract(self, transaction: &mut Transaction) {
        // Get entity and subject before consuming the delegation
        let delegation_entity = self.0.this();
        let subject = self.subject().clone();

        // Retract all the delegation facts first
        self.0.retract(transaction);

        // Then retract the ownership relation
        let subject_str = match &subject {
            DelegatedSubject::Specific(did) => did.to_string(),
            DelegatedSubject::Any => "*".to_string(),
        };

        // subject_entity is the space DID as an Entity
        let subject_entity =
            Entity::from_str(&subject_str).expect("DID strings are valid Entity URIs");

        Relation::new(
            the!("space/owner"),
            subject_entity,
            Value::String(delegation_entity.to_string()),
        )
        .retract(transaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Operator;
    use ucan::Delegation as UcanDelegation;
    use ucan::did::Ed25519Signer;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// Create a test delegation with random operators.
    fn make_test_delegation() -> Delegation {
        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();

        let ucan_delegation = UcanDelegation::builder()
            .issuer(Ed25519Signer::from(&issuer))
            .audience(audience.did().clone())
            .subject(DelegatedSubject::Specific(subject.did().clone()))
            .command(vec!["read".to_string(), "write".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        Delegation::from(ucan_delegation)
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_creates_ownership_from_delegation() {
        let delegation = make_test_delegation();
        let expected_subject = delegation.subject().clone();
        let ownership = Ownership::from(delegation);

        assert_eq!(ownership.subject(), &expected_subject);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_provides_access_to_underlying_delegation() {
        let delegation = make_test_delegation();
        let expected_issuer = delegation.issuer().to_string();
        let ownership = Ownership::from(delegation);

        let cid_str = format!("ucan:{}", ownership.delegation().cid());
        assert!(cid_str.starts_with("ucan:"));
        assert_eq!(ownership.delegation().issuer().to_string(), expected_issuer);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_matches_subject_with_delegation() {
        let delegation = make_test_delegation();
        let ownership = Ownership::from(delegation);

        assert_eq!(ownership.subject(), ownership.delegation().subject());
    }
}
