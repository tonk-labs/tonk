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
    /// The passkey's custody space, whose cell holds the sealed account
    /// secret. Named here so every carried block can be checked against
    /// it rather than trusted to agree with itself.
    pub custody: Did,
    /// The pre-signed `/use/put/memory/cell` invocation that writes the
    /// custody cell, by CID.
    ///
    /// Verified here and executed at activation. Self-signed by the
    /// custody key and proofless, so it verifies against that key alone
    /// and any holder can redeem it — which is what lets a browser other
    /// than the enrolling one finish the account.
    pub recovery: Cid,
    /// The custody space's consent to being provisioned by this
    /// account, by CID.
    pub consent: Cid,
    /// The sealed envelope [`Self::recovery`] checksums, by CID. Opaque
    /// ciphertext: the bytes the activation write puts in the cell.
    ///
    /// Required, like the rest of the custody material: an enrollment
    /// that carries none would leave an account no second device could
    /// open, which is the failure this whole path exists to prevent.
    pub sealed: Cid,
}

impl Effect for Enroll {
    type Of = Customer;
    type Output = Result<Receipt, RegistrationError>;
}

/// The archive catalog an enrollment deposit covers: the tree-node index
/// backing branch push and pull. Catalog reads still require knowing a
/// node's digest, which only the scoped memory cells reveal.
pub const SERVICE_CATALOG: &str = "index";

/// The memory space of the branch the service writes under the account:
/// named by the service's own DID, so the grant is per-service and
/// rotates with the key. A service whose identity changes re-enrolls
/// into a fresh branch rather than inheriting the old one.
pub fn service_space(service: &Did) -> String {
    format!("branch/{service}")
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
    /// What the consumer is: `"space"` (the default) for a user's data
    /// space, `"custody"` for a passkey's custody namespace — plumbing
    /// the account provisions for itself. The service keeps the kind so
    /// deletion can tell a user's spaces from the account's own key
    /// custody: custody namespaces never appear in a deletion review
    /// and are purged by customer finalization, last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl Effect for Add {
    type Of = Provider;
    type Output = Result<ConsumerReceipt, RegistrationError>;
}

/// `/provider/remove` — deprovision a hosted consumer space: the
/// reverse of [`Add`], and how a hosted space is deleted. The
/// invocation's subject is the owning customer DID; the service purges
/// the space's hosted content and denies the consumer forever. No
/// per-space artifact is presented — the customer's own chain is the
/// authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Remove {
    /// The hosted space being deprovisioned.
    pub consumer: Did,
}

impl Effect for Remove {
    type Of = Provider;
    type Output = Result<(), RegistrationError>;
}

/// Ability segment `/ucan`: acts defined by the UCAN specification
/// itself rather than by this service's roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ucan;

impl Attenuation for Ucan {
    type Of = Subject;
}

/// `/ucan/revoke` — withdraw a delegation, per the
/// [UCAN revocation spec](https://github.com/ucan-wg/revocation).
///
/// The subject is the principal whose authority is being exercised, and
/// it is what a validator matches against the issuers of a presented
/// chain. The invocation's issuer may differ, when revocation authority
/// was itself delegated.
///
/// Argument names are the spec's IPLD schema (`rev`, `pth`), not the
/// longer forms its prose uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Revoke {
    /// The delegation being withdrawn, by canonical CID.
    #[serde(rename = "rev")]
    pub revoke: Cid,
    /// The delegation path witnessing that the subject may revoke the
    /// target: root through target, each named by CID and carried as a
    /// block in the same container.
    ///
    /// The spec makes this optional, and names the reason to require it:
    /// storing revocations nobody was entitled to issue is a denial of
    /// service vector.
    #[serde(rename = "pth")]
    pub path: Vec<Cid>,
}

impl Effect for Revoke {
    type Of = Ucan;
    type Output = Result<RevokeReceipt, RegistrationError>;
}

/// The successful answer to a `/ucan/revoke` invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevokeReceipt {
    /// The delegation now recorded as revoked.
    pub revoked: Cid,
    /// The principal whose authority withdrew it.
    pub subject: Did,
    /// Whether this call recorded the revocation, as against finding it
    /// already present. Revocation is idempotent, so a replay answers
    /// success either way.
    pub recorded: bool,
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

