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
    Account, ActivateOutcome, BUMP_ATTEMPTS, COMPLETE_LINK, CONSUME_LINK, CodeRow, DELETE_ACCOUNT,
    DELETE_ACCOUNT_DEVICES, DELETE_ACCOUNT_LINKS, DELETE_CODE, DetachStoreOutcome, Device,
    DeviceStatus, ESTABLISH_REPOSITORY_DESCRIPTOR, INSERT_ACCOUNT, INSERT_ACCOUNT_WITH_DESCRIPTOR,
    INSERT_DEVICE, INSERT_DEVICE_FOR_NEW_ACCOUNT, INSERT_LINK, LinkCompletion, LinkRequest,
    NewAccount, NewDevice, SELECT_ACCOUNT_BY_EMAIL, SELECT_ACCOUNT_BY_ROOT,
    SELECT_ACTIVE_DEVICE_BY_DID, SELECT_ATTACHMENT, SELECT_CODE, SELECT_DEVICE_FOR_ACCOUNT,
    SELECT_DEVICES_BY_ACCOUNT, SELECT_LINK, SELECT_LINK_BY_ATTACHMENT,
    SELECT_REPOSITORY_DESCRIPTOR, Store, StoreError, UPDATE_DEVICE_REVOKE,
    UPDATE_DEVICE_REVOKE_BY_CID, UPSERT_CODE,
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
    repository_descriptor: Option<Vec<u8>>,
    passkey_created_at: Option<f64>,
    passkey_created_on: Option<String>,
    created_at: f64,
}

impl From<AccountRowD1> for Account {
    fn from(row: AccountRowD1) -> Self {
        Account {
            id: row.id as i64,
            email: row.email,
            root_did: row.root_did,
            credential_id: row.credential_id,
            repository_descriptor: row.repository_descriptor,
            passkey_created_at: row.passkey_created_at.map(|value| value as u64),
            passkey_created_on: row.passkey_created_on,
            created_at: row.created_at as u64,
        }
    }
}

/// A device row as deserialized straight off a D1 query, before the
/// status column is parsed.
#[derive(Deserialize)]
struct DeviceRowD1 {
    id: f64,
    account_id: f64,
    device_did: String,
    attachment_id: String,
    delegation_cid: String,
    delegation_hex: Option<String>,
    name: String,
    status: String,
    created_at: f64,
}

impl TryFrom<DeviceRowD1> for Device {
    type Error = StoreError;

