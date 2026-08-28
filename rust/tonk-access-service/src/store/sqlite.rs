//! Native `rusqlite` twin of the [`Store`](crate::store::Store) trait,
//! used for tests and local development. Backed by the same schema
//! applied to the control D1 database in production.

use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};

use super::{
    ACTIVATE_CUSTOMER, ADD_CONSUMER, ANONYMIZE_DELETED_CONSUMERS, Consumer, ConsumerDeletionState,
    ConsumerKind, Customer, DELETE_CUSTOMER, DELETE_SELF_CONSUMER, FINISH_CONSUMER_DELETION,
    INSERT_CUSTOMER, INSERT_SELF_CONSUMER, MARK_CONSUMER_DELETING, MARK_SELF_CONSUMER_DELETING,
    SELECT_CONSUMER, SELECT_CONSUMERS_BY_OWNER, SELECT_CUSTOMER, SELECT_CUSTOMER_BY_EMAIL, Store,
    StoreError, UPDATE_REGISTERED_EMAIL, parse_status,
};

/// Native `rusqlite`-backed [`Store`], for tests and local development.
pub struct SqliteStore(Mutex<Connection>);

impl SqliteStore {
    /// Open a fresh in-memory database and apply every migration under
    /// `migrations/` — the same schema applied to the control D1 database
    /// in production.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        Self::prepare(conn)
    }

    /// Set a customer's status to `Suspended`, for tests.
    ///
    /// No production path writes that status: suspension is an
    /// administrative act and the service has no endpoint for it yet.
    /// The screening gate still has to answer for it, so tests need a
    /// way to produce one.
    #[cfg(test)]
    pub(crate) async fn suspend_for_test(&self, did: &str) {
        self.0
            .lock()
            .expect("store mutex poisoned")
            .execute(
                "UPDATE customer SET status = 'Suspended' WHERE did = ?1",
                params![did],
            )
            .expect("the status is written");
    }

    /// Open (or create) a file-backed database, applying the migrations
    /// only on first open. Development durability: a restarted local
    /// service keeps its customers instead of wiping them.
    pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(map_err)?;
        Self::prepare(conn)
    }

    fn prepare(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        let mut version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(map_err)?;
        if version == 0 {
            conn.execute_batch(include_str!("../../migrations/0001_control.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 1)
                .map_err(map_err)?;
            version = 1;
        }
        if version < 2 {
            conn.execute_batch(include_str!("../../migrations/0002_deletion.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 2)
                .map_err(map_err)?;
            version = 2;
        }
        if version < 3 {
            conn.execute_batch(include_str!("../../migrations/0003_deprovision.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 3)
                .map_err(map_err)?;
            version = 3;
        }
        if version < 4 {
            conn.execute_batch(include_str!("../../migrations/0004_consumer_kind.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 4)
                .map_err(map_err)?;
            version = 4;
        }
        if version < 5 {
            conn.execute_batch(include_str!("../../migrations/0005_customer_email.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 5)
                .map_err(map_err)?;
        }
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

#[async_trait]
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

    async fn customer_by_email(&self, email: &str) -> Result<Option<Customer>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_CUSTOMER_BY_EMAIL, params![email], |row| {
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
        let row = conn
            .query_row(SELECT_CONSUMER, params![did], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        row.map(
            |(did, provider, owner, registered, kind, state, deleted_at)| {
                Ok(Consumer {
                    did,
                    provider,
                    owner,
                    registered: registered as u64,
                    kind: ConsumerKind::parse(&kind)?,
                    deletion_state: ConsumerDeletionState::parse(&state)?,
                    deleted_at: deleted_at.map(|value| value as u64),
                })
            },
        )
        .transpose()
    }

    async fn consumers_by_owner(&self, owner: &str) -> Result<Vec<Consumer>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let mut statement = conn.prepare(SELECT_CONSUMERS_BY_OWNER).map_err(map_err)?;
        let rows = statement
            .query_map(params![owner], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .map_err(map_err)?;
        rows.map(|row| {
            let (did, provider, owner, registered, kind, state, deleted_at) =
                row.map_err(map_err)?;
            Ok(Consumer {
                did,
                provider,
                owner,
                registered: registered as u64,
                kind: ConsumerKind::parse(&kind)?,
                deletion_state: ConsumerDeletionState::parse(&state)?,
                deleted_at: deleted_at.map(|value| value as u64),
            })
        })
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

    async fn add_consumer(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: ConsumerKind,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                ADD_CONSUMER,
                params![did, provider, now as i64, kind.as_str()],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn mark_consumer_deleting(&self, did: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(MARK_CONSUMER_DELETING, params![did])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn finish_consumer_deletion(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(FINISH_CONSUMER_DELETION, params![did, now as i64])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn mark_self_consumer_deleting(&self, did: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(MARK_SELF_CONSUMER_DELETING, params![did])
            .map_err(map_err)?
            > 0)
    }

    async fn delete_customer(&self, did: &str) -> Result<bool, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(ANONYMIZE_DELETED_CONSUMERS, params![did])
            .map_err(map_err)?;
        tx.execute(DELETE_SELF_CONSUMER, params![did])
            .map_err(map_err)?;
        let changed = tx.execute(DELETE_CUSTOMER, params![did]).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SIGNUP_PLAN;
    use tonk_account::customer::CustomerStatus;

    async fn enrolled(store: &SqliteStore, did: &str, email: &str) {
        store
            .enroll_customer(did, email, &[], SIGNUP_PLAN, 1_700_000_000)
            .await
            .expect("enrollment writes a customer");
    }

    #[dialog_common::test]
    async fn it_finds_an_enrolled_customer_by_their_address() {
        let store = SqliteStore::in_memory().expect("store");
        enrolled(&store, "did:key:zA", "jsmith@example.com").await;

        let found = store
            .customer_by_email("jsmith@example.com")
            .await
            .expect("lookup succeeds")
            .expect("the enrolled customer is found");
        assert_eq!(found.did, "did:key:zA");
        assert_eq!(found.status, CustomerStatus::Registered);
    }

    #[dialog_common::test]
    async fn it_finds_nothing_for_an_unregistered_address() {
        let store = SqliteStore::in_memory().expect("store");
        enrolled(&store, "did:key:zA", "jsmith@example.com").await;

        assert!(
            store
                .customer_by_email("nobody@example.com")
                .await
                .expect("lookup succeeds")
                .is_none()
        );
    }

    /// The lookup answers with one customer because the schema permits
    /// only one. Without the unique index this is where a second account
    /// on an address would slip in and make the answer arbitrary.
    #[dialog_common::test]
    async fn it_refuses_a_second_customer_on_one_address() {
        let store = SqliteStore::in_memory().expect("store");
        enrolled(&store, "did:key:zA", "jsmith@example.com").await;

        let conflict = store
            .enroll_customer(
                "did:key:zB",
                "jsmith@example.com",
                &[],
                SIGNUP_PLAN,
                1_700_000_001,
            )
            .await;
        assert!(
            matches!(conflict, Err(StoreError::Conflict(_))),
            "a second customer on one address is a conflict, got {conflict:?}"
        );
    }

    /// The address a lookup is keyed by is the normalized one, so a
    /// caller holding the address finds the row whatever the enrolling
    /// client sent. Enrollment normalizes before it reaches the store, so
    /// this pins the store's half: what goes in is what comes back out.
    #[dialog_common::test]
    async fn it_keys_a_customer_by_the_address_it_was_given() {
        let store = SqliteStore::in_memory().expect("store");
        enrolled(&store, "did:key:zA", "jsmith@example.com").await;

        assert!(
            store
                .customer_by_email("JSmith@Example.COM")
                .await
                .expect("lookup succeeds")
                .is_none(),
            "the store matches exactly; normalizing is the caller's job"
        );
    }

    /// A suspended customer is still found, so the lookup can answer 410
    /// with the key rather than pretending the address is unknown.
    /// Nothing writes `Suspended` yet, so the status is set directly.
    #[dialog_common::test]
    async fn it_finds_a_suspended_customer() {
        let store = SqliteStore::in_memory().expect("store");
        enrolled(&store, "did:key:zA", "jsmith@example.com").await;
        store
            .0
            .lock()
            .expect("store mutex poisoned")
            .execute(
                "UPDATE customer SET status = 'Suspended' WHERE did = ?1",
                params!["did:key:zA"],
            )
            .expect("the status is written");

        let found = store
            .customer_by_email("jsmith@example.com")
            .await
            .expect("lookup succeeds")
            .expect("a suspended customer is still found");
        assert_eq!(found.status, CustomerStatus::Suspended);
    }
}
