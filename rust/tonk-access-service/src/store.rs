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
    /// The account this subscription is for.
    pub account: String,
    /// The email activation was (or will be) sent to.
    pub email: String,
    /// The space this service replicates its accounting into, once one
    /// exists. A deployment that replicates nothing names none.
    pub ledger: Option<String>,
    /// Lifecycle state.
    pub status: CustomerStatus,
    /// The plan the customer is on.
    pub plan: String,
    /// Activation time as a unix timestamp in seconds; zero while
    /// `Registered`.
    pub verified_at: u64,
    /// Terms version accepted at activation.
    pub terms_version: Option<String>,
}

/// What the provisioning gate reads about one subject.
///
/// Rows rather than a verdict: deciding is `provisioning::screen`'s job,
/// and keeping the policy there means the store stays a store.
#[derive(Debug, Clone, Default)]
pub struct Servability {
    /// The subject's own registration, when the subject is a customer.
    pub own: Option<CustomerStatus>,
    /// Whether a consumer row exists for the subject at all.
    pub consumer: bool,
    /// When the subscription expires, if it does.
    pub expires_at: Option<u64>,
    /// When a purge began, if one has.
    pub deleted_at: Option<u64>,
    /// The suspension on this subscription, if it carries one.
    pub suspension: Option<Suspension>,
    /// When the data was dropped for non-payment, if it was.
    pub archived_at: Option<u64>,
    /// The provider's registration, when the provider is a customer.
    pub provider: Option<CustomerStatus>,
}

/// A suspension recorded against one subscription.
///
/// The code is what a client matches on and the message is what a person
/// reads; both are written together so a suspension can always explain
/// itself. `until` absent means indefinite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suspension {
    /// Machine-readable reason.
    pub code: String,
    /// What to tell a person.
    pub message: String,
    /// When it lifts on its own, if it does.
    pub until: Option<u64>,
}

/// A space the service replicates, servable only while `provider` names
/// an active customer.
#[derive(Debug, Clone)]
pub struct Subscription {
    /// The DID this subscription is for.
    pub consumer: String,
    /// The customer paying for this subscription.
    pub provider: String,
    /// Registration time as a unix timestamp in seconds.
    pub registered_at: u64,
    /// What this consumer is: a user's data space, or a custody
    /// namespace the account provisions for its own key material.
    pub kind: SubscriptionKind,
    /// When deletion began, if it has. The row disappears once it
    /// finishes, so this is set only while a purge is in flight.
    pub deleted_at: Option<u64>,
    /// While set and in the future, this DID is only held — the name is
    /// claimed but nothing is served under it. Null once provisioned,
    /// which is what a claimed row looks like.
    pub expires_at: Option<u64>,
}

/// What a consumer namespace holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionKind {
    /// A user's data space — reviewable, individually deletable.
    Space,
    /// A passkey's custody namespace — account plumbing. Never shown in
    /// a deletion review; purged by customer finalization, last, so the
    /// deletion machinery cannot destroy the account's own key custody
    /// while anything might still need it.
    Custody,
    /// The service's own bookkeeping space for a customer. Written at
    /// enrollment beside the customer's subscription, because the
    /// receipt hands the account a grant to read it and the gate serves
    /// nothing for an unprovisioned subject. Like custody, it is the
    /// account's plumbing rather than a space the user made.
    Ledger,
}

