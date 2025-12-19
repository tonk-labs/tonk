//! Delegation wrapper for UCAN delegations.
//!
//! This module provides a newtype wrapper around `ucan::Delegation<Ed25519Did>`
//! that implements the `Claim` trait from dialog-query, allowing delegations
//! to be stored as facts in a dialog-db space.
//!
//! We use a wrapper because Rust's orphan rules prevent implementing external
//! traits (`Claim`) for external types (`ucan::Delegation`).

use dialog_query::claim::{Claim, Relation, Transaction};
use dialog_query::{Entity, Value};
pub use ipld_core::cid::Cid;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use ucan::Delegation as UcanDelegation;
use ucan::command::Command;
use ucan::delegation::builder::DelegationBuilder;
use ucan::delegation::subject::DelegatedSubject;
use ucan::did::{DidSigner, Ed25519Did};
use ucan::time::timestamp::Timestamp;
use ucan::unset::Unset;

use crate::relation::the;

/// Errors that can occur when validating delegations.
#[derive(Debug, Error)]
pub enum DelegationError {
    /// The delegation has expired (current time is past expiration).
    #[error("Delegation expired")]
    Expired,

    /// The delegation is not yet valid (current time is before notBefore).
    #[error("Delegation not yet valid (notBefore in future)")]
    NotYetValid,

    /// The delegation is invalid for some other reason.
    #[error("Invalid delegation: {0}")]
    InvalidDelegation(String),
}

/// A wrapper around `ucan::Delegation<Ed25519Did>` that implements `Claim`.
///
/// This wrapper exists because we need to implement the `Claim` trait from
/// dialog-query for UCAN delegations, but Rust's orphan rules prevent
/// implementing external traits for external types.
///
/// When a `Delegation` is asserted as a `Claim`, it stores the following
/// facts in the database:
/// - `db/blob`: The raw DAG-CBOR bytes of the delegation
/// - `ucan/issuer`: The DID of the entity granting the delegation
/// - `ucan/audience`: The DID of the entity receiving the delegation
/// - `ucan/subject`: The DID this delegation applies to (or "*" for powerline)
/// - `ucan/cmd`: The command path being delegated (e.g., "/read/write")
#[derive(Debug, Clone, Serialize, Deserialize)]
// Serialization is transparent - Delegation serializes/deserializes identically
// to the inner UcanDelegation<Ed25519Did>, producing the same bytes.
#[serde(transparent)]
pub struct Delegation(UcanDelegation<Ed25519Did>);

impl From<UcanDelegation<Ed25519Did>> for Delegation {
    fn from(delegation: UcanDelegation<Ed25519Did>) -> Self {
        Self(delegation)
    }
}

impl Delegation {
    /// Returns a builder for creating a new delegation.
    pub fn builder<S: DidSigner<Did = Ed25519Did>>()
    -> DelegationBuilder<S, Unset, Unset, Unset, Unset> {
        UcanDelegation::builder()
    }

    /// Returns a reference to the inner UCAN delegation.
    pub fn inner(&self) -> &UcanDelegation<Ed25519Did> {
        &self.0
    }

    /// Validates that the delegation is valid at the given time.
    ///
    /// A delegation is valid if:
    /// - It has not expired (expiration > now), or has no expiration
    /// - The notBefore time has passed (notBefore <= now), or has no notBefore
    ///
    /// # Arguments
    /// * `now` - The timestamp to validate against
    ///
    /// # Returns
    /// * `Ok(())` if the delegation is valid at the given time
    /// * `Err(DelegationError::Expired)` if the delegation has expired
    /// * `Err(DelegationError::NotYetValid)` if the notBefore time hasn't passed
    pub fn validate(&self, now: Timestamp) -> Result<(), DelegationError> {
        // Check expiration - delegation is invalid if now >= expiration
        if let Some(exp) = self.0.expiration() {
            if exp <= now {
                return Err(DelegationError::Expired);
            }
        }

        // Check notBefore - delegation is invalid if now < notBefore
        if let Some(nbf) = self.0.not_before() {
            if nbf > now {
                return Err(DelegationError::NotYetValid);
            }
        }

        Ok(())
    }

