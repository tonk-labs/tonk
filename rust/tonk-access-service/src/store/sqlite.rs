//! Native `rusqlite` twin of the [`Store`](crate::store::Store) trait,
//! used for tests and local development. Backed by the same schema
//! applied to the control D1 database in production.

use std::sync::Mutex;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};

use super::{
    ACTIVATE_CUSTOMER, ACTIVATE_SUBSCRIPTIONS, ADD_SUBSCRIPTION, ARCHIVE_SUBSCRIPTION, Customer,
    DELETE_CUSTOMER, DELETE_PURGED_SUBSCRIPTIONS, DELETE_SELF_SUBSCRIPTION, Enrollment,
    INSERT_CUSTODY_SUBSCRIPTION, INSERT_CUSTOMER, INSERT_LEDGER_SUBSCRIPTION,
    INSERT_SELF_SUBSCRIPTION, RECORD_ACTIVATION_SENT, RELEASE_LAPSED_ADDRESS, REMOVE_SUBSCRIPTION,
    RESUME_SUBSCRIPTION, SELECT_CUSTOMER, SELECT_CUSTOMER_BY_EMAIL, SELECT_SERVABILITY,
    SELECT_SUBSCRIPTION, SELECT_SUBSCRIPTIONS_BY_OWNER, START_SELF_SUBSCRIPTION_DELETION,
    START_SUBSCRIPTION_DELETION, SUSPEND_SUBSCRIPTION, Servability, Store, StoreError,
    Subscription, SubscriptionKind, Suspension, UPDATE_REGISTERED_EMAIL, parse_status,
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
    /// Set a subscription's expiry, which nothing writes in production
    /// yet: renewal is the increment that will.
    pub(crate) async fn expire_for_test(&self, consumer: &str, at: u64) {
        self.0
            .lock()
            .expect("store mutex poisoned")
            .execute(
                "UPDATE subscription SET expires_at = ?2 WHERE consumer = ?1",
                params![consumer, at as i64],
            )
            .expect("the expiry is written");
    }

    /// Suspend a customer, which nothing writes in production yet:
    /// there is no admin path, so the fixture sets the column the way
    /// one eventually will.
    #[cfg(test)]
    pub(crate) async fn suspend_for_test(&self, did: &str) {
        self.0
            .lock()
            .expect("store mutex poisoned")
            .execute(
                "UPDATE customer SET status = 'Suspended' WHERE account = ?1",
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
            version = 5;
        }
        if version < 6 {
            conn.execute_batch(include_str!("../../migrations/0006_account_schema.sql"))
                .map_err(map_err)?;
            conn.pragma_update(None, "user_version", 6)
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
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        row.map(
            |(did, email, ledger, status, plan, verified_at, terms_version)| {
                Ok(Customer {
                    account: did,
                    email,
                    ledger,
                    status: parse_status(&status)?,
                    plan,
                    verified_at: verified_at as u64,
                    terms_version,
                })
            },
        )
        .transpose()
    }

    async fn customer_by_email(&self, email: &str) -> Result<Option<Customer>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_CUSTOMER_BY_EMAIL, params![email], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        row.map(
            |(did, email, ledger, status, plan, verified_at, terms_version)| {
                Ok(Customer {
                    account: did,
                    email,
                    ledger,
                    status: parse_status(&status)?,
                    plan,
                    verified_at: verified_at as u64,
                    terms_version,
                })
            },
        )
        .transpose()
    }

    async fn servability(&self, did: &str) -> Result<Servability, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_SERVABILITY, params![did], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        // The `(SELECT ?1)` on the left of every join always yields a
        // row, so `None` here means the query itself found nothing —
        // treated the same as a subject with no rows at all.
        let Some((
            own,
            consumer,
            expires_at,
            deleted_at,
            suspend_code,
            suspend_message,
            suspend_until_at,
            archived_at,
            provider_status,
        )) = row
        else {
            return Ok(Servability::default());
        };
        Ok(Servability {
            own: own.as_deref().map(parse_status).transpose()?,
            consumer: consumer.is_some(),
            expires_at: expires_at.map(|value| value as u64),
            deleted_at: deleted_at.map(|value| value as u64),
            // The code and the message are written together, so one
            // without the other is a row nothing could explain.
            suspension: suspend_code.map(|code| Suspension {
                message: suspend_message.unwrap_or_default(),
                code,
                until: suspend_until_at.map(|value| value as u64),
            }),
            archived_at: archived_at.map(|value| value as u64),
            provider: provider_status.as_deref().map(parse_status).transpose()?,
        })
    }

    async fn consumer(&self, did: &str) -> Result<Option<Subscription>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(SELECT_SUBSCRIPTION, params![did], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .optional()
            .map_err(map_err)?;
        row.map(
            |(did, provider, registered_at, kind, deleted_at, expires_at)| {
                Ok(Subscription {
                    consumer: did,
                    provider,
                    registered_at: registered_at as u64,
                    kind: SubscriptionKind::parse(&kind)?,
                    deleted_at: deleted_at.map(|value| value as u64),
                    expires_at: expires_at.map(|value| value as u64),
                })
            },
        )
        .transpose()
    }

    async fn subscriptions_by_provider(
        &self,
        provider: &str,
    ) -> Result<Vec<Subscription>, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let mut statement = conn
            .prepare(SELECT_SUBSCRIPTIONS_BY_OWNER)
            .map_err(map_err)?;
        let rows = statement
            .query_map(params![provider], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(map_err)?;
        rows.map(|row| {
            let (did, provider, registered_at, kind, deleted_at, expires_at) =
                row.map_err(map_err)?;
            Ok(Subscription {
                consumer: did,
                provider,
                registered_at: registered_at as u64,
                kind: SubscriptionKind::parse(&kind)?,
                deleted_at: deleted_at.map(|value| value as u64),
                expires_at: expires_at.map(|value| value as u64),
            })
        })
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
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            INSERT_CUSTOMER,
            params![did, email, ledger, plan, now as i64],
        )
        .map_err(map_err)?;
        tx.execute(
            INSERT_SELF_SUBSCRIPTION,
            params![did, now as i64, expires_at as i64],
        )
        .map_err(map_err)?;
        tx.execute(
            INSERT_LEDGER_SUBSCRIPTION,
            params![ledger, did, now as i64, expires_at as i64],
        )
        .map_err(map_err)?;
        // The passkey's custody space, in the same transaction as the
        // customer it belongs to: an enrollment claims every namespace it
        // needs or claims none. Split across two writes, a failure between
        // them left a customer whose own passkey could not be read back.
        tx.execute(
            INSERT_CUSTODY_SUBSCRIPTION,
            params![custody, did, now as i64, expires_at as i64],
        )
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

    async fn release_lapsed_address(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(RELEASE_LAPSED_ADDRESS, params![did, now as i64])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn add_subscription(
        &self,
        did: &str,
        provider: &str,
        now: u64,
        kind: SubscriptionKind,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                ADD_SUBSCRIPTION,
                params![did, provider, now as i64, kind.as_str()],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn suspend_subscription(
        &self,
        consumer: &str,
        code: &str,
        message: &str,
        until: Option<u64>,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(
                SUSPEND_SUBSCRIPTION,
                params![consumer, code, message, until.map(|at| at as i64)],
            )
            .map_err(map_err)?
            > 0)
    }

    async fn resume_subscription(&self, consumer: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(RESUME_SUBSCRIPTION, params![consumer])
            .map_err(map_err)?
            > 0)
    }

    async fn archive_subscription(&self, consumer: &str, now: u64) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(ARCHIVE_SUBSCRIPTION, params![consumer, now as i64])
            .map_err(map_err)?
            > 0)
    }

    async fn mark_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(START_SUBSCRIPTION_DELETION, params![did, now as i64])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn finish_consumer_deletion(&self, did: &str) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(REMOVE_SUBSCRIPTION, params![did])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    async fn mark_self_consumer_deleting(&self, did: &str, now: u64) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(START_SELF_SUBSCRIPTION_DELETION, params![did, now as i64])
            .map_err(map_err)?
            > 0)
    }

    async fn delete_customer(&self, did: &str) -> Result<bool, StoreError> {
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(DELETE_PURGED_SUBSCRIPTIONS, params![did])
            .map_err(map_err)?;
        tx.execute(DELETE_SELF_SUBSCRIPTION, params![did])
            .map_err(map_err)?;
        let changed = tx.execute(DELETE_CUSTOMER, params![did]).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(changed == 1)
    }

    async fn claim_activation_resend(
        &self,
        account: &str,
        now: u64,
        not_since: u64,
    ) -> Result<bool, StoreError> {
        let conn = self.0.lock().expect("store mutex poisoned");
        Ok(conn
            .execute(
                RECORD_ACTIVATION_SENT,
                params![account, now as i64, not_since as i64],
            )
            .map_err(map_err)?
            > 0)
    }

    async fn activate_customer(
        &self,
        did: &str,
        terms_version: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        // Status and expiry together: the subscriptions were written to
        // lapse if the link was never clicked, and it has been. Half of
        // this would leave an active customer whose spaces expire.
        let mut conn = self.0.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(map_err)?;
        let changed = tx
            .execute(ACTIVATE_CUSTOMER, params![did, now as i64, terms_version])
            .map_err(map_err)?;
        tx.execute(ACTIVATE_SUBSCRIPTIONS, params![did])
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
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
            .enroll_customer(Enrollment {
                did,
                email,
                plan: SIGNUP_PLAN,
                ledger: did,
                custody: &format!("{did}-custody"),
                now: 1_700_000_000,
                expires_at: u64::MAX,
            })
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
        assert_eq!(found.account, "did:key:zA");
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
            .enroll_customer(Enrollment {
                did: "did:key:zB",
                email: "jsmith@example.com",
                plan: SIGNUP_PLAN,
                ledger: "did:key:zB",
                custody: "did:key:zB-custody",
                now: 1_700_000_001,
                expires_at: u64::MAX,
            })
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
                "UPDATE customer SET status = 'Suspended' WHERE account = ?1",
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
