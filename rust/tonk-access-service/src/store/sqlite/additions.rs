use super::*;
use crate::delivery::additions::*;
use crate::delivery::chunks::{Kind, reference};

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAddition> {
    Ok(StoredAddition {
        sequence: row.get(0)?,
        addition: NewAddition {
            delivery_id: row.get(1)?,
            request_hash: row.get(2)?,
            recipient: row.get(3)?,
            account: row.get(4)?,
            addition_hex: row.get(5)?,
            created_at: row.get(6)?,
        },
    })
}
#[async_trait]
impl AdditionStore for SqliteStore {
    async fn publish_addition(
        &self,
        addition: &NewAddition,
    ) -> Result<AdditionPublication, StoreError> {
        valid_shape(addition)?;
        let marker = reference(&addition.addition_hex)?;
        let mut conn = self
            .0
            .lock()
            .map_err(|_| StoreError::Internal("store mutex poisoned".into()))?;
        let tx = conn.transaction().map_err(map_err)?;
        let changed = tx
            .execute(
                INSERT,
                params![
                    addition.delivery_id,
                    addition.request_hash,
                    addition.recipient,
                    addition.account,
                    marker,
                    addition.created_at,
                    addition.addition_hex.len() as i64
                ],
            )
            .map_err(map_err)?;
        super::chunks::insert(
            &tx,
            Kind::Addition,
            &addition.delivery_id,
            &marker,
            &addition.addition_hex,
        )?;
        tx.commit().map_err(map_err)?;
        if changed > 0 {
            return Ok(AdditionPublication::Created);
        }
        let stored = conn
            .query_row(SELECT_ID, [&addition.delivery_id], row)
            .optional()
            .map_err(map_err)?;
        if let Some(mut stored) = stored {
            stored.addition.addition_hex = super::chunks::load(
                &conn,
                Kind::Addition,
                &stored.addition.delivery_id,
                stored.addition.addition_hex,
            )?;
            return Ok(if stored.addition == *addition {
                AdditionPublication::Existing
            } else {
                AdditionPublication::Conflict
            });
        }
        let mailbox:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM connection_delivery WHERE request_hash=?1 AND recipient=?2 AND account=?3)",params![addition.request_hash,addition.recipient,addition.account],|r|r.get(0)).map_err(map_err)?;
        Ok(if mailbox {
            AdditionPublication::Capacity
        } else {
            AdditionPublication::UnknownMailbox
        })
    }
    async fn next_addition(
        &self,
        request_hash: &str,
        recipient: &str,
        after: u64,
    ) -> Result<Option<StoredAddition>, StoreError> {
        let conn = self
            .0
            .lock()
            .map_err(|_| StoreError::Internal("store mutex poisoned".into()))?;
        let stored = conn
            .query_row(SELECT_NEXT, params![request_hash, recipient, after], row)
            .optional()
            .map_err(map_err)?;
        hydrate(&conn, stored)
    }
    async fn addition_by_id(
        &self,
        id: &str,
        request_hash: &str,
        recipient: &str,
    ) -> Result<Option<StoredAddition>, StoreError> {
        let conn = self
            .0
            .lock()
            .map_err(|_| StoreError::Internal("store mutex poisoned".into()))?;
        let stored = conn
            .query_row(SELECT_ID, [id], row)
            .optional()
            .map_err(map_err)?;
        hydrate(
            &conn,
            stored.filter(|r| {
                r.addition.request_hash == request_hash && r.addition.recipient == recipient
            }),
        )
    }
}

fn hydrate(
    conn: &rusqlite::Connection,
    stored: Option<StoredAddition>,
) -> Result<Option<StoredAddition>, StoreError> {
    stored
        .map(|mut row| {
            row.addition.addition_hex = super::chunks::load(
                conn,
                Kind::Addition,
                &row.addition.delivery_id,
                row.addition.addition_hex,
            )?;
            Ok(row)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{DeliveryStore, StoredApproval};
    #[dialog_common::test]
    async fn connection_addition_storage_pins_account_and_recipient_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("addition.sqlite");
        let store = SqliteStore::open(&file).unwrap();
        store.activate_delivery_account_for_test("account");
        let original = StoredApproval {
            request_hash: "request".into(),
            recipient: "cli".into(),
            account: "account".into(),
            approval_hex: "00".into(),
            created_at: 1,
            approval_deadline: 2,
        };
        let addition = NewAddition {
            delivery_id: "delivery-a".into(),
            request_hash: "request".into(),
            recipient: "cli".into(),
            account: "account".into(),
            addition_hex: "11".into(),
            created_at: 10,
        };
        assert_eq!(
            store.publish_addition(&addition).await.unwrap(),
            AdditionPublication::UnknownMailbox
        );
        store.publish_approval(&original).await.unwrap();
        let mut forged = addition.clone();
        forged.account = "other-account".into();
        assert_eq!(
            store.publish_addition(&forged).await.unwrap(),
            AdditionPublication::UnknownMailbox
        );
        forged.account = "account".into();
        forged.recipient = "other-cli".into();
        assert_eq!(
            store.publish_addition(&forged).await.unwrap(),
            AdditionPublication::UnknownMailbox
        );
        assert_eq!(
            store.publish_addition(&addition).await.unwrap(),
            AdditionPublication::Created
        );
        assert_eq!(
            store.publish_addition(&addition).await.unwrap(),
            AdditionPublication::Existing
        );
        let mut conflict = addition.clone();
        conflict.addition_hex = "22".into();
        assert_eq!(
            store.publish_addition(&conflict).await.unwrap(),
            AdditionPublication::Conflict
        );
        let first = store
            .next_addition("request", "cli", 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.addition, addition);
        assert!(
            store
                .next_addition("request", "other-cli", 0)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .next_addition("request", "cli", first.sequence)
                .await
                .unwrap()
                .is_none()
        );
        drop(store);
        let reopened = SqliteStore::open(&file).unwrap();
        assert_eq!(
            reopened.next_addition("request", "cli", 0).await.unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            reopened.publish_addition(&addition).await.unwrap(),
            AdditionPublication::Existing
        );
        let mut next = addition.clone();
        next.delivery_id = "delivery-b".into();
        next.addition_hex = "33".into();
        assert_eq!(
            reopened.publish_addition(&next).await.unwrap(),
            AdditionPublication::Created
        );
        let second = reopened
            .next_addition("request", "cli", first.sequence)
            .await
            .unwrap()
            .unwrap();
        assert!(second.sequence > first.sequence);
        assert_eq!(second.addition, next);
        assert_eq!(
            reopened.approval_for("request", "cli").await.unwrap(),
            Some(original)
        );
    }
}