impl SubscriptionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Custody => "custody",
            Self::Ledger => "ledger",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "space" => Ok(Self::Space),
            "custody" => Ok(Self::Custody),
            "ledger" => Ok(Self::Ledger),
            other => Err(StoreError::Internal(format!(
                "unknown consumer kind: {other}"
            ))),
        }
    }
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

    /// Look up the customer registered under a normalized email address.
    /// The address must already be in
    /// [`normalize_email`](crate::email::normalize_email) form; this does
    /// not normalize it, because a caller that skipped normalization has
    /// a bug the lookup should not paper over.
    ///
    /// At most one customer holds an address: `customer_email` is unique.
    async fn customer_by_email(&self, email: &str) -> Result<Option<Customer>, StoreError>;

    /// Look up a consumer by DID.
    async fn consumer(&self, did: &str) -> Result<Option<Subscription>, StoreError>;

    /// Read everything the provisioning gate decides on, in one query.
    async fn servability(&self, did: &str) -> Result<Servability, StoreError>;

    /// List every consumer originally provided by one account, including
    /// deleted rows whose live provider has been cleared.
    async fn subscriptions_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<Subscription>, StoreError>;

    /// Atomically write a new customer row together with its self-provided
    /// account consumer. Two steps would leave a window in which a
    /// consumer exists with no provider.
    async fn enroll_customer(
        &self,
        did: &str,
        email: &str,
        plan: &str,
        ledger: &str,
        now: u64,
        expires_at: u64,
    ) -> Result<(), StoreError>;

    /// Update the email of a customer still `Registered`. Returns whether
    /// a row changed; an `Active` customer's email is never touched here.
    async fn update_registered_email(&self, did: &str, email: &str) -> Result<bool, StoreError>;

    /// Provision `did` as a consumer under `provider`. Idempotent for the
    /// same provider; answers false when a different customer already
    /// provides it, which the caller reports as a conflict.
    async fn add_subscription(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: SubscriptionKind,
    ) -> Result<bool, StoreError>;

    /// Record a suspension against one subscription, replacing any that
    /// stands. Answers false when no live subscription matched.
    async fn suspend_subscription(
        &self,
        consumer: &str,
        code: &str,
        message: &str,
        until: Option<u64>,
    ) -> Result<bool, StoreError>;

    /// Lift a suspension. Answers false when no live subscription
    /// matched; lifting one that is not suspended is not an error.
    async fn resume_subscription(&self, consumer: &str) -> Result<bool, StoreError>;

    /// Drop a subscription's data, keeping the row for billing. Answers
    /// false when no live, unarchived subscription matched.
    async fn archive_subscription(&self, consumer: &str, now: u64) -> Result<bool, StoreError>;

    /// Atomically deny future storage operations before object removal.
    async fn mark_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError>;

    /// Remove a subscription once its object prefix is empty.
    async fn finish_consumer_deletion(&self, did: &str) -> Result<bool, StoreError>;

    /// Deny the customer's own account-space consumer after root authorization.
    async fn mark_self_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError>;

    /// Remove the self consumer and customer row only when all other owned
    /// consumers are already deleted. Returns whether the customer was removed.
    async fn delete_customer(&self, did: &str) -> Result<bool, StoreError>;

    /// Claim the right to send this customer's activation link again,
    /// recording the moment. Answers false when the customer is not
    /// `Registered` or the last send was too recent — the caller sends
    /// only when this says yes, so the limit cannot be raced.
    async fn claim_activation_resend(
        &self,
        account: &str,
        now: u64,
        not_since: u64,
    ) -> Result<bool, StoreError>;

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
SELECT account, email, ledger, status, plan, verified_at, terms_version
  FROM customer WHERE account = ?1
"#;

/// Lookup by address, which `customer_email` makes unique.
pub const SELECT_CUSTOMER_BY_EMAIL: &str = r#"
SELECT account, email, ledger, status, plan, verified_at, terms_version
  FROM customer WHERE email = ?1
"#;

pub const SELECT_SUBSCRIPTION: &str = r#"
SELECT consumer, provider, registered_at, kind, deleted_at, expires_at
  FROM subscription WHERE consumer = ?1
"#;