    /// Returns the audience DID (the entity receiving the delegation).
    pub fn audience(&self) -> &Ed25519Did {
        self.0.audience()
    }

    /// Returns the issuer DID (the entity granting the delegation).
    pub fn issuer(&self) -> &Ed25519Did {
        self.0.issuer()
    }

    /// Returns the subject of the delegation.
    ///
    /// The subject specifies what the delegation applies to:
    /// - `DelegatedSubject::Specific(did)` - applies to a specific DID (e.g., a space)
    /// - `DelegatedSubject::Any` - a "powerline" delegation that applies to any subject
    pub fn subject(&self) -> &DelegatedSubject<Ed25519Did> {
        self.0.subject()
    }

    /// Returns true if this is a "powerline" delegation (subject is `*`).
    ///
    /// Powerline delegations grant capabilities over any subject, not just
    /// a specific one. They're typically used for broad administrative access.
    pub fn is_powerline(&self) -> bool {
        matches!(self.0.subject(), DelegatedSubject::Any)
    }

    /// Returns the command being delegated.
    ///
    /// The command is a path-like structure (e.g., "/read/write") that
    /// specifies what operations are being delegated.
    pub fn command(&self) -> &Command {
        self.0.command()
    }

    /// Returns the content identifier (CID) for this delegation.
    ///
    /// The CID is a content-addressed hash of the delegation's DAG-CBOR
    /// representation. It uniquely identifies this delegation and is used
    /// as the entity URI when storing delegation facts.
    pub fn cid(&self) -> Cid {
        self.0.to_cid()
    }

    /// Returns this delegation as a dialog-db Entity.
    ///
    /// The entity URI is formatted as `ucan:{cid}` where `{cid}` is the
    /// content identifier of this delegation.
    pub fn this(&self) -> Entity {
        let cid_str = format!("ucan:{}", self.cid());
        Entity::from_str(&cid_str).expect("ucan:{cid} is a valid Entity URI")
    }

    /// Serializes this delegation to DAG-CBOR bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_ipld_dagcbor::to_vec(&self.0).expect("UcanDelegation serialization cannot fail")
    }

    /// Returns the expiration timestamp, if set.
    ///
    /// After this time, the delegation is no longer valid.
    pub fn expiration(&self) -> Option<Timestamp> {
        self.0.expiration()
    }

    /// Returns the notBefore timestamp, if set.
    ///
    /// Before this time, the delegation is not yet valid.
    pub fn not_before(&self) -> Option<Timestamp> {
        self.0.not_before()
    }
}

impl Claim for Delegation {
    /// Asserts this delegation as facts in the transaction.
    ///
    /// Creates relations for:
    /// - `db/blob`: Raw CBOR bytes
    /// - `ucan/issuer`: Issuer DID string
    /// - `ucan/audience`: Audience DID string
    /// - `ucan/subject`: Subject DID string (or "*")
    /// - `ucan/cmd`: Command string
    fn assert(self, transaction: &mut Transaction) {
        let this = self.this();
        let subject: Value = match self.subject() {
            DelegatedSubject::Specific(did) => did.to_string().into(),
            DelegatedSubject::Any => "*".to_string().into(),
        };

        Relation::new(the!("db/blob"), this.clone(), self.to_bytes().into()).assert(transaction);
        Relation::new(
            the!("ucan/issuer"),
            this.clone(),
            self.issuer().to_string().into(),
        )
        .assert(transaction);
        Relation::new(
            the!("ucan/audience"),
            this.clone(),
            self.audience().to_string().into(),
        )
        .assert(transaction);
        Relation::new(the!("ucan/subject"), this.clone(), subject).assert(transaction);
        Relation::new(the!("ucan/cmd"), this, self.command().to_string().into())
            .assert(transaction);
    }

