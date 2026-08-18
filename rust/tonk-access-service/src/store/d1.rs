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
    ACTIVATE_CUSTOMER, Consumer, Customer, INSERT_CUSTOMER, INSERT_SELF_CONSUMER, SELECT_CONSUMER,
    SELECT_CUSTOMER, Store, StoreError, UPDATE_REGISTERED_EMAIL, parse_status,
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

/// A consumer row as deserialized straight off a D1 query.
#[derive(Deserialize)]
struct ConsumerRowD1 {
    did: String,
    provider: Option<String>,
    registered: f64,
}

impl From<ConsumerRowD1> for Consumer {
    fn from(row: ConsumerRowD1) -> Self {
        Consumer {
            did: row.did,
            provider: row.provider,
            registered: row.registered as u64,
        }
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

    async fn consumer(&self, did: &str) -> Result<Option<Consumer>, StoreError> {
        let row: Option<ConsumerRowD1> = self
            .0
            .prepare(SELECT_CONSUMER)
            .bind(&[JsValue::from(did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(Consumer::from))
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
