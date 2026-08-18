//! Customer registration contracts, shared between the access service
//! and its clients.
//!
//! The registration commands are dialog capabilities. `/customer` is the
//! role the subject exercises: enrollment roots at the customer's own
//! self-certifying DID, activation at the service's, reaching the
//! customer as an emailed delegation. Both sides derive the command path
//! and the argument shape from the same types, so the wire format has one
//! definition.

use dialog_capability::{Attenuate, Attenuation, Effect, Subject};
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Ability segment `/customer`: acts the subject takes on its own
/// customer identity with the access service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Customer;

impl Attenuation for Customer {
    type Of = Subject;
}

/// `/customer/enroll` — become a customer. The invocation's subject is
/// the customer DID and the chain roots there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Enroll {
    /// Address the activation link is sent to.
    pub email: String,
    /// The deposited delegation granting the service access to the
    /// account space, by CID; its bytes travel in the same container. An
    /// argument, not a proof: it never extends the invocation's chain.
    pub access: Cid,
}

impl Effect for Enroll {
    type Of = Customer;
    type Output = Result<Receipt, RegistrationError>;
}

/// `/customer/activate` — finalize enrollment. The invocation's subject
/// is the service DID; the chain runs through the delegation enrollment
/// emailed, so presenting a verifying chain proves the link round-tripped
/// through the inbox back to the customer's key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Activate {
    /// The customer being activated; must match the audience of the
    /// service-issued link delegation in the chain.
    pub customer: Did,
    /// The terms-of-service version the accept button presented.
    pub terms: String,
}

impl Effect for Activate {
    type Of = Customer;
    type Output = Result<Receipt, RegistrationError>;
}

/// Ability segment `/provider`: acts a customer takes as the party
/// paying for consumer spaces. Distinct from `/customer` so delegating
/// one role can never leak the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider;

impl Attenuation for Provider {
    type Of = Subject;
}

/// `/provider/add` — provision a consumer space under the invoking
/// customer. The invocation's subject is the customer DID; the consumer's
/// consent travels as a deposited delegation named by CID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Add {
    /// The space being provisioned.
    pub consumer: Did,
    /// The consent delegation, by CID; its bytes travel in the same
    /// container. It must root at the consumer and be issued to the
    /// invoking customer, granting `/consumer/provision` or broader.
    pub consent: Cid,
}

impl Effect for Add {
    type Of = Provider;
    type Output = Result<ConsumerReceipt, RegistrationError>;
}

/// Ability segment `/consumer`: acts a space takes on its own behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consumer;

impl Attenuation for Consumer {
    type Of = Subject;
}

/// `/consumer/provision` — the consent a space delegates to the customer
/// it accepts as its provider; the delegation's audience is what names
/// that customer. Never invoked: it exists to be deposited with
/// [`Add`], and a powerline delegation satisfies it as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Attenuate)]
pub struct Provision;

impl Effect for Provision {
    type Of = Consumer;
    type Output = ();
}

/// The successful answer to a `/provider/add` invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsumerReceipt {
    /// The provisioned space.
    pub consumer: Did,
    /// The customer now providing it.
    pub provider: Did,
}

/// Customer lifecycle state, as stored and as answered on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CustomerStatus {
    /// Enrolled, activation email not yet acted on. Not servable.
    Registered,
    /// Email verified and terms accepted. Servable.
    Active,
    /// Service withdrawn by an operator decision.
    Suspended,
}

impl CustomerStatus {
    /// The stored column value for this status.
    pub fn as_str(&self) -> &'static str {
        match self {
            CustomerStatus::Registered => "Registered",
            CustomerStatus::Active => "Active",
            CustomerStatus::Suspended => "Suspended",
        }
    }

    /// Parse a stored column value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "Registered" => Ok(CustomerStatus::Registered),
            "Active" => Ok(CustomerStatus::Active),
            "Suspended" => Ok(CustomerStatus::Suspended),
            other => Err(format!("unknown customer status: {other}")),
        }
    }
}

/// The successful answer to a registration invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// The customer acted on.
    pub customer: Did,
    /// The customer's lifecycle state after the act.
    pub status: CustomerStatus,
}

/// A registration refusal. Serialized with the variant as a `code` tag,
/// so the wire carries the reason itself rather than a code table both
/// sides keep in step by hand.
#[derive(Debug, Clone, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "code")]
pub enum RegistrationError {
    /// The container or its arguments did not parse.
    #[error("{message}")]
    Invalid {
        /// What did not parse.
        message: String,
    },
    /// The chain did not verify or has expired.
    #[error("{message}")]
    Unauthorized {
        /// Which check did not hold.
        message: String,
    },
    /// The chain verified but does not permit this act.
    #[error("{message}")]
    Forbidden {
        /// What was not permitted.
        message: String,
    },
    /// No customer to act on.
    #[error("no such customer")]
    UnknownCustomer,
    /// The consumer already has a different provider.
    #[error("this consumer already has a provider")]
    ConsumerProvided,
    /// The customer is already active, so enrollment is refused.
    #[error("this customer is already active")]
    CustomerActive,
    /// The customer is suspended, so nothing self-serve applies.
    #[error("this customer is suspended")]
    CustomerSuspended,
    /// The service failed, not the caller.
    #[error("{message}")]
    Internal {
        /// What failed.
        message: String,
    },
}

impl RegistrationError {
    /// The HTTP status this refusal answers with.
    pub fn status(&self) -> u16 {
        match self {
            RegistrationError::Invalid { .. } => 400,
            RegistrationError::Unauthorized { .. } => 401,
            RegistrationError::Forbidden { .. } => 403,
            RegistrationError::UnknownCustomer => 404,
            RegistrationError::CustomerActive
            | RegistrationError::CustomerSuspended
            | RegistrationError::ConsumerProvided => 409,
            RegistrationError::Internal { .. } => 500,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use dialog_capability::{Capability, did};

    use super::*;

    fn subject() -> Subject {
        Subject::from(did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"))
    }

    #[test]
    fn it_derives_the_role_first_command_paths() {
        let enroll: Capability<Enroll> = subject().attenuate(Customer).invoke(Enroll {
            email: "alice@example.com".into(),
            access: Cid::default(),
        });
        assert_eq!(enroll.ability(), "/customer/enroll");

        let activate: Capability<Activate> = subject().attenuate(Customer).invoke(Activate {
            customer: did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"),
            terms: "2026-08".into(),
        });
        assert_eq!(activate.ability(), "/customer/activate");

        let add: Capability<Add> = subject().attenuate(Provider).invoke(Add {
            consumer: did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"),
            consent: Cid::default(),
        });
        assert_eq!(add.ability(), "/provider/add");

        let provision: Capability<Provision> = subject().attenuate(Consumer).invoke(Provision);
        assert_eq!(provision.ability(), "/consumer/provision");
    }

    #[test]
    fn it_serializes_the_refusal_reason_as_a_code_tag() {
        let value = serde_json::to_value(RegistrationError::CustomerActive).unwrap();
        assert_eq!(value["code"], "CustomerActive");
        let value = serde_json::to_value(RegistrationError::Unauthorized {
            message: "expired".into(),
        })
        .unwrap();
        assert_eq!(value["code"], "Unauthorized");
        assert_eq!(value["message"], "expired");
    }
}
