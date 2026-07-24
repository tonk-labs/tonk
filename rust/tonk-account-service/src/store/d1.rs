//! Cloudflare D1-backed twin of the [`Store`](crate::store::Store) trait,
//! for production use.
//!
//! D1 is SQLite, so this implementation issues the same query strings as
//! [`SqliteStore`](crate::store::sqlite::SqliteStore) — hoisted as shared
//! `const` items on [`crate::store`] — and deserializes rows through
//! serde structs instead of `rusqlite`'s row API. D1 surfaces numbers as
//! JS `f64`; the timestamps and row ids this crate deals with fit
//! losslessly.

use serde::Deserialize;
use worker::d1::D1Database;
use worker::wasm_bindgen::JsValue;

use crate::store::{
    Account, BUMP_ATTEMPTS, COMPLETE_LINK, CONSUME_LINK, CodeRow, DELETE_CODE, Device,
    DeviceStatus, INSERT_ACCOUNT, INSERT_DEVICE, INSERT_DEVICE_FOR_LINK,
    INSERT_DEVICE_FOR_NEW_ACCOUNT, INSERT_LINK, LinkRequest, NewDevice, SELECT_ACCOUNT_BY_ROOT,
    SELECT_CODE, SELECT_DEVICE_BY_DID, SELECT_DEVICES_BY_ACCOUNT, SELECT_LINK, Store, StoreError,
    UPDATE_ACCOUNT_ROOT, UPDATE_DEVICE_DELEGATION, UPDATE_DEVICE_REVOKE, UPSERT_CODE,
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

/// A pending code row as deserialized straight off a D1 query.
#[derive(Deserialize)]
struct CodeRowD1 {
    email: String,
    code_hash: String,
    created_at: f64,
    expires_at: f64,
    attempts: f64,
}

impl From<CodeRowD1> for CodeRow {
    fn from(row: CodeRowD1) -> Self {
        CodeRow {
            email: row.email,
            code_hash: row.code_hash,
            created_at: row.created_at as u64,
            expires_at: row.expires_at as u64,
            attempts: row.attempts as u32,
        }
    }
}

/// An account row as deserialized straight off a D1 query.
#[derive(Deserialize)]
struct AccountRowD1 {
    id: f64,
    email: String,
    root_did: String,
    credential_id: String,
    created_at: f64,
}

impl From<AccountRowD1> for Account {
    fn from(row: AccountRowD1) -> Self {
        Account {
            id: row.id as i64,
            email: row.email,
            root_did: row.root_did,
            credential_id: row.credential_id,
            created_at: row.created_at as u64,
        }
    }
}

/// A device row as deserialized straight off a D1 query, before the
/// status column is parsed.
#[derive(Deserialize)]
struct DeviceRowD1 {
    account_id: f64,
    device_did: String,
    delegation_cid: String,
    name: String,
    status: String,
    created_at: f64,
}

impl TryFrom<DeviceRowD1> for Device {
    type Error = StoreError;

    fn try_from(row: DeviceRowD1) -> Result<Self, StoreError> {
        Ok(Device {
            account_id: row.account_id as i64,
            device_did: row.device_did,
            delegation_cid: row.delegation_cid,
            name: row.name,
            status: DeviceStatus::parse(&row.status)?,
            created_at: row.created_at as u64,
        })
    }
}

#[derive(Deserialize)]
struct LinkRequestD1 {
    token_hash: String,
    device_did: String,
    device_name: String,
    delegation_hex: Option<String>,
    created_at: f64,
    expires_at: f64,
    consumed_at: Option<f64>,
}

impl From<LinkRequestD1> for LinkRequest {
    fn from(row: LinkRequestD1) -> Self {
        LinkRequest {
            token_hash: row.token_hash,
            device_did: row.device_did,
            device_name: row.device_name,
            delegation_hex: row.delegation_hex,
            created_at: row.created_at as u64,
            expires_at: row.expires_at as u64,
            consumed_at: row.consumed_at.map(|value| value as u64),
        }
    }
}

#[derive(Deserialize)]
struct ConsumedLinkD1 {
    delegation_hex: String,
}

impl Store for D1Store {
    async fn code(&self, email: &str) -> Result<Option<CodeRow>, StoreError> {
        let row: Option<CodeRowD1> = self
            .0
            .prepare(SELECT_CODE)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(CodeRow::from))
    }

    async fn put_code(&self, row: &CodeRow) -> Result<(), StoreError> {
        self.0
            .prepare(UPSERT_CODE)
            .bind(&[
                JsValue::from(row.email.as_str()),
                JsValue::from(row.code_hash.as_str()),
                JsValue::from_f64(row.created_at as f64),
                JsValue::from_f64(row.expires_at as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn bump_attempts(&self, email: &str) -> Result<(), StoreError> {
        self.0
            .prepare(BUMP_ATTEMPTS)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn delete_code(&self, email: &str) -> Result<(), StoreError> {
        self.0
            .prepare(DELETE_CODE)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn create_account(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        created_at: u64,
    ) -> Result<i64, StoreError> {
        let result = self
            .0
            .prepare(INSERT_ACCOUNT)
            .bind(&[
                JsValue::from(email),
                JsValue::from(root_did),
                JsValue::from(credential_id),
                JsValue::from_f64(created_at as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        result
            .meta()
            .map_err(map_err)?
            .and_then(|meta| meta.last_row_id)
            .ok_or_else(|| StoreError::Internal("insert did not return a row id".to_string()))
    }

    async fn create_account_with_device(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        device: &NewDevice,
        created_at: u64,
    ) -> Result<i64, StoreError> {
        // D1 batches run as a single transaction: either every statement
        // commits or none does. The device insert reaches back for the
        // account id it needs via SQLite's `last_insert_rowid()` rather
        // than a value threaded in from Rust, since the account id isn't
        // known until the first statement in this same batch executes.
        let insert_account = self
            .0
            .prepare(INSERT_ACCOUNT)
            .bind(&[
                JsValue::from(email),
                JsValue::from(root_did),
                JsValue::from(credential_id),
                JsValue::from_f64(created_at as f64),
            ])
            .map_err(map_err)?;
        let insert_device = self
            .0
            .prepare(INSERT_DEVICE_FOR_NEW_ACCOUNT)
            .bind(&[
                JsValue::from(device.device_did.as_str()),
                JsValue::from(device.delegation_cid.as_str()),
                JsValue::from(device.name.as_str()),
                JsValue::from_f64(created_at as f64),
            ])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![insert_account, insert_device])
            .await
            .map_err(map_err)?;
        results
            .first()
            .and_then(|result| result.meta().ok().flatten())
            .and_then(|meta| meta.last_row_id)
            .ok_or_else(|| StoreError::Internal("insert did not return a row id".to_string()))
    }

    async fn account_by_root(&self, root_did: &str) -> Result<Option<Account>, StoreError> {
        let row: Option<AccountRowD1> = self
            .0
            .prepare(SELECT_ACCOUNT_BY_ROOT)
            .bind(&[JsValue::from(root_did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(Account::from))
    }

    async fn insert_device(&self, device: &Device) -> Result<(), StoreError> {
        self.0
            .prepare(INSERT_DEVICE)
            .bind(&[
                JsValue::from_f64(device.account_id as f64),
                JsValue::from(device.device_did.as_str()),
                JsValue::from(device.delegation_cid.as_str()),
                JsValue::from(device.name.as_str()),
                JsValue::from(device.status.as_str()),
                JsValue::from_f64(device.created_at as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn devices(&self, account_id: i64) -> Result<Vec<Device>, StoreError> {
        let result = self
            .0
            .prepare(SELECT_DEVICES_BY_ACCOUNT)
            .bind(&[JsValue::from_f64(account_id as f64)])
            .map_err(map_err)?
            .all()
            .await
            .map_err(map_err)?;
        let rows: Vec<DeviceRowD1> = result.results().map_err(map_err)?;
        rows.into_iter().map(Device::try_from).collect()
    }

    async fn device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError> {
        let row: Option<DeviceRowD1> = self
            .0
            .prepare(SELECT_DEVICE_BY_DID)
            .bind(&[JsValue::from(device_did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Device::try_from).transpose()
    }

    async fn revoke_device(&self, account_id: i64, device_did: &str) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(UPDATE_DEVICE_REVOKE)
            .bind(&[
                JsValue::from_f64(account_id as f64),
                JsValue::from(device_did),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        let changes = result
            .meta()
            .map_err(map_err)?
            .and_then(|meta| meta.changes)
            .unwrap_or(0);
        Ok(changes > 0)
    }

    async fn rotate_root(
        &self,
        account_id: i64,
        new_root_did: &str,
        new_credential_id: &str,
    ) -> Result<(), StoreError> {
        self.0
            .prepare(UPDATE_ACCOUNT_ROOT)
            .bind(&[
                JsValue::from_f64(account_id as f64),
                JsValue::from(new_root_did),
                JsValue::from(new_credential_id),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn update_device_delegation(
        &self,
        account_id: i64,
        device_did: &str,
        delegation_cid: &str,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(UPDATE_DEVICE_DELEGATION)
            .bind(&[
                JsValue::from_f64(account_id as f64),
                JsValue::from(device_did),
                JsValue::from(delegation_cid),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        let changes = result
            .meta()
            .map_err(map_err)?
            .and_then(|meta| meta.changes)
            .unwrap_or(0);
        Ok(changes > 0)
    }

    async fn put_link(&self, link: &LinkRequest) -> Result<(), StoreError> {
        self.0
            .prepare(INSERT_LINK)
            .bind(&[
                JsValue::from(link.token_hash.as_str()),
                JsValue::from(link.device_did.as_str()),
                JsValue::from(link.device_name.as_str()),
                JsValue::from_f64(link.created_at as f64),
                JsValue::from_f64(link.expires_at as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn link(&self, token_hash: &str) -> Result<Option<LinkRequest>, StoreError> {
        let row: Option<LinkRequestD1> = self
            .0
            .prepare(SELECT_LINK)
            .bind(&[JsValue::from(token_hash)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(LinkRequest::from))
    }

    async fn complete_link(
        &self,
        token_hash: &str,
        device: &Device,
        delegation_hex: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        let insert = self
            .0
            .prepare(INSERT_DEVICE_FOR_LINK)
            .bind(&[
                JsValue::from_f64(device.account_id as f64),
                JsValue::from(device.device_did.as_str()),
                JsValue::from(device.delegation_cid.as_str()),
                JsValue::from(device.name.as_str()),
                JsValue::from(device.status.as_str()),
                JsValue::from_f64(device.created_at as f64),
                JsValue::from(token_hash),
                JsValue::from_f64(now as f64),
            ])
            .map_err(map_err)?;
        let complete = self
            .0
            .prepare(COMPLETE_LINK)
            .bind(&[
                JsValue::from(delegation_hex),
                JsValue::from(token_hash),
                JsValue::from_f64(now as f64),
            ])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![insert, complete])
            .await
            .map_err(map_err)?;
        let changes = |index: usize| {
            results
                .get(index)
                .and_then(|result| result.meta().ok().flatten())
                .and_then(|meta| meta.changes)
                .unwrap_or(0)
        };
        Ok(changes(0) == 1 && changes(1) == 1)
    }

    async fn consume_link(&self, token_hash: &str, now: u64) -> Result<Option<String>, StoreError> {
        let row: Option<ConsumedLinkD1> = self
            .0
            .prepare(CONSUME_LINK)
            .bind(&[
                JsValue::from_f64(now as f64),
                JsValue::from(token_hash),
                JsValue::from_f64(now as f64),
            ])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(|row| row.delegation_hex))
    }
}