    /// Retracts this delegation's facts from the transaction.
    fn retract(self, transaction: &mut Transaction) {
        let this = self.this();
        let subject: Value = match self.subject() {
            DelegatedSubject::Specific(did) => did.to_string().into(),
            DelegatedSubject::Any => "*".to_string().into(),
        };

        Relation::new(the!("db/blob"), this.clone(), self.to_bytes().into()).retract(transaction);
        Relation::new(
            the!("ucan/issuer"),
            this.clone(),
            self.issuer().to_string().into(),
        )
        .retract(transaction);
        Relation::new(
            the!("ucan/audience"),
            this.clone(),
            self.audience().to_string().into(),
        )
        .retract(transaction);
        Relation::new(the!("ucan/subject"), this.clone(), subject).retract(transaction);
        Relation::new(the!("ucan/cmd"), this, self.command().to_string().into())
            .retract(transaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Operator;
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
    fn it_exposes_delegation_fields() {
        let delegation = make_test_delegation();

        assert!(delegation.issuer().to_string().starts_with("did:key:"));
        assert!(delegation.audience().to_string().starts_with("did:key:"));
        match delegation.subject() {
            DelegatedSubject::Specific(did) => {
                assert!(did.to_string().starts_with("did:key:"));
            }
            DelegatedSubject::Any => panic!("Expected specific subject"),
        }
        assert_eq!(delegation.command().to_string(), "/read/write");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_roundtrips_through_serde() {
        let delegation = make_test_delegation();

        let bytes = serde_ipld_dagcbor::to_vec(&delegation).expect("Failed to encode");
        let decoded: Delegation = serde_ipld_dagcbor::from_slice(&bytes).expect("Failed to decode");

        assert_eq!(
            delegation.issuer().to_string(),
            decoded.issuer().to_string()
        );
        assert_eq!(
            delegation.audience().to_string(),
            decoded.audience().to_string()
        );
        assert_eq!(
            delegation.command().to_string(),
            decoded.command().to_string()
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_serializes_transparently() {
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

        let ucan_bytes =
            serde_ipld_dagcbor::to_vec(&ucan_delegation).expect("Failed to encode UcanDelegation");

        let delegation = Delegation::from(ucan_delegation);
        let delegation_bytes =
            serde_ipld_dagcbor::to_vec(&delegation).expect("Failed to encode Delegation");

        assert_eq!(
            ucan_bytes, delegation_bytes,
            "Delegation should serialize identically to UcanDelegation"
        );

        let decoded: Delegation =
            serde_ipld_dagcbor::from_slice(&ucan_bytes).expect("Failed to decode as Delegation");
        assert_eq!(
            delegation.issuer().to_string(),
            decoded.issuer().to_string()
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_produces_valid_cid() {
        let delegation = make_test_delegation();

        let cid = delegation.cid();
        let cid_str = format!("ucan:{}", cid);

        let entity = Entity::from_str(&cid_str);
        assert!(entity.is_ok(), "CID should be valid Entity URI");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_returns_entity_from_this() {
        let delegation = make_test_delegation();

        let entity = delegation.this();
        assert!(
            entity.to_string().starts_with("ucan:"),
            "Entity should have ucan: prefix"
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_produces_deterministic_cid() {
        let delegation = make_test_delegation();

        let cid1 = delegation.cid();
        let cid2 = delegation.cid();

        assert_eq!(cid1, cid2, "CID should be deterministic");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_validates_delegation_without_expiration() {
        let delegation = make_test_delegation();
        let now = Timestamp::now();

        assert!(delegation.validate(now).is_ok());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_creates_powerline_delegation() {
        let issuer = Operator::generate();
        let audience = Operator::generate();

        let ucan_delegation = UcanDelegation::builder()
            .issuer(Ed25519Signer::from(&issuer))
            .audience(audience.did().clone())
            .subject(DelegatedSubject::Any)
            .command(vec!["read".to_string()])
            .try_build()
            .expect("Failed to build delegation");

        let delegation = Delegation::from(ucan_delegation);

        assert!(delegation.is_powerline());
        assert!(matches!(delegation.subject(), DelegatedSubject::Any));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_provides_access_to_inner() {
        let delegation = make_test_delegation();

        let inner = delegation.inner();
        assert_eq!(inner.issuer().to_string(), delegation.issuer().to_string());
    }
}