/// `/customer/resend` — send the activation link again.
///
/// Self-issued on the service's own subject, and the account is an
/// argument rather than the subject: nobody presenting this holds the
/// account's key, which is the whole point — the person waiting for the
/// email cannot sign as the account they have not activated yet.
///
/// That makes it unauthenticated in effect, which is fine because the
/// mail only ever goes to the address already on the customer row. A
/// caller who names someone else's account causes mail to an inbox they
/// do not control. Two guards keep that from being a nuisance: it works
/// only while the customer is `Registered`, and no more often than
/// [`RESEND_INTERVAL_SECONDS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Resend {
    /// The account whose activation link to send again.
    pub account: Did,
}

impl Effect for Resend {
    type Of = Customer;
    type Output = ();
}

/// How often one customer's activation link may be resent. Short,
/// because the person is watching an inbox and will press the button
/// again; long enough that pressing it repeatedly sends one mail.
pub const RESEND_INTERVAL_SECONDS: u64 = 60;

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
    /// The provider serving this customer: the UCAN endpoint its
    /// spaces attach their remotes to.
    ///
    /// The service decides which provider serves its customers and says
    /// so here, rather than every client deriving an address from
    /// whichever origin its request happened to reach. A client records
    /// this and attaches spaces to it; it never has to guess.
    ///
    /// Optional so a receipt from a service that predates this field
    /// still decodes. An absent address means "unchanged", not "no
    /// provider": a client that already recorded one keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The ledger space this service owns for the customer, and a
    /// `/use/get` delegation letting the account read it.
    ///
    /// The service writes metering and billing there and the account
    /// only reads, so a client cannot rewrite its own record. Owning it
    /// is also what keeps the service's ledger out of the
    /// customer's reach: a delegation the client granted, the client
    /// could withdraw.
    ///
    /// Optional so a receipt from a service that predates this still
    /// decodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger: Option<Ledger>,
}

/// The service-owned ledger space named in a [`Receipt`], with the
/// authority to read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    /// The space's DID: the subject a client points a remote at.
    pub did: Did,
    /// Hex-encoded `/use/get` delegation chain, issued by the space to
    /// the account. Saved the way a joined space's authority is, and
    /// presented on every pull.
    pub read_hex: String,
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
    /// The subject is not a consumer this service holds anything for, so
    /// a revocation about it would guard nothing. Distinct from an
    /// unactivated customer: that one has data here and may still
    /// revoke.
    #[error("this subject is not a registered consumer")]
    UnknownConsumer,
    /// The customer is already active, so enrollment is refused.
    #[error("this customer is already active")]
    CustomerActive,
    /// The address belongs to another account. One address holds one
    /// customer — that is what the lookup resolving an email to a
    /// `did:key` depends on — so this is a refusal the enrolling client
    /// can act on, not a storage failure.
    #[error("this email address is registered to another account")]
    AddressTaken,
    /// The customer enrolled but has not confirmed their email address,
    /// so nothing may be provisioned under them yet. Recoverable by the
    /// customer alone: re-enrolling resends the activation email.
    #[error("this customer is awaiting email activation")]
    CustomerInactive,
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
            RegistrationError::UnknownConsumer => 404,
            RegistrationError::CustomerActive
            | RegistrationError::CustomerInactive
            | RegistrationError::CustomerSuspended
            | RegistrationError::AddressTaken
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
            custody: did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"),
            recovery: Cid::default(),
            consent: Cid::default(),
            sealed: Cid::default(),
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
            kind: None,
        });
        assert_eq!(add.ability(), "/provider/add");

        // The spec's own command, so it is `/ucan/revoke` rather than
        // one of this service's roles.
        let revoke: Capability<Revoke> = subject().attenuate(Ucan).invoke(Revoke {
            revoke: Cid::default(),
            path: vec![Cid::default()],
        });
        assert_eq!(revoke.ability(), "/ucan/revoke");

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