    fn try_from(row: DeviceRowD1) -> Result<Self, StoreError> {
        Ok(Device {
            id: row.id as i64,
            account_id: row.account_id as i64,
            device_did: row.device_did,
            attachment_id: row.attachment_id,
            delegation_cid: row.delegation_cid,
            delegation_hex: row.delegation_hex.unwrap_or_default(),
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
    account_id: Option<f64>,
    attachment_id: Option<String>,
    delegation_cid: Option<String>,
    delegation_hex: Option<String>,
    descriptor_hex: Option<String>,
    created_at: f64,
    expires_at: f64,
    completed_at: Option<f64>,
    consumed_at: Option<f64>,
    activated_at: Option<f64>,
    cancelled_at: Option<f64>,
}

impl From<LinkRequestD1> for LinkRequest {
    fn from(row: LinkRequestD1) -> Self {
        LinkRequest {
            token_hash: row.token_hash,
            device_did: row.device_did,
            device_name: row.device_name,
            account_id: row.account_id.map(|value| value as i64),
            attachment_id: row.attachment_id,
            delegation_cid: row.delegation_cid,
            delegation_hex: row.delegation_hex,
            descriptor_hex: row.descriptor_hex,
            created_at: row.created_at as u64,
            expires_at: row.expires_at as u64,
            completed_at: row.completed_at.map(|value| value as u64),
            consumed_at: row.consumed_at.map(|value| value as u64),
            activated_at: row.activated_at.map(|value| value as u64),
            cancelled_at: row.cancelled_at.map(|value| value as u64),
        }
    }
}

#[derive(Deserialize)]
struct DescriptorRowD1 {
    repository_descriptor: Vec<u8>,
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
        account: &NewAccount<'_>,
        device: &NewDevice,
    ) -> Result<i64, StoreError> {
        // D1 batches run as a single transaction: either every statement
        // commits or none does. The device insert reaches back for the
        // account id it needs via SQLite's `last_insert_rowid()` rather
        // than a value threaded in from Rust, since the account id isn't
        // known until the first statement in this same batch executes.
        let descriptor = js_sys::Uint8Array::from(account.repository_descriptor);
        let insert_account = self
            .0
            .prepare(INSERT_ACCOUNT_WITH_DESCRIPTOR)
            .bind(&[
                JsValue::from(account.email),
                JsValue::from(account.root_did),
                JsValue::from(account.credential_id),
                descriptor.into(),
                account
                    .passkey
                    .map(|metadata| JsValue::from_f64(metadata.created_at as f64))
                    .unwrap_or(JsValue::NULL),
                account
                    .passkey
                    .map(|metadata| JsValue::from(metadata.created_on.as_str()))
                    .unwrap_or(JsValue::NULL),
                JsValue::from_f64(account.created_at as f64),
            ])
            .map_err(map_err)?;
        let insert_device = self
            .0
            .prepare(INSERT_DEVICE_FOR_NEW_ACCOUNT)
            .bind(&[
                JsValue::from(device.device_did.as_str()),
                JsValue::from(device.attachment_id.as_str()),
                JsValue::from(device.delegation_cid.as_str()),
                JsValue::from(device.delegation_hex.as_str()),
                JsValue::from(device.name.as_str()),
                JsValue::from_f64(account.created_at as f64),
            ])
            .map_err(map_err)?;
        let consume_code = self
            .0
            .prepare(DELETE_CODE)
            .bind(&[JsValue::from(account.email)])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![insert_account, insert_device, consume_code])
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

    async fn account_by_email(&self, email: &str) -> Result<Option<Account>, StoreError> {
        let row: Option<AccountRowD1> = self
            .0
            .prepare(SELECT_ACCOUNT_BY_EMAIL)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(Account::from))
    }

    async fn delete_account(&self, account_id: i64, email: &str) -> Result<bool, StoreError> {
        let id = JsValue::from_f64(account_id as f64);
        let links = self
            .0
            .prepare(DELETE_ACCOUNT_LINKS)
            .bind(&[id.clone()])
            .map_err(map_err)?;
        let devices = self
            .0
            .prepare(DELETE_ACCOUNT_DEVICES)
            .bind(&[id.clone()])
            .map_err(map_err)?;
        let code = self
            .0
            .prepare(DELETE_CODE)
            .bind(&[JsValue::from(email)])
            .map_err(map_err)?;
        let account = self
            .0
            .prepare(DELETE_ACCOUNT)
            .bind(&[id, JsValue::from(email)])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![links, devices, code, account])
            .await
            .map_err(map_err)?;
        Ok(results
            .last()
            .and_then(|result| result.meta().ok().flatten())
            .and_then(|meta| meta.changes)
            .unwrap_or(0)
            == 1)
    }

    async fn establish_repository_descriptor(
        &self,
        account_id: i64,
        candidate: &[u8],
    ) -> Result<(Vec<u8>, bool), StoreError> {
        let candidate = js_sys::Uint8Array::from(candidate);
        let establish = self
            .0
            .prepare(ESTABLISH_REPOSITORY_DESCRIPTOR)
            .bind(&[JsValue::from_f64(account_id as f64), candidate.into()])
            .map_err(map_err)?;
        let select = self
            .0
            .prepare(SELECT_REPOSITORY_DESCRIPTOR)
            .bind(&[JsValue::from_f64(account_id as f64)])
            .map_err(map_err)?;
        let results = self
            .0
            .batch(vec![establish, select])
            .await
            .map_err(map_err)?;
        let created = results
            .first()
            .ok_or_else(|| StoreError::Internal("descriptor update returned no result".into()))?
            .results::<DescriptorRowD1>()
            .map_err(map_err)?
            .len()
            == 1;
        let winner = results
            .get(1)
            .ok_or_else(|| StoreError::Internal("descriptor lookup returned no result".into()))?
            .results::<DescriptorRowD1>()
            .map_err(map_err)?
            .into_iter()
            .next()
            .ok_or_else(|| StoreError::Internal("account not found".into()))?
            .repository_descriptor;
        Ok((winner, created))
    }

