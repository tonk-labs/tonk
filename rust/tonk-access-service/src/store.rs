//! Control-state storage for customer registration.
//!
//! The ordered SQL files under `migrations/` are the schema's source of
//! truth: Wrangler applies them to the control D1 database and
//! [`sqlite::SqliteStore`] applies the same sequence natively. Both
//! backends implement [`Store`], so registration logic elsewhere in this
//! crate is written once, generically over the trait.

use async_trait::async_trait;
use tonk_account::customer::CustomerStatus;

/// The plan a customer lands on at activation. Repricing inserts a new
/// plan row (rows are immutable), so a successor row also updates this
/// constant.
pub const SIGNUP_PLAN: &str = "trial@2026-08";

/// Errors surfaced by a [`Store`] implementation.
#[derive(Debug)]
pub enum StoreError {
    /// A uniqueness or primary-key constraint was violated.
    Conflict(String),
    /// Any other storage failure.
    Internal(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Conflict(detail) => write!(f, "conflict: {detail}"),
            StoreError::Internal(detail) => write!(f, "storage failure: {detail}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Parse a stored status column; unknown values are an internal error.
pub fn parse_status(value: &str) -> Result<CustomerStatus, StoreError> {
    CustomerStatus::parse(value).map_err(StoreError::Internal)
}

/// A billable party, keyed by the DID that also names its account
/// consumer.
#[derive(Debug, Clone)]
pub struct Customer {
    /// The customer's DID.
    pub did: String,
    /// The email activation was (or will be) sent to.
    pub email: String,
    /// Lifecycle state.
    pub status: CustomerStatus,
    /// The plan the customer is on.
    pub plan: String,
    /// Activation time as a unix timestamp in seconds; zero while
    /// `Registered`.
    pub verified: u64,
    /// Terms version accepted at activation.
    pub terms_version: Option<String>,
}

/// A space the service replicates, servable only while `provider` names
/// an active customer.
#[derive(Debug, Clone)]
pub struct Consumer {
    /// The consumer space's DID.
    pub did: String,
    /// The customer paying for this consumer; null means not servable.
    pub provider: Option<String>,
    /// Registration time as a unix timestamp in seconds.
    pub registered: u64,
}

/// Storage operations registration needs. Declared through the dual
/// `async_trait` forms dialog uses, so the trait itself promises `Send`
/// futures natively (and nothing on wasm32) and generic consumers can be
/// written once.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Store {
    /// Look up a customer by DID.
    async fn customer(&self, did: &str) -> Result<Option<Customer>, StoreError>;

    /// Look up a consumer by DID.
    async fn consumer(&self, did: &str) -> Result<Option<Consumer>, StoreError>;

    /// Atomically write a new customer row together with its self-provided
    /// account consumer. Two steps would leave a window in which a
    /// consumer exists with no provider.
    async fn enroll_customer(
        &self,
        did: &str,
        email: &str,
        access: &[u8],
        plan: &str,
        now: u64,
    ) -> Result<(), StoreError>;

    /// Update the email of a customer still `Registered`. Returns whether
    /// a row changed; an `Active` customer's email is never touched here.
    async fn update_registered_email(&self, did: &str, email: &str) -> Result<bool, StoreError>;

    /// Provision `did` as a consumer under `provider`. Idempotent for the
    /// same provider; answers false when a different customer already
    /// provides it, which the caller reports as a conflict.
    async fn add_consumer(&self, did: &str, provider: &str, now: u64) -> Result<bool, StoreError>;

    /// Promote a `Registered` customer to `Active`, recording the
    /// activation time, terms acceptance, and cycle anchor. Returns false
    /// when no `Registered` row matched, which the caller disambiguates
    /// by reading the customer back.
    async fn activate_customer(
        &self,
        did: &str,
        terms_version: &str,
        now: u64,
    ) -> Result<bool, StoreError>;
}

// Shared query text. D1 is SQLite, so both backends issue byte-identical
// SQL and differ only in row decoding and error mapping.

pub const SELECT_CUSTOMER: &str = r#"
SELECT did, email, status, plan, verified, terms_version
  FROM customer WHERE did = ?1
"#;

pub const SELECT_CONSUMER: &str = r#"
SELECT did, provider, registered FROM consumer WHERE did = ?1
"#;

pub const INSERT_CUSTOMER: &str = r#"
INSERT INTO customer (did, email, status, plan, cycle_anchor, access)
VALUES (?1, ?2, 'Registered', ?3, ?4, ?5)
"#;

/// The customer's own account space is a consumer like any other, and the
/// customer provides it. `ON CONFLICT DO NOTHING` keeps re-enrollment
/// idempotent when the consumer row survived an earlier attempt.
pub const INSERT_SELF_CONSUMER: &str = r#"
INSERT INTO consumer (did, provider, registered)
VALUES (?1, ?1, ?2)
ON CONFLICT (did) DO NOTHING
"#;

/// Provisioning is idempotent per provider: re-adding under the same
/// customer re-runs the update, while a consumer someone else provides
/// matches no row and changes nothing.
pub const ADD_CONSUMER: &str = r#"
INSERT INTO consumer (did, provider, registered)
VALUES (?1, ?2, ?3)
ON CONFLICT (did) DO UPDATE SET provider = excluded.provider
WHERE consumer.provider IS NULL OR consumer.provider = excluded.provider
"#;

pub const UPDATE_REGISTERED_EMAIL: &str = r#"
UPDATE customer SET email = ?2 WHERE did = ?1 AND status = 'Registered'
"#;

pub const ACTIVATE_CUSTOMER: &str = r#"
UPDATE customer
   SET status = 'Active',
       verified = ?2,
       terms_version = ?3,
       terms_accepted_at = ?2,
       cycle_anchor = ?2
 WHERE did = ?1 AND status = 'Registered'
"#;

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod sqlite;

#[cfg(target_arch = "wasm32")]
pub mod d1;
