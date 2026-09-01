//! Cloudflare D1-backed twin of the [`Store`](crate::store::Store) trait,
//! for production use.
//!
//! D1 is SQLite, so this implementation issues the same query strings as
//! [`SqliteStore`](crate::store::sqlite::SqliteStore) — hoisted as shared
//! `const` items on [`crate::store`] — and deserializes rows through
//! serde structs instead of `rusqlite`'s row API. D1 surfaces numbers as
//! JS `f64`; the timestamps this crate deals with fit losslessly.

use async_trait::async_trait;
use serde::Deserialize;
use worker::d1::D1Database;
use worker::wasm_bindgen::JsValue;

use crate::store::{
    ACTIVATE_CUSTOMER, ACTIVATE_SUBSCRIPTIONS, ADD_SUBSCRIPTION, ARCHIVE_SUBSCRIPTION, Customer,
    DELETE_CUSTOMER, DELETE_PURGED_SUBSCRIPTIONS, DELETE_SELF_SUBSCRIPTION, Enrollment,
    INSERT_CUSTODY_SUBSCRIPTION, INSERT_CUSTOMER, INSERT_LEDGER_SUBSCRIPTION,
    INSERT_SELF_SUBSCRIPTION, RECORD_ACTIVATION_SENT, RELEASE_LAPSED_ADDRESS, REMOVE_SUBSCRIPTION,
    RESUME_SUBSCRIPTION, SELECT_CUSTOMER, SELECT_CUSTOMER_BY_EMAIL, SELECT_SERVABILITY,
    SELECT_SUBSCRIPTION, SELECT_SUBSCRIPTIONS_BY_OWNER, START_SELF_SUBSCRIPTION_DELETION,
    START_SUBSCRIPTION_DELETION, SUSPEND_SUBSCRIPTION, Servability, Store, StoreError,
    Subscription, SubscriptionKind, Suspension, UPDATE_REGISTERED_EMAIL, parse_status,
};

/// Cloudflare D1-backed [`Store`], for production use.
pub struct D1Store(D1Database);

impl D1Store {
    /// Wrap a D1 database binding.
    pub fn new(db: D1Database) -> Self {
        Self(db)
    }
}

/// Map a D1 error onto [`StoreError`]. D1 surfaces constraint violations
/// as generic errors, so uniqueness conflicts are detected by matching
/// the error message's text.
fn map_err(err: worker::Error) -> StoreError {
    let message = err.to_string();
    if message.contains("UNIQUE constraint failed") || message.contains("PRIMARY KEY") {
        StoreError::Conflict(message)
    } else {
        StoreError::Internal(message)
    }
}

/// A customer row as deserialized straight off a D1 query, before the
/// status column is parsed.
#[derive(Deserialize)]
struct CustomerRowD1 {
    account: String,
    ledger: Option<String>,
    email: String,
    status: String,
    plan: String,
    verified_at: f64,
    terms_version: Option<String>,
}

impl TryFrom<CustomerRowD1> for Customer {
    type Error = StoreError;

    fn try_from(row: CustomerRowD1) -> Result<Self, StoreError> {
        Ok(Customer {
            account: row.account,
            email: row.email,
            ledger: row.ledger,
            status: parse_status(&row.status)?,
            plan: row.plan,
            verified_at: row.verified_at as u64,
            terms_version: row.terms_version,
        })
    }
}

/// The gate's joined row, by the aliases `SELECT_SERVABILITY` gives its
/// columns: two `status` columns share a name without them.
#[derive(Deserialize)]
struct ServabilityRowD1 {
    own_status: Option<String>,
    consumer_did: Option<String>,
    consumer_expires_at: Option<f64>,
    consumer_deleted_at: Option<f64>,
    consumer_suspend_code: Option<String>,
    consumer_suspend_message: Option<String>,
    consumer_suspend_until_at: Option<f64>,
    consumer_archived_at: Option<f64>,
    provider_status: Option<String>,
}

impl TryFrom<ServabilityRowD1> for Servability {
    type Error = StoreError;