    async fn insert_device(&self, device: &Device) -> Result<(), StoreError> {
        self.0
            .prepare(INSERT_DEVICE)
            .bind(&[
                JsValue::from_f64(device.account_id as f64),
                JsValue::from(device.device_did.as_str()),
                JsValue::from(device.attachment_id.as_str()),
                JsValue::from(device.delegation_cid.as_str()),
                JsValue::from(device.delegation_hex.as_str()),
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

    async fn active_device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError> {
        let row: Option<DeviceRowD1> = self
            .0
            .prepare(SELECT_ACTIVE_DEVICE_BY_DID)
            .bind(&[JsValue::from(device_did)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Device::try_from).transpose()
    }

    async fn device_for_account(
        &self,
        account_id: i64,
        device_did: &str,
    ) -> Result<Option<Device>, StoreError> {
        let row: Option<DeviceRowD1> = self
            .0
            .prepare(SELECT_DEVICE_FOR_ACCOUNT)
            .bind(&[
                JsValue::from_f64(account_id as f64),
                JsValue::from(device_did),
            ])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        row.map(Device::try_from).transpose()
    }

    async fn attachment(&self, attachment_id: &str) -> Result<Option<Device>, StoreError> {
        let row: Option<DeviceRowD1> = self
            .0
            .prepare(SELECT_ATTACHMENT)
            .bind(&[JsValue::from(attachment_id)])
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

    async fn revoke_device_by_cid(
        &self,
        account_id: i64,
        delegation_cid: &str,
    ) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(UPDATE_DEVICE_REVOKE_BY_CID)
            .bind(&[
                JsValue::from_f64(account_id as f64),
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

    async fn complete_link(&self, completion: &LinkCompletion<'_>) -> Result<bool, StoreError> {
        let result = self
            .0
            .prepare(COMPLETE_LINK)
            .bind(&[
                JsValue::from_f64(completion.account_id as f64),
                JsValue::from(completion.attachment_id),
                JsValue::from(completion.delegation_cid),
                JsValue::from(completion.delegation_hex),
                JsValue::from(completion.descriptor_hex),
                JsValue::from_f64(completion.now as f64),
                JsValue::from_f64((completion.now + 24 * 60 * 60) as f64),
                JsValue::from(completion.token_hash),
                JsValue::from_f64(completion.now as f64),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(result
            .meta()
            .map_err(map_err)?
            .and_then(|m| m.changes)
            .unwrap_or(0)
            == 1)
    }

    async fn consume_link(
        &self,
        token_hash: &str,
        now: u64,
    ) -> Result<Option<LinkRequest>, StoreError> {
        let row: Option<LinkRequestD1> = self
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
        Ok(row.map(LinkRequest::from))
    }

    async fn completed_link_by_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<Option<LinkRequest>, StoreError> {
        let row: Option<LinkRequestD1> = self
            .0
            .prepare(SELECT_LINK_BY_ATTACHMENT)
            .bind(&[JsValue::from(attachment_id)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(row.map(LinkRequest::from))
    }

    async fn activate_completed_link(
        &self,
        token_hash: &str,
        attachment_id: &str,
        now: u64,
    ) -> Result<ActivateOutcome, StoreError> {
        if let Some(device) = self.attachment(attachment_id).await? {
            return Ok(match device.status {
                DeviceStatus::Active => ActivateOutcome::Active(device),
                DeviceStatus::Detached => ActivateOutcome::Cancelled,
                DeviceStatus::Revoked => ActivateOutcome::RevokedDelegation,
            });
        }
        let Some(link) = self.link(token_hash).await? else {
            return Ok(ActivateOutcome::Unknown);
        };
        if link.attachment_id.as_deref() != Some(attachment_id) {
            return Ok(ActivateOutcome::Unknown);
        }
        if link.cancelled_at.is_some() {
            return Ok(ActivateOutcome::Cancelled);
        }
        // A still-active earlier attachment of the same device does not
        // block a freshly completed handoff: the ceremony re-proved
        // possession of the device key, so the new generation supersedes
        // the old one (guarded on the same not-revoked condition as the
        // insert, so a revoked delegation cannot detach anything).
        let supersede_sql = "UPDATE devices SET status = 'detached' WHERE status = 'active' AND device_did = (SELECT device_did FROM link_requests l WHERE token_hash = ?1 AND attachment_id = ?2 AND completed_at IS NOT NULL AND cancelled_at IS NULL AND NOT EXISTS (SELECT 1 FROM devices d WHERE d.delegation_cid = l.delegation_cid AND d.status = 'revoked'))";
        let supersede = self
            .0
            .prepare(supersede_sql)
            .bind(&[JsValue::from(token_hash), JsValue::from(attachment_id)])
            .map_err(map_err)?;
        let insert_sql = "INSERT INTO devices (account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at) SELECT account_id, device_did, attachment_id, delegation_cid, delegation_hex, device_name, 'active', ?1 FROM link_requests l WHERE token_hash = ?2 AND attachment_id = ?3 AND completed_at IS NOT NULL AND cancelled_at IS NULL AND NOT EXISTS (SELECT 1 FROM devices d WHERE d.delegation_cid = l.delegation_cid AND d.status = 'revoked')";
        let insert = self
            .0
            .prepare(insert_sql)
            .bind(&[
                JsValue::from_f64(now as f64),
                JsValue::from(token_hash),
                JsValue::from(attachment_id),
            ])
            .map_err(map_err)?;
        let mark = self
            .0
            .prepare("UPDATE link_requests SET activated_at = COALESCE(activated_at, ?1) WHERE token_hash = ?2 AND attachment_id = ?3 AND EXISTS (SELECT 1 FROM devices WHERE attachment_id = ?3)")
            .bind(&[
                JsValue::from_f64(now as f64),
                JsValue::from(token_hash),
                JsValue::from(attachment_id),
            ])
            .map_err(map_err)?;
        self.0
            .batch(vec![supersede, insert, mark])
            .await
            .map_err(map_err)?;
        if let Some(device) = self.attachment(attachment_id).await? {
            return Ok(match device.status {
                DeviceStatus::Active => ActivateOutcome::Active(device),
                DeviceStatus::Detached => ActivateOutcome::Cancelled,
                DeviceStatus::Revoked => ActivateOutcome::RevokedDelegation,
            });
        }
        let revoked: Option<DeviceRowD1> = self
            .0
            .prepare("SELECT id, account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at FROM devices WHERE delegation_cid = ?1 AND status = 'revoked'")
            .bind(&[JsValue::from(link.delegation_cid.as_deref().unwrap_or_default())])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        Ok(if revoked.is_some() {
            ActivateOutcome::RevokedDelegation
        } else {
            ActivateOutcome::Unknown
        })
    }

    async fn detach_attachment(
        &self,
        attachment_id: &str,
        now: u64,
    ) -> Result<DetachStoreOutcome, StoreError> {
        if let Some(device) = self.attachment(attachment_id).await? {
            return match device.status {
                DeviceStatus::Detached => Ok(DetachStoreOutcome::AlreadyDetached),
                DeviceStatus::Revoked => Ok(DetachStoreOutcome::Revoked),
                DeviceStatus::Active => {
                    self.0
                        .prepare("UPDATE devices SET status = 'detached' WHERE attachment_id = ?1 AND status = 'active'")
                        .bind(&[JsValue::from(attachment_id)])
                        .map_err(map_err)?
                        .run()
                        .await
                        .map_err(map_err)?;
                    Ok(DetachStoreOutcome::Detached)
                }
            };
        }
        let result = self
            .0
            .prepare("UPDATE link_requests SET cancelled_at = COALESCE(cancelled_at, ?1) WHERE attachment_id = ?2 AND completed_at IS NOT NULL AND activated_at IS NULL")
            .bind(&[
                JsValue::from_f64(now as f64),
                JsValue::from(attachment_id),
            ])
            .map_err(map_err)?
            .run()
            .await
            .map_err(map_err)?;
        Ok(
            if result
                .meta()
                .map_err(map_err)?
                .and_then(|m| m.changes)
                .unwrap_or(0)
                == 1
            {
                DetachStoreOutcome::CancelledPendingActivation
            } else {
                DetachStoreOutcome::UnknownAttachment
            },
        )
    }
}
