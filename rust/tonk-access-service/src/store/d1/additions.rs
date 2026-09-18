use super::*;
use crate::delivery::additions::*;
use crate::delivery::chunks::{Kind, reference};
#[async_trait(?Send)]
impl AdditionStore for D1Store {
    async fn publish_addition(&self, a: &NewAddition) -> Result<AdditionPublication, StoreError> {
        valid_shape(a)?;
        let marker = reference(&a.addition_hex)?;
        let first = self
            .0
            .prepare(INSERT)
            .bind(&[
                JsValue::from_str(&a.delivery_id),
                JsValue::from_str(&a.request_hash),
                JsValue::from_str(&a.recipient),
                JsValue::from_str(&a.account),
                JsValue::from_str(&marker),
                JsValue::from_f64(a.created_at as f64),
                JsValue::from_f64(a.addition_hex.len() as f64),
            ])
            .map_err(map_err)?;
        if super::chunks::publish(
            &self.0,
            first,
            Kind::Addition,
            &a.delivery_id,
            &marker,
            &a.addition_hex,
        )
        .await?
        {
            return Ok(AdditionPublication::Created);
        }
        let stored: Option<StoredAddition> = self
            .0
            .prepare(SELECT_ID)
            .bind(&[JsValue::from_str(&a.delivery_id)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        if let Some(stored) = hydrate(&self.0, stored).await? {
            return Ok(if stored.addition == *a {
                AdditionPublication::Existing
            } else {
                AdditionPublication::Conflict
            });
        }
        let mailbox:Option<String>=self.0.prepare("SELECT request_hash FROM connection_delivery WHERE request_hash=?1 AND recipient=?2 AND account=?3")
            .bind(&[JsValue::from_str(&a.request_hash),JsValue::from_str(&a.recipient),JsValue::from_str(&a.account)]).map_err(map_err)?.first(Some("request_hash")).await.map_err(map_err)?;
        Ok(if mailbox.is_some() {
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
        let stored = self
            .0
            .prepare(SELECT_NEXT)
            .bind(&[
                JsValue::from_str(request_hash),
                JsValue::from_str(recipient),
                JsValue::from_f64(after as f64),
            ])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        hydrate(&self.0, stored).await
    }
    async fn addition_by_id(
        &self,
        id: &str,
        request_hash: &str,
        recipient: &str,
    ) -> Result<Option<StoredAddition>, StoreError> {
        let stored: Option<StoredAddition> = self
            .0
            .prepare(SELECT_ID)
            .bind(&[JsValue::from_str(id)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        hydrate(
            &self.0,
            stored.filter(|r| {
                r.addition.request_hash == request_hash && r.addition.recipient == recipient
            }),
        )
        .await
    }
}

async fn hydrate(
    db: &D1Database,
    stored: Option<StoredAddition>,
) -> Result<Option<StoredAddition>, StoreError> {
    match stored {
        Some(mut row) => {
            row.addition.addition_hex = super::chunks::load(
                db,
                Kind::Addition,
                &row.addition.delivery_id,
                row.addition.addition_hex,
            )
            .await?;
            Ok(Some(row))
        }
        None => Ok(None),
    }
}