    fn try_from(row: ServabilityRowD1) -> Result<Self, Self::Error> {
        Ok(Servability {
            own: row.own_status.as_deref().map(parse_status).transpose()?,
            consumer: row.consumer_did.is_some(),
            expires_at: row.consumer_expires_at.map(|value| value as u64),
            deleted_at: row.consumer_deleted_at.map(|value| value as u64),
            // The code and the message are written together, so one
            // without the other is a row nothing could explain.
            suspension: row.consumer_suspend_code.map(|code| Suspension {
                message: row.consumer_suspend_message.unwrap_or_default(),
                code,
                until: row.consumer_suspend_until_at.map(|value| value as u64),
            }),
            archived_at: row.consumer_archived_at.map(|value| value as u64),
            provider: row
                .provider_status
                .as_deref()
                .map(parse_status)
                .transpose()?,
        })
    }
}

/// A consumer row as deserialized straight off a D1 query.
#[derive(Deserialize)]
struct SubscriptionRowD1 {
    consumer: String,
    provider: String,
    registered_at: f64,
    kind: String,
    deleted_at: Option<f64>,
    expires_at: Option<f64>,
}

impl TryFrom<SubscriptionRowD1> for Subscription {
    type Error = StoreError;

    fn try_from(row: SubscriptionRowD1) -> Result<Self, Self::Error> {
        Ok(Subscription {
            consumer: row.consumer,
            provider: row.provider,
            registered_at: row.registered_at as u64,
            kind: SubscriptionKind::parse(&row.kind)?,
            deleted_at: row.deleted_at.map(|value| value as u64),
            expires_at: row.expires_at.map(|value| value as u64),
        })
    }
}

