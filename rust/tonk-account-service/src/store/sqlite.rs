//! Native `rusqlite` twin of the [`Store`](crate::store::Store) trait,
//! used for tests and local development. Backed by the same schema
//! applied to Cloudflare D1 in production.

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Account, CodeRow, Device, DeviceStatus, Store, StoreError};

/// Native `rusqlite`-backed [`Store`], for tests and local development.
///
/// Wraps a single connection behind a mutex: sqlite serializes writes
/// internally, and the volume this crate deals with does not warrant a
/// connection pool.
pub struct SqliteStore(Mutex<Connection>);

impl SqliteStore {
    /// Open a fresh in-memory database and apply
    /// `migrations/0001_init.sql` — the same schema applied to
    /// Cloudflare D1 in production.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0001_init.sql"))
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
type DeviceRow = (i64, String, String, String, String, i64);

fn device_from_row(row: DeviceRow) -> Result<Device, StoreError> {
    let (account_id, device_did, delegation_cid, name, status, created_at) = row;
    Ok(Device {
        account_id,
        device_did,
        delegation_cid,
        name,
        status: DeviceStatus::parse(&status)?,
        created_at: created_at as u64,
    })
}

impl Store for SqliteStore {
    async fn code(&self, email: &str) -> Result<Option<CodeRow>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(
            "SELECT email, code_hash, created_at, expires_at, attempts \
             FROM email_codes WHERE email = ?1",
            params![email],
            |row| {
                Ok(CodeRow {
                    email: row.get(0)?,
                    code_hash: row.get(1)?,
                    created_at: row.get::<_, i64>(2)? as u64,
                    expires_at: row.get::<_, i64>(3)? as u64,
                    attempts: row.get::<_, i64>(4)? as u32,
                })
            },
        )
        .optional()
        .map_err(map_err)
    }

    async fn put_code(&self, row: &CodeRow) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO email_codes (email, code_hash, created_at, expires_at, attempts) \
             VALUES (?1, ?2, ?3, ?4, 0) \
             ON CONFLICT(email) DO UPDATE SET \
                code_hash = excluded.code_hash, \
                created_at = excluded.created_at, \
                expires_at = excluded.expires_at, \
                attempts = 0",
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
        conn.execute(
            "UPDATE email_codes SET attempts = attempts + 1 WHERE email = ?1",
            params![email],
        )
        .map_err(map_err)?;
        Ok(())
    }

    async fn delete_code(&self, email: &str) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute("DELETE FROM email_codes WHERE email = ?1", params![email])
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
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO accounts (email, root_did, credential_id, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![email, root_did, credential_id, created_at as i64],
        )
        .map_err(map_err)?;
        Ok(conn.last_insert_rowid())
    }

    async fn account_by_root(&self, root_did: &str) -> Result<Option<Account>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(
            "SELECT id, email, root_did, credential_id, created_at \
             FROM accounts WHERE root_did = ?1",
            params![root_did],
            |row| {
                Ok(Account {
                    id: row.get(0)?,
                    email: row.get(1)?,
                    root_did: row.get(2)?,
                    credential_id: row.get(3)?,
                    created_at: row.get::<_, i64>(4)? as u64,
                })
            },
        )
        .optional()
        .map_err(map_err)
    }

    async fn insert_device(&self, device: &Device) -> Result<(), StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO devices (account_id, device_did, delegation_cid, name, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                device.account_id,
                device.device_did,
                device.delegation_cid,
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
        let mut stmt = conn
            .prepare(
                "SELECT account_id, device_did, delegation_cid, name, status, created_at \
                 FROM devices WHERE account_id = ?1",
            )
            .map_err(map_err)?;
        let rows: Vec<DeviceRow> = stmt
            .query_map(params![account_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(map_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_err)?;
        rows.into_iter().map(device_from_row).collect()
    }

    async fn device_by_did(&self, device_did: &str) -> Result<Option<Device>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row: Option<DeviceRow> = conn
            .query_row(
                "SELECT account_id, device_did, delegation_cid, name, status, created_at \
                 FROM devices WHERE device_did = ?1",
                params![device_did],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(map_err)?;
        row.map(device_from_row).transpose()
    }

    async fn revoke_device(&self, account_id: i64, device_did: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                "UPDATE devices SET status = 'revoked' WHERE account_id = ?1 AND device_did = ?2",
                params![account_id, device_did],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
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
            account_id: id,
            device_did: "did:key:zDev".into(),
            delegation_cid: "bafyCid".into(),
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
                .device_by_did("did:key:zDev")
                .await
                .unwrap()
                .unwrap()
                .status,
            DeviceStatus::Revoked
        );
        assert!(!store.revoke_device(id, "did:key:zAbsent").await.unwrap());
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
}