/// Everything the provisioning gate decides on, in one round trip.
///
/// The gate runs before every presign, and answered from three separate
/// selects it could read a customer that activates before the third one
/// runs. One statement removes both the round trips and that window.
///
/// A subject may be a customer, a consumer, or both — an account is both,
/// since enrollment writes it a self-provided consumer row. Every column
/// is therefore nullable and the caller decides; this only gathers.
pub const SELECT_SERVABILITY: &str = r#"
SELECT own.status           AS own_status,
       sub.consumer         AS consumer_did,
       sub.expires_at       AS consumer_expires_at,
       sub.deleted_at       AS consumer_deleted_at,
       sub.suspend_code     AS consumer_suspend_code,
       sub.suspend_message  AS consumer_suspend_message,
       sub.suspend_until_at AS consumer_suspend_until_at,
       sub.archived_at      AS consumer_archived_at,
       provider.status      AS provider_status
  FROM (SELECT ?1 AS did) AS asked
  LEFT JOIN customer own      ON own.account = asked.did
  LEFT JOIN subscription sub  ON sub.consumer = asked.did
  LEFT JOIN customer provider ON provider.account = sub.provider
"#;

pub const SELECT_SUBSCRIPTIONS_BY_OWNER: &str = r#"
SELECT consumer, provider, registered_at, kind, deleted_at, expires_at
  FROM subscription WHERE provider = ?1 ORDER BY registered_at, consumer
"#;

pub const INSERT_CUSTOMER: &str = r#"
INSERT INTO customer (account, email, status, plan, cycle_anchor_at)
VALUES (?1, ?2, 'Registered', ?3, ?4)
"#;

/// The customer's own account space is a subscription like any other,
/// and the customer provides it.
///
/// It expires at the activation deadline. Enrollment writes the whole
/// state up front — cell, customer, subscriptions — and none of it is
/// served while the customer is `Registered`, so a link nobody clicks
/// leaves rows that clear themselves rather than an account half made.
/// Activation lifts the expiry.
///
/// `ON CONFLICT DO UPDATE` rather than `DO NOTHING`: re-enrolling
/// extends the deadline, which is what resending the link means.
pub const INSERT_SELF_SUBSCRIPTION: &str = r#"
INSERT INTO subscription (consumer, provider, registered_at, expires_at)
VALUES (?1, ?1, ?2, ?3)
ON CONFLICT (consumer) DO UPDATE SET expires_at = excluded.expires_at
 WHERE subscription.deleted_at IS NULL
"#;

/// The customer's ledger space, provisioned under them.
///
/// Enrollment hands the account a `/use/get` over this space, and the
/// gate serves nothing for an unprovisioned subject — so the grant
/// would name a space the service then refused to read. Written in the
/// same transaction as the customer, on the same expiry, because it is
/// as much a part of the account as the account's own subscription.
///
/// `kind = 'ledger'` so deletion can tell the service's own bookkeeping
/// from a space the user made.
pub const INSERT_LEDGER_SUBSCRIPTION: &str = r#"
INSERT INTO subscription (consumer, provider, registered_at, expires_at, kind)
VALUES (?1, ?2, ?3, ?4, 'ledger')
ON CONFLICT (consumer) DO UPDATE SET expires_at = excluded.expires_at
 WHERE subscription.deleted_at IS NULL
"#;

/// Record that the activation link was sent, but only if enough time
/// has passed since the last one.
///
/// The rate limit is the write: no row changes when it is too soon, so
/// the caller learns whether to send by whether this took. Doing it as
/// one statement means two requests arriving together cannot both
/// decide they are first.
pub const RECORD_ACTIVATION_SENT: &str = r#"
UPDATE customer
   SET activation_sent_at = ?2
 WHERE account = ?1
   AND status = 'Registered'
   AND (activation_sent_at IS NULL OR activation_sent_at <= ?3)
"#;

/// Lift the expiry from everything a customer provides, which is what
/// activation does: the rows were written to lapse if the link was
/// never clicked, and it has been.
pub const ACTIVATE_SUBSCRIPTIONS: &str = r#"
UPDATE subscription
   SET expires_at = NULL
 WHERE provider = ?1 AND deleted_at IS NULL
"#;

/// Provisioning is idempotent per provider: re-adding under the same
/// customer re-runs the update, while a consumer someone else provides
/// matches no row and changes nothing.
///
/// A reservation another customer holds is refused for as long as it
/// stands, and claimable once it lapses — `?3` is the current time, so
/// the comparison is against this call rather than a swept state.
/// Claiming clears `expires_at`, which is what a provisioned row
/// looks like: null never lapses.
pub const ADD_SUBSCRIPTION: &str = r#"
INSERT INTO subscription (consumer, provider, registered_at, kind)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (consumer) DO UPDATE SET
  provider = excluded.provider,
  kind = excluded.kind,
  expires_at = NULL