#[async_trait(?Send)]
impl Store for D1Store {
    async fn customer(&self, did: &str) -> Result<Option<Customer>, StoreError> {
        let row: Option<CustomerRowD1> = self
            .0
            .prepare(SELECT_CUSTOMER)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Customer::try_from).transpose()
    }

    async fn customer_by_email(&self, email: &str) -> Result<Option<Customer>, StoreError> {
        let row: Option<CustomerRowD1> = self
            .0
            .prepare(SELECT_CUSTOMER_BY_EMAIL)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Customer::try_from).transpose()
    }

    async fn servability(&self, did: &str) -> Result<Servability, StoreError> {
        let row: Option<ServabilityRowD1> = self
            .0
            .prepare(SELECT_SERVABILITY)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        // `(SELECT ?1)` always yields a row, so an absent one means the
        // query found nothing — the same answer as a subject with no rows.
        row.map(Servability::try_from)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    async fn consumer(&self, did: &str) -> Result<Option<Subscription>, StoreError> {
        let row: Option<SubscriptionRowD1> = self
            .0
            .prepare(SELECT_SUBSCRIPTION)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Subscription::try_from).transpose()
    }

    async fn subscriptions_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<Subscription>, StoreError> {
        let result = self
            .0
            .prepare(SELECT_SUBSCRIPTIONS_BY_OWNER)
            .bind(&[JsValue::from(provider)])
            .map_err(map_err)?
            .all()
            .await
            .map_err(map_err)?;
        result
            .results::<SubscriptionRowD1>()
            .map_err(map_err)?
            .into_iter()
            .map(Subscription::try_from)
            .collect()
    }

    async fn enroll_customer(&self, enrollment: Enrollment<'_>) -> Result<(), StoreError> {
        let Enrollment {
            did,
            email,
            plan,
            ledger,
            custody,
            now,
            expires_at,
        } = enrollment;
        // D1 batches run as a single transaction: either every statement
        // commits or none does.
        let insert_customer = self
            .0
            .prepare(INSERT_CUSTOMER)
            .bind(&[
                JsValue::from(did),
                JsValue::from(email),
                JsValue::from(ledger),
                JsValue::from(plan),
                JsValue::from_f64(now as f64),
            ])
            .map_err(map_err)?;
        let insert_consumer = self
            .0
            .prepare(INSERT_SELF_SUBSCRIPTION)
            .bind(&[
                JsValue::from(did),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(expires_at as f64),
            ])
            .map_err(map_err)?;
        let insert_ledger = self
            .0
            .prepare(INSERT_LEDGER_SUBSCRIPTION)
            .bind(&[
                JsValue::from(ledger),
                JsValue::from(did),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(expires_at as f64),
            ])
            .map_err(map_err)?;
        let insert_custody = self
            .0
            .prepare(INSERT_CUSTODY_SUBSCRIPTION)
            .bind(&[
                JsValue::from(custody),
                JsValue::from(did),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(expires_at as f64),
            ])
            .map_err(map_err)?;
        // One batch, which D1 runs as a single transaction: an enrollment
        // claims the customer and every namespace it needs, or claims
        // none. Split across separate writes, a failure between them left
        // a customer whose own passkey could not be read back.
        self.0
            .batch(vec![
                insert_customer,
                insert_consumer,
                insert_ledger,
                insert_custody,
            ])
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn update_registered_email(&self, did: &str, email: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(UPDATE_REGISTERED_EMAIL)
            .bind(&[JsValue::from(did), JsValue::from(email)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn release_lapsed_address(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(RELEASE_LAPSED_ADDRESS)
            .bind(&[JsValue::from(did), JsValue::from(now as f64)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn add_subscription(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: SubscriptionKind,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(ADD_SUBSCRIPTION)
            .bind(&[
                JsValue::from(did),
                JsValue::from(provider),
                JsValue::from_f64(now as f64),
                JsValue::from(kind.as_str()),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn suspend_subscription(
        &self,
        consumer: &str,
        code: &str,
        message: &str,
        until: Option<u64>,
    ) -> Result<bool, StoreError> {
        let until = match until {
            Some(at) => JsValue::from_f64(at as f64),
            None => JsValue::NULL,
        };
        let result = self
            .0
            .prepare(SUSPEND_SUBSCRIPTION)
            .bind(&[
                JsValue::from(consumer),
                JsValue::from(code),
                JsValue::from(message),
                until,
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn resume_subscription(&self, consumer: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(RESUME_SUBSCRIPTION)
            .bind(&[JsValue::from(consumer)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn archive_subscription(&self, consumer: &str, now: u64) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(ARCHIVE_SUBSCRIPTION)
            .bind(&[JsValue::from(consumer), JsValue::from_f64(now as f64)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn mark_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(START_SUBSCRIPTION_DELETION)
            .bind(&[JsValue::from(did), JsValue::from_f64(now as f64)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn finish_consumer_deletion(&self, did: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(REMOVE_SUBSCRIPTION)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn mark_self_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(START_SELF_SUBSCRIPTION_DELETION)
            .bind(&[JsValue::from(did), JsValue::from_f64(now as f64)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn delete_customer(&self, did: &str) -> Result<bool, StoreError> {
        let anonymize = self
            .0
            .prepare(DELETE_PURGED_SUBSCRIPTIONS)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?;
        let self_consumer = self
            .0
            .prepare(DELETE_SELF_SUBSCRIPTION)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?;
        let customer = self
            .0
            .prepare(DELETE_CUSTOMER)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![anonymize, self_consumer, customer])
            .await
            .map_err(map_err)?;
        Ok(results.last().map(changed_rows).unwrap_or_default() == 1)
    }

    async fn claim_activation_resend(
        &self,
        account: &str,
        now: u64,
        not_since: u64,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(RECORD_ACTIVATION_SENT)
            .bind(&[
                JsValue::from(account),
                JsValue::from_f64(now as f64),
                JsValue::from_f64(not_since as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn activate_customer(
        &self,
        did: &str,
        terms_version: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        // Status and expiry in one batch, which D1 runs as a single
        // transaction: the subscriptions were written to lapse if the
        // link was never clicked, and it has been. Half of this would
        // leave an active customer whose spaces expire.
        let activate_customer = self
            .0
            .prepare(ACTIVATE_CUSTOMER)
            .bind(&[
                JsValue::from(did),
                JsValue::from_f64(now as f64),
                JsValue::from(terms_version),
            ])
            .map_err(map_err)?;
        let activate_subscriptions = self
            .0
            .prepare(ACTIVATE_SUBSCRIPTIONS)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![activate_customer, activate_subscriptions])
            .await
            .map_err(map_err)?;
        Ok(results
            .first()
            .is_some_and(|result| changed_rows(result) > 0))
    }
}

/// The number of rows an executed statement changed, zero when the
/// driver reported no metadata.
fn changed_rows(result: &worker::d1::D1Result) -> usize {
    result
        .meta()
        .ok()
        .flatten()
        .and_then(|meta| meta.changes)
        .unwrap_or_default()
}
