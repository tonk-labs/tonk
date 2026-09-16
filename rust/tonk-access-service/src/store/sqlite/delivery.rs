use super::*;
use crate::delivery::chunks::{Kind, reference};
use crate::delivery::*;

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredApproval> {
    Ok(StoredApproval {
        request_hash: row.get(0)?,
        recipient: row.get(1)?,
        account: row.get(2)?,
        approval_hex: row.get(3)?,
        created_at: row.get(4)?,
        approval_deadline: row.get(5)?,
    })
}

#[async_trait]
impl DeliveryStore for SqliteStore {
    async fn publish_approval(&self, approval: &StoredApproval) -> Result<Publication, StoreError> {
        valid_storage_shape(approval)?;
        let mut conn = self
            .0
            .lock()
            .map_err(|_| StoreError::Internal("store mutex poisoned".into()))?;
        let marker = reference(&approval.approval_hex)?;
        let tx = conn.transaction().map_err(map_err)?;
        let changed = tx
            .execute(
                INSERT_APPROVAL,
                params![
                    approval.request_hash,
                    approval.recipient,
                    approval.account,
                    marker,
                    approval.created_at,
                    approval.approval_deadline,
                    approval.approval_hex.len() as u64
                ],
            )
            .map_err(map_err)?;
        super::chunks::insert(
            &tx,
            Kind::Initial,
            &approval.request_hash,
            &marker,
            &approval.approval_hex,
        )?;
        tx.commit().map_err(map_err)?;
        if changed > 0 {
            return Ok(Publication::Created);
        }
        let stored = conn
            .query_row(SELECT_BY_HASH, [&approval.request_hash], row)
            .optional()
            .map_err(map_err)?;
        let stored = stored
            .map(|mut row| {
                row.approval_hex =
                    super::chunks::load(&conn, Kind::Initial, &row.request_hash, row.approval_hex)?;
                Ok::<_, StoreError>(row)
            })
            .transpose()?;
        Ok(match stored {
            Some(stored) if stored == *approval => Publication::Existing,
            Some(_) => Publication::Conflict,
            None => {
                let active:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM customer WHERE account=?1 AND status='Active')",[&approval.account],|r|r.get(0)).map_err(map_err)?;
                if active {
                    Publication::Capacity
                } else {
                    Publication::AccountUnavailable
                }
            }
        })
    }

    async fn approval_for(
        &self,
        request_hash: &str,
        recipient: &str,
    ) -> Result<Option<StoredApproval>, StoreError> {
        let conn = self
            .0
            .lock()
            .map_err(|_| StoreError::Internal("store mutex poisoned".into()))?;
        let stored = conn
            .query_row(SELECT_APPROVAL, params![request_hash, recipient], row)
            .optional()
            .map_err(map_err)?;
        stored
            .map(|mut row| {
                row.approval_hex =
                    super::chunks::load(&conn, Kind::Initial, &row.request_hash, row.approval_hex)?;
                Ok(row)
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_delivery_concurrent_complete_approvals_do_not_mix() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("race.sqlite");
        let first = SqliteStore::open(&file).unwrap();
        let second = SqliteStore::open(&file).unwrap();
        first.activate_delivery_account_for_test("account-a");
        first.activate_delivery_account_for_test("account-b");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut joins = Vec::new();
        for (store, id) in [(first, "a"), (second, "b")] {
            let barrier = barrier.clone();
            joins.push(std::thread::spawn(move || {
                let approval = StoredApproval {
                    request_hash: "same-request".into(),
                    recipient: format!("recipient-{id}"),
                    account: format!("account-{id}"),
                    approval_hex: id.repeat(6 * 1024 * 1024),
                    created_at: 1,
                    approval_deadline: 2,
                };
                barrier.wait();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                (
                    runtime.block_on(store.publish_approval(&approval)).unwrap(),
                    approval,
                )
            }));
        }
        let results: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
        assert_eq!(
            results
                .iter()
                .filter(|(r, _)| *r == Publication::Created)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|(r, _)| *r == Publication::Conflict)
                .count(),
            1
        );
        let winning = &results
            .iter()
            .find(|(r, _)| *r == Publication::Created)
            .unwrap()
            .1;
        let store = SqliteStore::open(&file).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let stored = runtime
            .block_on(store.approval_for("same-request", &winning.recipient))
            .unwrap()
            .unwrap();
        assert_eq!(
            &stored, winning,
            "every address and payload field is from one winner"
        );
    }

    #[dialog_common::test]
    async fn connection_delivery_quota_preserves_existing_and_other_accounts() {
        let store = SqliteStore::in_memory().unwrap();
        store.activate_delivery_account_for_test("account-a");
        store.activate_delivery_account_for_test("account-b");
        let mut approval = StoredApproval {
            request_hash: String::new(),
            recipient: "recipient".into(),
            account: "account-a".into(),
            approval_hex: "00".into(),
            created_at: 1,
            approval_deadline: 2,
        };
        for id in 0..1024 {
            approval.request_hash = id.to_string();
            assert_eq!(
                store.publish_approval(&approval).await.unwrap(),
                Publication::Created
            );
        }
        assert_eq!(
            store.publish_approval(&approval).await.unwrap(),
            Publication::Existing
        );
        approval.request_hash = "overflow".into();
        assert_eq!(
            store.publish_approval(&approval).await.unwrap(),
            Publication::Capacity
        );
        assert!(
            store
                .approval_for("overflow", "recipient")
                .await
                .unwrap()
                .is_none()
        );
        approval.account = "account-b".into();
        assert_eq!(
            store.publish_approval(&approval).await.unwrap(),
            Publication::Created
        );
    }

    #[dialog_common::test]
    async fn connection_delivery_storage_is_atomic_immutable_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("delivery.sqlite");
        let original = StoredApproval {
            request_hash: "request-a".into(),
            recipient: "recipient-a".into(),
            account: "account-a".into(),
            approval_hex: "001122".into(),
            created_at: 10,
            approval_deadline: 20,
        };
        let store = SqliteStore::open(&file).unwrap();
        store.activate_delivery_account_for_test("account-a");
        assert!(
            store
                .approval_for("missing", "recipient-a")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store.publish_approval(&original).await.unwrap(),
            Publication::Created
        );
        assert_eq!(
            store.publish_approval(&original).await.unwrap(),
            Publication::Existing
        );
        let mut conflicting = original.clone();
        conflicting.approval_hex = "334455".into();
        assert_eq!(
            store.publish_approval(&conflicting).await.unwrap(),
            Publication::Conflict
        );
        conflicting.recipient = "recipient-b".into();
        assert_eq!(
            store.publish_approval(&conflicting).await.unwrap(),
            Publication::Conflict
        );
        assert!(
            store
                .approval_for("request-a", "recipient-b")
                .await
                .unwrap()
                .is_none()
        );
        drop(store);
        let reopened = SqliteStore::open(&file).unwrap();
        assert_eq!(
            reopened
                .approval_for("request-a", "recipient-a")
                .await
                .unwrap(),
            Some(original.clone())
        );
        assert_eq!(
            reopened.publish_approval(&original).await.unwrap(),
            Publication::Existing
        );
        let count: u64 = reopened
            .0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM connection_delivery", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 1,
            "polls and conflicts never create partial or pending rows"
        );
    }
}