WHERE subscription.provider = excluded.provider
  AND subscription.deleted_at IS NULL
"#;

/// Reserve `did` for `provider` until `expires_at`, so the window
/// between naming a space and provisioning it cannot be raced.
///
/// The same guard as [`ADD_SUBSCRIPTION`]: a free DID, one this provider
/// already holds, or one whose reservation has lapsed. Re-reserving
/// extends the deadline, which is what a passkey re-enrolling on a new
/// device does.
pub const UPDATE_REGISTERED_EMAIL: &str = r#"
UPDATE customer SET email = ?2 WHERE account = ?1 AND status = 'Registered'
"#;

pub const ACTIVATE_CUSTOMER: &str = r#"
UPDATE customer
   SET status = 'Active',
       verified_at = ?2,
       terms_version = ?3,
       terms_accepted_at = ?2,
       cycle_anchor_at = ?2
 WHERE account = ?1 AND status = 'Registered'
"#;

/// Record a suspension. Overwrites any suspension already standing:
/// re-suspending with a new reason replaces the old one rather than
/// layering, so a row always carries exactly the reason in force.
pub const SUSPEND_SUBSCRIPTION: &str = r#"
UPDATE subscription
   SET suspend_code = ?2, suspend_message = ?3, suspend_until_at = ?4
 WHERE consumer = ?1 AND deleted_at IS NULL
"#;

/// Lift a suspension, clearing the reason with it: a row that says it is
/// suspended and cannot say why is one nothing can explain.
pub const RESUME_SUBSCRIPTION: &str = r#"
UPDATE subscription
   SET suspend_code = NULL, suspend_message = NULL, suspend_until_at = NULL
 WHERE consumer = ?1 AND deleted_at IS NULL
"#;

/// Drop a subscription's data while keeping the row, because what it
/// accrued still has to be billed.
pub const ARCHIVE_SUBSCRIPTION: &str = r#"
UPDATE subscription
   SET archived_at = ?2
 WHERE consumer = ?1 AND deleted_at IS NULL AND archived_at IS NULL
"#;

pub const START_SUBSCRIPTION_DELETION: &str = r#"
UPDATE subscription SET deleted_at = ?2
 WHERE consumer = ?1 AND deleted_at IS NULL
"#;

pub const REMOVE_SUBSCRIPTION: &str = r#"
DELETE FROM subscription
 WHERE consumer = ?1 AND deleted_at IS NOT NULL
"#;

pub const START_SELF_SUBSCRIPTION_DELETION: &str = r#"
UPDATE subscription SET deleted_at = ?2
 WHERE consumer = ?1 AND provider = ?1 AND deleted_at IS NULL
"#;

/// Drop the purged subscriptions of a deleted customer.
///
/// They used to be kept, with the provider blanked, so nothing could
/// provision the same space DID again. That marker cannot survive a
/// required provider, and it was guarding little: only the holder of a
/// space's key can present its DID at all, and a customer who deletes
/// their account and returns with the same space is a customer, not an
/// attacker.
pub const DELETE_PURGED_SUBSCRIPTIONS: &str = r#"
DELETE FROM subscription
 WHERE provider = ?1
   AND consumer <> ?1
"#;

pub const DELETE_SELF_SUBSCRIPTION: &str = r#"
DELETE FROM subscription
 WHERE consumer = ?1
   AND provider = ?1
   AND deleted_at IS NOT NULL
"#;

pub const DELETE_CUSTOMER: &str = r#"
DELETE FROM customer
 WHERE account = ?1
   AND NOT EXISTS (SELECT 1 FROM subscription WHERE provider = ?1)
"#;

pub mod ingest;

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod sqlite;

#[cfg(target_arch = "wasm32")]
pub mod d1;
