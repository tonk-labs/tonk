use super::*;
use crate::delivery::chunks::{Kind, reference};
use crate::delivery::*;

#[async_trait(?Send)]
impl DeliveryStore for D1Store {
    async fn publish_approval(&self, approval: &StoredApproval) -> Result<Publication, StoreError> {
        valid_storage_shape(approval)?;
        let marker = reference(&approval.approval_hex)?;
        let first = self
            .0
            .prepare(INSERT_APPROVAL)
            .bind(&[
                JsValue::from_str(&approval.request_hash),
                JsValue::from_str(&approval.recipient),
                JsValue::from_str(&approval.account),
                JsValue::from_str(&marker),
                JsValue::from_f64(approval.created_at as f64),
                JsValue::from_f64(approval.approval_deadline as f64),
                JsValue::from_f64(approval.approval_hex.len() as f64),
            ])
            .map_err(map_err)?;
        if super::chunks::publish(
            &self.0,
            first,
            Kind::Initial,
            &approval.request_hash,
            &marker,
            &approval.approval_hex,
        )
        .await?
        {
            return Ok(Publication::Created);
        }
        let stored: Option<StoredApproval> = self
            .0
            .prepare(SELECT_BY_HASH)
            .bind(&[JsValue::from_str(&approval.request_hash)])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        let stored = match stored {
            Some(mut row) => {
                row.approval_hex = super::chunks::load(
                    &self.0,
                    Kind::Initial,
                    &row.request_hash,
                    row.approval_hex,
                )
                .await?;
                Some(row)
            }
            None => None,
        };
        Ok(match stored {
            Some(stored) if stored == *approval => Publication::Existing,
            Some(_) => Publication::Conflict,
            None => {
                let active: Option<String> = self
                    .0
                    .prepare("SELECT account FROM customer WHERE account=?1 AND status='Active'")
                    .bind(&[JsValue::from_str(&approval.account)])
                    .map_err(map_err)?
                    .first(Some("account"))
                    .await
                    .map_err(map_err)?;
                if active.is_some() {
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
        let stored: Option<StoredApproval> = self
            .0
            .prepare(SELECT_APPROVAL)
            .bind(&[
                JsValue::from_str(request_hash),
                JsValue::from_str(recipient),
            ])
            .map_err(map_err)?
            .first(None)
            .await
            .map_err(map_err)?;
        match stored {
            Some(mut row) => {
                row.approval_hex = super::chunks::load(
                    &self.0,
                    Kind::Initial,
                    &row.request_hash,
                    row.approval_hex,
                )
                .await?;
                Ok(Some(row))
            }
            None => Ok(None),
        }
    }
}
