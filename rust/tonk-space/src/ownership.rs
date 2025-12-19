//! Space ownership claims.
//!
//! This module provides the `Ownership` type which wraps a `Delegation` and
//! adds an additional `space_attr/owner` relation when asserted. This links a
//! space DID to the delegation that grants ownership over it.

use crate::delegation::Delegation;
use crate::schema::space;
use dialog_query::claim::{Claim, Transaction};
use dialog_query::{Entity, With};
use std::str::FromStr;
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::Ed25519Did;

/// Represents space ownership - links a space DID to a delegation.
///
/// When asserted as a `Claim`, this will:
/// 1. Assert all the underlying delegation facts (issuer, audience, subject, etc.)
/// 2. Assert a `space_attr/owner` relation linking the space DID to the delegation CID
///
/// This allows querying "who owns this space?" by looking up the `space_attr/owner`
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

    /// Returns the space DID this ownership applies to.
    ///
    /// - If the delegation has a specific subject, returns that subject DID
    /// - If the delegation is a powerline (`*`), falls back to the issuer DID
    pub fn space(&self) -> &Ed25519Did {
        match self.0.subject() {
            DelegatedSubject::Specific(did) => did,
            DelegatedSubject::Any => self.0.issuer(),
        }
    }

    /// Returns a reference to the underlying delegation.
    pub fn delegation(&self) -> &Delegation {
        &self.0
    }
}

impl Claim for Ownership {
    /// Asserts the ownership claim as facts in the transaction.
    ///
    /// This first asserts all delegation facts, then adds the `space_attr/owner`
    /// relation linking the space DID to the delegation CID.
    fn assert(self, transaction: &mut Transaction) {
        // Get entity and space DID before consuming the delegation
        let delegation = self.0.this();
        let space = self.space().to_string();

        // Assert all the delegation facts first
        self.0.assert(transaction);

        // Then assert the ownership relation
        transaction.assert(With {
            this: Entity::from_str(&space).expect("DID strings are valid Entity URIs"),
            has: space::Owner(delegation),
        });
    }

    /// Retracts the ownership claim from the transaction.
    fn retract(self, transaction: &mut Transaction) {
        // Get entity and space DID before consuming the delegation
        let delegation = self.0.this();
        let space = self.space().to_string();

        // Retract all the delegation facts first
        self.0.retract(transaction);

        // Then retract the ownership relation
        transaction.retract(With {
            this: Entity::from_str(&space).expect("DID strings are valid Entity URIs"),
            has: space::Owner(delegation),
        });
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
            .audience(*audience.did())
            .subject(DelegatedSubject::Specific(*subject.did()))
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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_returns_specific_subject_as_space() {
        let issuer = Operator::generate();
        let audience = Operator::generate();
        let subject = Operator::generate();
        let expected_space = *subject.did();

        let ucan_delegation = UcanDelegation::builder()
            .issuer(Ed25519Signer::from(&issuer))
            .audience(*audience.did())
            .subject(DelegatedSubject::Specific(*subject.did()))
            .command(vec!["read".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        let ownership = Ownership::from(Delegation::from(ucan_delegation));

        assert_eq!(ownership.space(), &expected_space);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_returns_issuer_as_space_for_powerline() {
        let issuer = Operator::generate();
        let audience = Operator::generate();
        let expected_space = *issuer.did();

        let ucan_delegation = UcanDelegation::builder()
            .issuer(Ed25519Signer::from(&issuer))
            .audience(*audience.did())
            .subject(DelegatedSubject::Any)
            .command(vec!["read".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        let ownership = Ownership::from(Delegation::from(ucan_delegation));

        assert_eq!(ownership.space(), &expected_space);
    }
}
