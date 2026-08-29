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
    ACTIVATE_CUSTOMER, ADD_CONSUMER, ANONYMIZE_DELETED_CONSUMERS, Consumer, ConsumerDeletionState,
    ConsumerKind, Customer, DELETE_CUSTOMER, DELETE_SELF_CONSUMER, FINISH_CONSUMER_DELETION,
    INSERT_CUSTOMER, INSERT_SELF_CONSUMER, MARK_CONSUMER_DELETING, MARK_SELF_CONSUMER_DELETING,
    RESERVE_CONSUMER, SELECT_CONSUMER, SELECT_CONSUMERS_BY_OWNER, SELECT_CUSTOMER,
    SELECT_CUSTOMER_BY_EMAIL, SELECT_SERVABILITY, Servability, Store, StoreError,
    UPDATE_REGISTERED_EMAIL, parse_status,
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
    did: String,
    email: String,
    status: String,
    plan: String,
    verified: f64,
    terms_version: Option<String>,
}

impl TryFrom<CustomerRowD1> for Customer {
    type Error = StoreError;

    fn try_from(row: CustomerRowD1) -> Result<Self, StoreError> {
        Ok(Customer {
            did: row.did,
            email: row.email,
            status: parse_status(&row.status)?,
            plan: row.plan,
            verified: row.verified as u64,
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
    consumer_provider: Option<String>,
    consumer_reserved_until: Option<f64>,
    provider_status: Option<String>,
}

impl TryFrom<ServabilityRowD1> for Servability {
    type Error = StoreError;

    fn try_from(row: ServabilityRowD1) -> Result<Self, Self::Error> {
        Ok(Servability {
            own: row.own_status.as_deref().map(parse_status).transpose()?,
            consumer: row.consumer_did.is_some(),
            provided: row.consumer_provider.is_some(),
            reserved_until: row.consumer_reserved_until.map(|value| value as u64),
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
struct ConsumerRowD1 {
    did: String,
    provider: Option<String>,
    owner: Option<String>,
    registered: f64,
    kind: String,
    deletion_state: String,
    deleted_at: Option<f64>,
    reserved_until: Option<f64>,
}

impl TryFrom<ConsumerRowD1> for Consumer {
    type Error = StoreError;

    fn try_from(row: ConsumerRowD1) -> Result<Self, Self::Error> {
        Ok(Consumer {
            did: row.did,
            provider: row.provider,
            owner: row.owner,
            registered: row.registered as u64,
            kind: ConsumerKind::parse(&row.kind)?,
            deletion_state: ConsumerDeletionState::parse(&row.deletion_state)?,
            deleted_at: row.deleted_at.map(|value| value as u64),
            reserved_until: row.reserved_until.map(|value| value as u64),
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

    async fn consumer(&self, did: &str) -> Result<Option<Consumer>, StoreError> {
        let row: Option<ConsumerRowD1> = self
            .0
            .prepare(SELECT_CONSUMER)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Consumer::try_from).transpose()
    }

    async fn consumers_by_owner(&self, owner: &str) -> Result<Vec<Consumer>, StoreError> {
        let result = self
            .0
            .prepare(SELECT_CONSUMERS_BY_OWNER)
            .bind(&[JsValue::from(owner)])
            .map_err(map_err)?
            .all()
            .await
            .map_err(map_err)?;
        result
            .results::<ConsumerRowD1>()
            .map_err(map_err)?
            .into_iter()
            .map(Consumer::try_from)
            .collect()
    }

    async fn enroll_customer(
        &self,
        did: &str,
        email: &str,
        access: &[u8],
        plan: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        // D1 batches run as a single transaction: either every statement
        // commits or none does.
        let access = js_sys::Uint8Array::from(access);
        let insert_customer = self
            .0
            .prepare(INSERT_CUSTOMER)
            .bind(&[
                JsValue::from(did),
                JsValue::from(email),
                JsValue::from(plan),
                JsValue::from_f64(now as f64),
                access.into(),
            ])
            .map_err(map_err)?;
        let insert_consumer = self
            .0
            .prepare(INSERT_SELF_CONSUMER)
            .bind(&[JsValue::from(did), JsValue::from_f64(now as f64)])
            .map_err(map_err)?;
        self.0
            .batch(vec![insert_customer, insert_consumer])
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

    async fn add_consumer(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: ConsumerKind,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(ADD_CONSUMER)
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

    async fn reserve_consumer(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: ConsumerKind,
        reserved_until: u64,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(RESERVE_CONSUMER)
            .bind(&[
                JsValue::from(did),
                JsValue::from(provider),
                JsValue::from_f64(now as f64),
                JsValue::from(kind.as_str()),
                JsValue::from_f64(reserved_until as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn mark_consumer_deleting(&self, did: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(MARK_CONSUMER_DELETING)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn finish_consumer_deletion(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(FINISH_CONSUMER_DELETION)
            .bind(&[JsValue::from(did), JsValue::from_f64(now as f64)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn mark_self_consumer_deleting(&self, did: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(MARK_SELF_CONSUMER_DELETING)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
    }

    async fn delete_customer(&self, did: &str) -> Result<bool, StoreError> {
        let anonymize = self
            .0
            .prepare(ANONYMIZE_DELETED_CONSUMERS)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?;
        let self_consumer = self
            .0
            .prepare(DELETE_SELF_CONSUMER)
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

    async fn activate_customer(
        &self,
        did: &str,
        terms_version: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(ACTIVATE_CUSTOMER)
            .bind(&[
                JsValue::from(did),
                JsValue::from_f64(now as f64),
                JsValue::from(terms_version),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(changed_rows(&result) > 0)
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
