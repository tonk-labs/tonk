//! Native `rusqlite` twin of the [`Store`](crate::store::Store) trait,
//! used for tests and local development. Backed by the same schema
//! applied to Cloudflare D1 in production.

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use super::{
    Account, ActivateOutcome, BUMP_ATTEMPTS, COMPLETE_LINK, CONSUME_LINK, CodeRow, DELETE_ACCOUNT,
    DELETE_ACCOUNT_DEVICES, DELETE_ACCOUNT_LINKS, DELETE_CODE, DetachStoreOutcome, Device,
    DeviceStatus, ESTABLISH_REPOSITORY_DESCRIPTOR, INSERT_ACCOUNT, INSERT_ACCOUNT_WITH_DESCRIPTOR,
    INSERT_DEVICE, INSERT_LINK, LinkCompletion, LinkRequest, NewAccount, NewDevice,
    SELECT_ACCOUNT_BY_EMAIL, SELECT_ACCOUNT_BY_ROOT, SELECT_ACTIVE_DEVICE_BY_DID,
    SELECT_ATTACHMENT, SELECT_CODE, SELECT_DEVICE_FOR_ACCOUNT, SELECT_DEVICES_BY_ACCOUNT,
    SELECT_LINK, SELECT_LINK_BY_ATTACHMENT, SELECT_REPOSITORY_DESCRIPTOR, Store, StoreError,
    UPDATE_DEVICE_REVOKE, UPDATE_DEVICE_REVOKE_BY_CID, UPSERT_CODE,
};

/// Native `rusqlite`-backed [`Store`], for tests and local development.
///
/// Wraps a single connection behind a mutex: sqlite serializes writes
/// internally, and the volume this crate deals with does not warrant a
/// connection pool.
pub struct SqliteStore(Mutex<Connection>);

impl SqliteStore {
    /// Open a fresh in-memory database and apply every migration under
    /// `migrations/` — the same schema applied to Cloudflare D1 in
    /// production.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        // The bundled libsqlite3-sys build already defaults foreign_keys
        // on, but this pins the behavior explicitly so a future build-flag
        // change (e.g. switching to the system libsqlite3) can't silently
        // drop enforcement.
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0001_init.sql"))
            .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0002_link_requests.sql"))
            .map_err(map_err)?;
        conn.execute_batch(include_str!(
            "../../migrations/0003_device_delegation_path.sql"
        ))
        .map_err(map_err)?;
        conn.execute_batch(include_str!(
            "../../migrations/0004_account_repository_descriptor.sql"
        ))
        .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0005_normalize_devices.sql"))
            .map_err(map_err)?;
        conn.execute_batch(include_str!(
            "../../migrations/0006_device_attachment_lifecycle.sql"
        ))
        .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0007_passkey_metadata.sql"))
            .map_err(map_err)?;
        Ok(Self(Mutex::new(conn)))
    }
}

/// Map a `rusqlite` error onto [`StoreError`]. Unique/primary-key
/// constraint violations become `Conflict`; everything else is
/// `Internal`.
fn map_err(err: rusqlite::Error) -> StoreError {
    if let rusqlite::Error::SqliteFailure(sqlite_err, _) = &err
        && sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation
        && matches!(
            sqlite_err.extended_code,
            rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
        )
    {
        return StoreError::Conflict(err.to_string());
    }
    StoreError::Internal(err.to_string())
}

/// A device row as read straight off a `devices` query, before the
/// status column is parsed.
type DeviceRow = (
    i64,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
);

fn device_from_row(row: DeviceRow) -> Result<Device, StoreError> {
    let (
        id,
        account_id,
        device_did,
        attachment_id,
        delegation_cid,
        delegation_hex,
        name,
        status,
        created_at,
    ) = row;
    Ok(Device {
        id,
        account_id,
        device_did,
        attachment_id,
        delegation_cid,
        delegation_hex: delegation_hex.unwrap_or_default(),
        name,
        status: DeviceStatus::parse(&status)?,
        created_at: created_at as u64,
    })
}

