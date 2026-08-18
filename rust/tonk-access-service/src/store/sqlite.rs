//! Native `rusqlite` twin of the [`Store`](crate::store::Store) trait,
//! used for tests and local development. Backed by the same schema
//! applied to the control D1 database in production.

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use super::{
    ACTIVATE_CUSTOMER, Consumer, Customer, INSERT_CUSTOMER, INSERT_SELF_CONSUMER, SELECT_CONSUMER,
    SELECT_CUSTOMER, Store, StoreError, UPDATE_REGISTERED_EMAIL, parse_status,
};

/// Native `rusqlite`-backed [`Store`], for tests and local development.
pub struct SqliteStore(Mutex<Connection>);

impl SqliteStore {
    /// Open a fresh in-memory database and apply every migration under
    /// `migrations/` — the same schema applied to the control D1 database
    /// in production.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        conn.execute_batch(include_str!("../../migrations/0001_control.sql"))
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

impl Store for SqliteStore {
    async fn customer(&self, did: &str) -> Result<Option<Customer>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_CUSTOMER, params![did], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        row.map(|(did, email, status, plan, verified, terms_version)| {
            Ok(Customer {
                did,
                email,
                status: parse_status(&status)?,
                plan,
                verified: verified as u64,
                terms_version,
            })
        })
        .transpose()
    }

    async fn consumer(&self, did: &str) -> Result<Option<Consumer>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        conn.query_row(SELECT_CONSUMER, params![did], |row| {
            Ok(Consumer {
                did: row.get(0)?,
                provider: row.get(1)?,
                registered: row.get::<_, i64>(2)? as u64,
            })
        })
        .optional()
        .map_err(map_err)
    }

    async fn enroll_customer(
        &self,
        did: &str,
        email: &str,
        access: &[u8],
        plan: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            INSERT_CUSTOMER,
            params![did, email, plan, now as i64, access],
        )
        .map_err(map_err)?;
        tx.execute(INSERT_SELF_CONSUMER, params![did, now as i64])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)
    }

    async fn update_registered_email(&self, did: &str, email: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(UPDATE_REGISTERED_EMAIL, params![did, email])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn activate_customer(
        &self,
        did: &str,
        terms_version: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(ACTIVATE_CUSTOMER, params![did, now as i64, terms_version])
            .map_err(map_err)?;
        Ok(changed > 0)
    }
}
