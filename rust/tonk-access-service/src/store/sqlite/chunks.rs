use super::*;
use crate::delivery::chunks::{self, Chunk, Kind};
pub(super) fn insert(
    conn: &Connection,
    kind: Kind,
    key: &str,
    marker: &str,
    payload: &str,
) -> Result<(), StoreError> {
    for (ordinal, piece) in chunks::pieces(payload).enumerate() {
        conn.execute(kind.insert(), params![key, ordinal as u64, piece, marker])
            .map_err(map_err)?;
    }
    Ok(())
}
pub(super) fn load(
    conn: &Connection,
    kind: Kind,
    key: &str,
    value: String,
) -> Result<String, StoreError> {
    if !chunks::is_chunked(&value) {
        return Ok(value);
    }
    let mut statement = conn.prepare(kind.select()).map_err(map_err)?;
    let parts = statement
        .query_map([key], |r| {
            Ok(Chunk {
                ordinal: r.get(0)?,
                content: r.get(1)?,
            })
        })
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;
    chunks::assemble(&value, parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::additions::{AdditionPublication, AdditionStore, NewAddition};
    use crate::delivery::{DeliveryStore, Publication, StoredApproval};
    use crate::store::{Enrollment, SIGNUP_PLAN};

    #[dialog_common::test]
    async fn connection_chunk_migration_keeps_inline_payloads_readable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("legacy.sqlite");
        let conn = Connection::open(&file).unwrap();
        for sql in [
            include_str!("../../../migrations/0001_control.sql"),
            include_str!("../../../migrations/0002_deletion.sql"),
            include_str!("../../../migrations/0003_deprovision.sql"),
            include_str!("../../../migrations/0004_consumer_kind.sql"),
            include_str!("../../../migrations/0005_customer_email.sql"),
            include_str!("../../../migrations/0006_account_schema.sql"),
            include_str!("../../../migrations/0007_connection_delivery.sql"),
            include_str!("../../../migrations/0008_connection_additions.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute(
            "INSERT INTO connection_delivery VALUES('old','cli','account','aabb',1,2)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO connection_addition(delivery_id,request_hash,recipient,account,addition_hex,created_at) VALUES('old-add','old','cli','account','ccdd',3)", []).unwrap();
        conn.pragma_update(None, "user_version", 8).unwrap();
        drop(conn);
        let store = SqliteStore::open(&file).unwrap();
        assert_eq!(
            store
                .approval_for("old", "cli")
                .await
                .unwrap()
                .unwrap()
                .approval_hex,
            "aabb"
        );
        assert_eq!(
            store
                .next_addition("old", "cli", 0)
                .await
                .unwrap()
                .unwrap()
                .addition
                .addition_hex,
            "ccdd"
        );
        let conn = store.0.lock().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT payload_size FROM connection_delivery WHERE request_hash='old'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
            4
        );
    }

    #[dialog_common::test]
    async fn connection_chunk_storage_restart_rollback_corruption_and_account_purge() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("large.sqlite");
        let store = SqliteStore::open(&file).unwrap();
        for account in ["account-a", "account-b"] {
            store
                .enroll_customer(Enrollment {
                    did: account,
                    email: &format!("{account}@example.test"),
                    plan: SIGNUP_PLAN,
                    ledger: account,
                    custody: &format!("{account}-custody"),
                    now: 1,
                    expires_at: u64::MAX,
                })
                .await
                .unwrap();
            store.activate_delivery_account_for_test(account);
            let initial = StoredApproval {
                request_hash: account.into(),
                recipient: "cli".into(),
                account: account.into(),
                approval_hex: "ab".repeat(3 * 1024 * 1024),
                created_at: 1,
                approval_deadline: 2,
            };
            assert_eq!(
                store.publish_approval(&initial).await.unwrap(),
                Publication::Created
            );
            let addition = NewAddition {
                delivery_id: account.into(),
                request_hash: account.into(),
                recipient: "cli".into(),
                account: account.into(),
                addition_hex: "cd".repeat(3 * 1024 * 1024),
                created_at: 3,
            };
            assert_eq!(
                store.publish_addition(&addition).await.unwrap(),
                AdditionPublication::Created
            );
            assert_eq!(
                store.publish_approval(&initial).await.unwrap(),
                Publication::Existing
            );
            assert_eq!(
                store.publish_addition(&addition).await.unwrap(),
                AdditionPublication::Existing
            );
            assert_eq!(
                store.approval_for(account, "cli").await.unwrap(),
                Some(initial)
            );
            assert_eq!(
                store
                    .next_addition(account, "cli", 0)
                    .await
                    .unwrap()
                    .unwrap()
                    .addition,
                addition
            );
        }
        drop(store);
        let store = SqliteStore::open(&file).unwrap();
        assert_eq!(
            store
                .approval_for("account-a", "cli")
                .await
                .unwrap()
                .unwrap()
                .approval_hex,
            "ab".repeat(3 * 1024 * 1024)
        );
        assert_eq!(
            store
                .next_addition("account-a", "cli", 0)
                .await
                .unwrap()
                .unwrap()
                .addition
                .addition_hex,
            "cd".repeat(3 * 1024 * 1024)
        );
        {
            let conn = store.0.lock().unwrap();
            let max: usize = conn
                .query_row(
                    "SELECT MAX(LENGTH(content)) FROM connection_delivery_chunk",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(max, chunks::CHUNK_BYTES);
            conn.execute_batch("CREATE TRIGGER fail_chunk BEFORE INSERT ON connection_delivery_chunk WHEN NEW.request_hash='failed' AND NEW.ordinal=1 BEGIN SELECT RAISE(ABORT,'injected chunk failure'); END;").unwrap();
        }
        let failed = StoredApproval {
            request_hash: "failed".into(),
            recipient: "cli".into(),
            account: "account-a".into(),
            approval_hex: "ee".repeat(1024 * 1024),
            created_at: 1,
            approval_deadline: 2,
        };
        assert!(store.publish_approval(&failed).await.is_err());
        assert!(store.approval_for("failed", "cli").await.unwrap().is_none());
        {
            let conn = store.0.lock().unwrap();
            let count: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM connection_delivery_chunk WHERE request_hash='failed'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            conn.execute("DELETE FROM connection_delivery_chunk WHERE request_hash='account-a' AND ordinal=1", []).unwrap();
        }
        assert!(
            store.approval_for("account-a", "cli").await.is_err(),
            "partial content must never be returned"
        );
        assert!(
            !store.delete_customer("account-a").await.unwrap(),
            "active account blocks mailbox cleanup"
        );
        assert!(
            store
                .next_addition("account-a", "cli", 0)
                .await
                .unwrap()
                .is_some()
        );
        store.deny_customer("account-a", 5).await.unwrap();
        assert!(store.delete_customer("account-a").await.unwrap());
        assert!(!store.delete_customer("account-a").await.unwrap());
        assert!(
            store
                .approval_for("account-a", "cli")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .next_addition("account-a", "cli", 0)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .approval_for("account-b", "cli")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .next_addition("account-b", "cli", 0)
                .await
                .unwrap()
                .is_some()
        );
        let late = StoredApproval {
            request_hash: "late-after-purge".into(),
            ..failed
        };
        assert_eq!(
            store.publish_approval(&late).await.unwrap(),
            Publication::AccountUnavailable,
            "a publisher authenticated before purge must not recreate account metadata afterward"
        );
        assert!(
            store
                .approval_for("late-after-purge", "cli")
                .await
                .unwrap()
                .is_none()
        );
        let conn = store.0.lock().unwrap();
        for table in ["connection_delivery_chunk", "connection_addition_chunk"] {
            let key = if table == "connection_delivery_chunk" {
                "request_hash"
            } else {
                "delivery_id"
            };
            let count: usize = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE {key}='account-a'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        }
    }
}