fn device_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LinkRequest> {
    Ok(LinkRequest {
        token_hash: row.get(0)?,
        device_did: row.get(1)?,
        device_name: row.get(2)?,
        account_id: row.get(3)?,
        attachment_id: row.get(4)?,
        delegation_cid: row.get(5)?,
        delegation_hex: row.get(6)?,
        descriptor_hex: row.get(7)?,
        created_at: row.get::<_, i64>(8)? as u64,
        expires_at: row.get::<_, i64>(9)? as u64,
        completed_at: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        consumed_at: row.get::<_, Option<i64>>(11)?.map(|v| v as u64),
        activated_at: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
        cancelled_at: row.get::<_, Option<i64>>(13)?.map(|v| v as u64),
    })
}

impl Store for SqliteStore {
    async fn code(&self, email: &str) -> Result<Option<CodeRow>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_CODE, params![email], |row| {
            Ok(CodeRow {
                email: row.get(0)?,
                code_hash: row.get(1)?,
                created_at: row.get::<_, i64>(2)? as u64,
                expires_at: row.get::<_, i64>(3)? as u64,
                attempts: row.get::<_, i64>(4)? as u32,
            })
        })
        .optional()
        .map_err(map_err)
    }

    async fn put_code(&self, row: &CodeRow) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            UPSERT_CODE,
            params![
                row.email,
                row.code_hash,
                row.created_at as i64,
                row.expires_at as i64
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn bump_attempts(&self, email: &str) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(BUMP_ATTEMPTS, params![email])
            .map_err(map_err)?;
        Ok(())
    }

    async fn delete_code(&self, email: &str) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(DELETE_CODE, params![email]).map_err(map_err)?;
        Ok(())
    }

    async fn create_account(
        &self,
        email: &str,
        root_did: &str,
        credential_id: &str,
        created_at: u64,
    ) -> Result<i64, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            INSERT_ACCOUNT,
            params![email, root_did, credential_id, created_at as i64],
        )
        .map_err(map_err)?;
        Ok(conn.last_insert_rowid())
    }

    async fn create_account_with_device(
        &self,
        account: &NewAccount<'_>,
        device: &NewDevice,
    ) -> Result<i64, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            INSERT_ACCOUNT_WITH_DESCRIPTOR,
            params![
                account.email,
                account.root_did,
                account.credential_id,
                account.repository_descriptor,
                account.passkey.map(|metadata| metadata.created_at as i64),
                account.passkey.map(|metadata| metadata.created_on.as_str()),
                account.created_at as i64
            ],
        )
        .map_err(map_err)?;
        let account_id = tx.last_insert_rowid();
        tx.execute(
            INSERT_DEVICE,
            params![
                account_id,
                device.device_did,
                device.attachment_id,
                device.delegation_cid,
                device.delegation_hex,
                device.name,
                DeviceStatus::Active.as_str(),
                account.created_at as i64,
            ],
        )
        .map_err(map_err)?;
        tx.execute(DELETE_CODE, params![account.email])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(account_id)
    }

    async fn account_by_root(&self, root_did: &str) -> Result<Option<Account>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_ACCOUNT_BY_ROOT, params![root_did], |row| {
            Ok(Account {
                id: row.get(0)?,
                email: row.get(1)?,
                root_did: row.get(2)?,
                credential_id: row.get(3)?,
                repository_descriptor: row.get(4)?,
                passkey_created_at: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
                passkey_created_on: row.get(6)?,
                created_at: row.get::<_, i64>(7)? as u64,
            })
        })
        .optional()
        .map_err(map_err)
    }

    async fn account_by_email(&self, email: &str) -> Result<Option<Account>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_ACCOUNT_BY_EMAIL, params![email], |row| {
            Ok(Account {
                id: row.get(0)?,
                email: row.get(1)?,
                root_did: row.get(2)?,
                credential_id: row.get(3)?,
                repository_descriptor: row.get(4)?,
                passkey_created_at: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
                passkey_created_on: row.get(6)?,
                created_at: row.get::<_, i64>(7)? as u64,
            })
        })
        .optional()
        .map_err(map_err)
    }

    async fn delete_account(&self, account_id: i64, email: &str) -> Result<bool, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(DELETE_ACCOUNT_LINKS, params![account_id])
            .map_err(map_err)?;
        tx.execute(DELETE_ACCOUNT_DEVICES, params![account_id])
            .map_err(map_err)?;
        tx.execute(DELETE_CODE, params![email]).map_err(map_err)?;
        let changed = tx
            .execute(DELETE_ACCOUNT, params![account_id, email])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
    }

    async fn establish_repository_descriptor(
        &self,
        account_id: i64,
        candidate: &[u8],
    ) -> Result<(Vec<u8>, bool), StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        let created: Option<Vec<u8>> = tx
            .query_row(
                ESTABLISH_REPOSITORY_DESCRIPTOR,
                params![account_id, candidate],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_err)?;
        let winner: Option<Vec<u8>> = tx
            .query_row(SELECT_REPOSITORY_DESCRIPTOR, params![account_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(map_err)?;
        let winner = winner.ok_or_else(|| StoreError::Internal("account not found".to_string()))?;
        tx.commit().map_err(map_err)?;
        Ok((winner, created.is_some()))
    }

    async fn insert_device(&self, device: &Device) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            INSERT_DEVICE,
            params![
                device.account_id,
                device.device_did,
                device.attachment_id,
                device.delegation_cid,
                device.delegation_hex,
                device.name,
                device.status.as_str(),
                device.created_at as i64,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn devices(&self, account_id: i64) -> Result<Vec<Device>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(SELECT_DEVICES_BY_ACCOUNT).map_err(map_err)?;
        let rows: Vec<DeviceRow> = stmt
            .query_map(params![account_id], device_row)
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?;
        rows.into_iter().map(device_from_row).collect()
    }

    async fn active_device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_ACTIVE_DEVICE_BY_DID, params![device_did], device_row)
            .optional()
            .map_err(map_err)?;
        row.map(device_from_row).transpose()
    }

    async fn device_for_account(
        &self,
        account_id: i64,
        device_did: &str,
    ) -> Result<Option<Device>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row: Option<DeviceRow> = conn
            .query_row(
                SELECT_DEVICE_FOR_ACCOUNT,
                params![account_id, device_did],
                device_row,
            )
            .optional()
            .map_err(map_err)?;
        row.map(device_from_row).transpose()
    }

    async fn attachment(&self, attachment_id: &str) -> Result<Option<Device>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_ATTACHMENT, params![attachment_id], device_row)
            .optional()
            .map_err(map_err)?;
        row.map(device_from_row).transpose()
    }

    async fn revoke_device(&self, account_id: i64, device_did: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(UPDATE_DEVICE_REVOKE, params![account_id, device_did])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn revoke_device_by_cid(
        &self,
        account_id: i64,
        delegation_cid: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                UPDATE_DEVICE_REVOKE_BY_CID,
                params![account_id, delegation_cid],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn put_link(&self, link: &LinkRequest) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            INSERT_LINK,
            params![
                link.token_hash,
                link.device_did,
                link.device_name,
                link.created_at as i64,
                link.expires_at as i64,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn link(&self, token_hash: &str) -> Result<Option<LinkRequest>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_LINK, params![token_hash], link_row)
            .optional()
            .map_err(map_err)
    }

    async fn complete_link(&self, completion: &LinkCompletion<'_>) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                COMPLETE_LINK,
                params![
                    completion.account_id,
                    completion.attachment_id,
                    completion.delegation_cid,
                    completion.delegation_hex,
                    completion.descriptor_hex,
                    completion.now as i64,
                    (completion.now + 24 * 60 * 60) as i64,
                    completion.token_hash,
                    completion.now as i64,
                ],
            )
            .map_err(map_err)?;
        Ok(changed == 1)
    }

    async fn consume_link(
        &self,
        token_hash: &str,
        now: u64,
    ) -> Result<Option<LinkRequest>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(
            CONSUME_LINK,
            params![now as i64, token_hash, now as i64],
            link_row,
        )
        .optional()
        .map_err(map_err)
    }

    async fn completed_link_by_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<Option<LinkRequest>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_LINK_BY_ATTACHMENT, params![attachment_id], link_row)
            .optional()
            .map_err(map_err)
    }

    async fn activate_completed_link(
        &self,
        token_hash: &str,
        attachment_id: &str,
        now: u64,
    ) -> Result<ActivateOutcome, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        let link = tx
            .query_row(SELECT_LINK, params![token_hash], link_row)
            .optional()
            .map_err(map_err)?;
        let Some(link) = link else {
            return Ok(ActivateOutcome::Unknown);
        };
        if link.attachment_id.as_deref() != Some(attachment_id)
            || link.account_id.is_none()
            || link.delegation_cid.is_none()
            || link.delegation_hex.is_none()
        {
            return Ok(ActivateOutcome::Unknown);
        }
        if link.cancelled_at.is_some() {
            return Ok(ActivateOutcome::Cancelled);
        }
        if let Some(row) = tx
            .query_row(SELECT_ATTACHMENT, params![attachment_id], device_row)
            .optional()
            .map_err(map_err)?
        {
            let device = device_from_row(row)?;
            return Ok(match device.status {
                DeviceStatus::Active => ActivateOutcome::Active(device),
                DeviceStatus::Detached => ActivateOutcome::Cancelled,
                DeviceStatus::Revoked => ActivateOutcome::RevokedDelegation,
            });
        }
        let delegation_cid = link.delegation_cid.as_deref().unwrap();
        let revoked: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE delegation_cid = ?1 AND status = 'revoked')",
                params![delegation_cid],
                |row| row.get(0),
            )
            .map_err(map_err)?;
        if revoked {
            return Ok(ActivateOutcome::RevokedDelegation);
        }
        let active = tx
            .query_row(
                SELECT_ACTIVE_DEVICE_BY_DID,
                params![link.device_did],
                device_row,
            )
            .optional()
            .map_err(map_err)?;
        if active.is_some() {
            return Ok(ActivateOutcome::ActiveDeviceConflict);
        }
        tx.execute(
            INSERT_DEVICE,
            params![
                link.account_id.unwrap(),
                link.device_did,
                attachment_id,
                delegation_cid,
                link.delegation_hex.unwrap(),
                link.device_name,
                DeviceStatus::Active.as_str(),
                now as i64,
            ],
        )
        .map_err(map_err)?;
        tx.execute(
            "UPDATE link_requests SET activated_at = COALESCE(activated_at, ?1) WHERE token_hash = ?2 AND attachment_id = ?3 AND cancelled_at IS NULL",
            params![now as i64, token_hash, attachment_id],
        )
        .map_err(map_err)?;
        let row = tx
            .query_row(SELECT_ATTACHMENT, params![attachment_id], device_row)
            .map_err(map_err)?;
        let device = device_from_row(row)?;
        tx.commit().map_err(map_err)?;
        Ok(ActivateOutcome::Active(device))
    }

    async fn detach_attachment(
        &self,
        attachment_id: &str,
        now: u64,
    ) -> Result<DetachStoreOutcome, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        if let Some(row) = tx
            .query_row(SELECT_ATTACHMENT, params![attachment_id], device_row)
            .optional()
            .map_err(map_err)?
        {
            let device = device_from_row(row)?;
            let outcome = match device.status {
                DeviceStatus::Active => {
                    tx.execute(
                        "UPDATE devices SET status = 'detached' WHERE attachment_id = ?1 AND status = 'active'",
                        params![attachment_id],
                    )
                    .map_err(map_err)?;
                    DetachStoreOutcome::Detached
                }
                DeviceStatus::Detached => DetachStoreOutcome::AlreadyDetached,
                DeviceStatus::Revoked => DetachStoreOutcome::Revoked,
            };
            tx.commit().map_err(map_err)?;
            return Ok(outcome);
        }
        let cancelled = tx
            .execute(
                "UPDATE link_requests SET cancelled_at = COALESCE(cancelled_at, ?1) WHERE attachment_id = ?2 AND completed_at IS NOT NULL AND activated_at IS NULL",
                params![now as i64, attachment_id],
            )
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(if cancelled == 1 {
            DetachStoreOutcome::CancelledPendingActivation
        } else {
            DetachStoreOutcome::UnknownAttachment
        })
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::core::devices::DeviceView;
    use rusqlite::Connection;

    use crate::store::sqlite::SqliteStore;
    use crate::store::{CodeRow, Device, DeviceStatus, Store, StoreError};

    #[dialog_common::test]
    async fn it_enforces_unique_email_and_root_did() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .create_account("a@x.com", "did:key:zRoot1", "cred1", 1)
            .await
            .unwrap();
        let dup_email = store
            .create_account("a@x.com", "did:key:zRoot2", "cred2", 2)
            .await;
        assert!(matches!(dup_email, Err(StoreError::Conflict(_))));
        let dup_root = store
            .create_account("b@x.com", "did:key:zRoot1", "cred3", 3)
            .await;
        assert!(matches!(dup_root, Err(StoreError::Conflict(_))));
    }

    #[dialog_common::test]
    async fn it_round_trips_devices_and_revocation() {
        let store = SqliteStore::in_memory().unwrap();
        let id = store
            .create_account("a@x.com", "did:key:zRoot", "cred", 1)
            .await
            .unwrap();
        let device = Device {
            id: 0,
            account_id: id,
            device_did: "did:key:zDev".into(),
            attachment_id: "05".repeat(32),
            delegation_cid: "bafyCid".into(),
            delegation_hex: "beef".into(),
            name: "laptop".into(),
            status: DeviceStatus::Active,
            created_at: 2,
        };
        store.insert_device(&device).await.unwrap();
        assert!(matches!(
            store.insert_device(&device).await,
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(store.devices(id).await.unwrap().len(), 1);
        assert!(store.revoke_device(id, "did:key:zDev").await.unwrap());
        assert_eq!(
            store
                .device_for_account(id, "did:key:zDev")
                .await
                .unwrap()
                .unwrap()
                .status,
            DeviceStatus::Revoked
        );
        assert!(!store.revoke_device(id, "did:key:zAbsent").await.unwrap());

        let other_id = store
            .create_account("b@x.com", "did:key:zRoot2", "cred2", 3)
            .await
            .unwrap();
        store
            .insert_device(&Device {
                id: 0,
                account_id: other_id,
                attachment_id: "07".repeat(32),
                delegation_cid: "bafyCid2".into(),
                delegation_hex: "cafe".into(),
                name: "other account".into(),
                status: DeviceStatus::Active,
                created_at: 4,
                ..device
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .device_for_account(id, "did:key:zDev")
                .await
                .unwrap()
                .unwrap()
                .delegation_cid,
            "bafyCid"
        );
        assert_eq!(
            store
                .device_for_account(other_id, "did:key:zDev")
                .await
                .unwrap()
                .unwrap()
                .delegation_cid,
            "bafyCid2"
        );
    }

    #[dialog_common::test]
    async fn it_preserves_legacy_devices_without_inventing_path_evidence() {
        let store = SqliteStore::in_memory().unwrap();
        let id = store
            .create_account("legacy@x.com", "did:key:zLegacyRoot", "cred", 1)
            .await
            .unwrap();
        {
            let conn = store.0.lock().expect("store mutex poisoned");
            conn.execute(
                "INSERT INTO devices \
                 (account_id, device_did, attachment_id, delegation_cid, name, status, created_at) \
                 VALUES (?1, ?2, ?3, ?3, ?4, 'active', ?5)",
                params![id, "did:key:zLegacyDevice", "bafyLegacy", "old laptop", 2],
            )
            .unwrap();
        }

        let device = store.devices(id).await.unwrap().pop().unwrap();
        assert!(device.delegation_hex.is_empty());
        assert_eq!(DeviceView::from(device).delegation_hex, None);
    }

    #[dialog_common::test]
    async fn it_upserts_and_consumes_email_codes() {
        let store = SqliteStore::in_memory().unwrap();
        let row = CodeRow {
            email: "a@x.com".into(),
            code_hash: "h1".into(),
            created_at: 1,
            expires_at: 601,
            attempts: 0,
        };
        store.put_code(&row).await.unwrap();
        store
            .put_code(&CodeRow {
                code_hash: "h2".into(),
                ..row.clone()
            })
            .await
            .unwrap();
        store.bump_attempts("a@x.com").await.unwrap();
        let read = store.code("a@x.com").await.unwrap().unwrap();
        assert_eq!((read.code_hash.as_str(), read.attempts), ("h2", 1));
        store.delete_code("a@x.com").await.unwrap();
        assert!(store.code("a@x.com").await.unwrap().is_none());
    }

    #[dialog_common::test]
    fn it_migrates_deployed_devices_to_account_scoped_registration() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(include_str!("../../migrations/0001_init.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../../migrations/0002_link_requests.sql"))
            .unwrap();
        conn.execute_batch(include_str!(
            "../../migrations/0004_account_repository_descriptor.sql"
        ))
        .unwrap();
        conn.execute_batch(
            "ALTER TABLE devices ADD COLUMN delegation_hex TEXT NOT NULL DEFAULT '';\
             INSERT INTO accounts (email, root_did, credential_id, created_at)\
             VALUES ('a@x.com', 'did:key:zRoot', 'cred', 1);\
             INSERT INTO devices (account_id, device_did, delegation_cid, delegation_hex, name, status, created_at)\
             VALUES (1, 'did:key:zDev', 'bafyCid', 'obsolete', 'laptop', 'active', 2);",
        )
        .unwrap();

        conn.execute_batch(include_str!("../../migrations/0005_normalize_devices.sql"))
            .unwrap();
        conn.execute_batch(include_str!(
            "../../migrations/0006_device_attachment_lifecycle.sql"
        ))
        .unwrap();

        let columns = conn
            .prepare("PRAGMA table_info(devices)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            columns,
            [
                "id",
                "account_id",
                "device_did",
                "attachment_id",
                "delegation_cid",
                "delegation_hex",
                "name",
                "status",
                "created_at"
            ]
        );
        // The signed path cannot be reconstructed from its CID, so normalizing
        // must carry it across the rebuild or cross-device revocation silently
        // stops working for every already-registered device.
        let retained: (String, String, String) = conn
            .query_row(
                "SELECT device_did, delegation_cid, delegation_hex FROM devices",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            retained,
            ("did:key:zDev".into(), "bafyCid".into(), "obsolete".into())
        );

        conn.execute_batch(
            "INSERT INTO accounts (email, root_did, credential_id, created_at)\
             VALUES ('b@x.com', 'did:key:zRoot2', 'cred2', 3);",
        )
        .unwrap();
        assert!(conn
            .execute(
                "INSERT INTO devices \
                 (account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at) \
                 VALUES (2, ?1, 'new-attachment', 'bafyCid2', 'cafe', 'other account', 'active', 4)",
                params!["did:key:zDev"],
            )
            .is_err());
        conn.execute(
            "UPDATE devices SET status = 'detached' WHERE device_did = ?1",
            params!["did:key:zDev"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO devices \
             (account_id, device_did, attachment_id, delegation_cid, delegation_hex, name, status, created_at) \
             VALUES (2, ?1, 'new-attachment', 'bafyCid2', 'cafe', 'other account', 'active', 4)",
            params!["did:key:zDev"],
        )
        .unwrap();
        let registrations: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM devices WHERE device_did = ?1",
                params!["did:key:zDev"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(registrations, 2);
    }

    #[dialog_common::test]
    fn it_migrates_nullable_passkey_creation_metadata() {
        let store = SqliteStore::in_memory().unwrap();
        let conn = store.0.lock().expect("store mutex poisoned");
        let columns = conn
            .prepare("PRAGMA table_info(accounts)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(columns.contains(&"passkey_created_at".to_string()));
        assert!(columns.contains(&"passkey_created_on".to_string()));
    }

    #[dialog_common::test]
    async fn it_enforces_the_device_account_foreign_key() {
        let store = SqliteStore::in_memory().unwrap();
        let orphan = Device {
            id: 0,
            account_id: 999,
            device_did: "did:key:zOrphan".into(),
            attachment_id: "06".repeat(32),
            delegation_cid: "bafyCid".into(),
            delegation_hex: "beef".into(),
            name: "ghost".into(),
            status: DeviceStatus::Active,
            created_at: 1,
        };
        assert!(matches!(
            store.insert_device(&orphan).await,
            Err(StoreError::Internal(_))
        ));
    }
}
