//! Offline additions to the recipient and account fixed by an initial approval.
use super::*;

pub const PUBLISH_ADDITION_PATH: &str = "/connection/addition";
pub const READ_ADDITIONS_PATH: &str = "/connection/additions/read";

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct NewAddition {
    pub delivery_id: String,
    pub request_hash: String,
    pub recipient: String,
    pub account: String,
    pub addition_hex: String,
    pub created_at: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct StoredAddition {
    pub sequence: u64,
    #[serde(flatten)]
    pub addition: NewAddition,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdditionPublication {
    Created,
    Existing,
    Conflict,
    Capacity,
    UnknownMailbox,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch="wasm32",async_trait(?Send))]
pub trait AdditionStore {
    async fn publish_addition(
        &self,
        addition: &NewAddition,
    ) -> Result<AdditionPublication, StoreError>;
    /// Return at most one complete signed selection, bounding response size.
    async fn next_addition(
        &self,
        request_hash: &str,
        recipient: &str,
        after: u64,
    ) -> Result<Option<StoredAddition>, StoreError>;
    async fn addition_by_id(
        &self,
        id: &str,
        request_hash: &str,
        recipient: &str,
    ) -> Result<Option<StoredAddition>, StoreError>;
}

pub(crate) const INSERT: &str = "INSERT INTO connection_addition
    (delivery_id,request_hash,recipient,account,addition_hex,created_at,payload_size)
    SELECT ?1,?2,?3,?4,?5,?6,?7 FROM connection_delivery
    WHERE request_hash=?2 AND recipient=?3 AND account=?4
    AND (SELECT COUNT(*) FROM connection_addition WHERE account=?4)<1024
    AND (SELECT COALESCE(SUM(payload_size),0) FROM connection_addition
        WHERE account=?4)+?7<=134217728
    ON CONFLICT(delivery_id) DO NOTHING";
pub(crate) const SELECT_ID: &str =
    "SELECT sequence,delivery_id,request_hash,recipient,account,addition_hex,created_at
    FROM connection_addition WHERE delivery_id=?1";
pub(crate) const SELECT_NEXT: &str = "SELECT sequence,delivery_id,request_hash,recipient,account,addition_hex,created_at
    FROM connection_addition WHERE request_hash=?1 AND recipient=?2 AND sequence>?3 ORDER BY sequence LIMIT 1";
pub(crate) fn valid_shape(addition: &NewAddition) -> Result<(), StoreError> {
    if addition.delivery_id.len() > 128
        || addition.request_hash.len() > 128
        || addition.recipient.len() > 256
        || addition.account.len() > 256
        || addition.addition_hex.len() > MAX_APPROVAL_BYTES * 2
        || addition.created_at > 9_007_199_254_740_991
    {
        return Err(StoreError::Internal(
            "addition exceeds storage bounds".into(),
        ));
    }
    Ok(())
}

pub async fn execute<S, I>(
    store: &S,
    index: &I,
    service: &dialog_varsig::Did,
    path: &str,
    body: &[u8],
    now: u64,
) -> DeliveryResponse
where
    S: crate::store::Store + DeliveryStore + AdditionStore,
    I: crate::revocation::index::RevocationIndex + dialog_common::ConditionalSync,
{
    use dialog_credentials::DidKeyResolver;
    use dialog_ucan_core::{Environment, VerificationContext};
    use tonk_invite::terminal::{Addition, Approval, ReadAdditions};
    if path == READ_ADDITIONS_PATH {
        if body.len() > MAX_READ_BYTES {
            return DeliveryResponse::error(413, "read request too large");
        }
        let read = match ReadAdditions::validate(body, now).await {
            Ok(r) => r,
            Err(_) => return DeliveryResponse::error(401, "invalid signed addition read"),
        };
        return match store
            .next_addition(read.request_id(), read.recipient().as_str(), read.after())
            .await
        {
            Ok(row) => {
                let next = row.as_ref().map(|r| r.sequence).unwrap_or(read.after());
                let deliveries:Vec<_>=row.into_iter().map(|r|serde_json::json!({"sequence":r.sequence,"bytes":r.addition.addition_hex})).collect();
                DeliveryResponse {
                    status: 200,
                    content_type: "application/json",
                    body: serde_json::to_vec(
                        &serde_json::json!({"deliveries":deliveries,"nextCursor":next}),
                    )
                    .expect("bounded delivery JSON"),
                }
            }
            Err(_) => DeliveryResponse::error(503, "delivery unavailable"),
        };
    }
    if path != PUBLISH_ADDITION_PATH {
        return DeliveryResponse::error(404, "not found");
    }
    if body.len() > MAX_APPROVAL_BYTES {
        return DeliveryResponse::error(413, "addition too large");
    }
    let addition = match Addition::validate(body, now).await {
        Ok(a) => a,
        Err(_) => return DeliveryResponse::error(401, "invalid signed addition"),
    };
    if addition.service() != service {
        return DeliveryResponse::error(401, "addition addresses another service");
    }
    let original = match store
        .approval_for(addition.request_id(), addition.recipient().as_str())
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return DeliveryResponse::error(404, "terminal mailbox not found"),
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    };
    if original.account != addition.account().as_str() {
        return DeliveryResponse::error(403, "addition does not match approving account");
    }
    let initial = match hex::decode(&original.approval_hex) {
        Ok(b) => b,
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    };
    let initial = match Approval::inspect(&initial).await {
        Ok(a) => a,
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    };
    if initial.is_declined()
        || initial.account() != addition.account()
        || initial.request().recipient() != addition.recipient()
        || initial.request().id() != addition.request_id()
        || initial.request().service() != service
    {
        return DeliveryResponse::error(
            403,
            "initial decision does not authorize this delivery address",
        );
    }
    let id = addition.id();
    let encoded = hex::encode(body);
    match store
        .addition_by_id(&id, addition.request_id(), addition.recipient().as_str())
        .await
    {
        Ok(Some(row)) if row.addition.addition_hex == encoded => return receipt(&id, false),
        Ok(Some(_)) => return DeliveryResponse::error(409, "delivery has different bytes"),
        Ok(None) => {}
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    }
    match store.customer(addition.account().as_str()).await {
        Ok(Some(customer))
            if matches!(
                customer.status,
                tonk_account::customer::CustomerStatus::Active
            ) => {}
        Ok(_) => return DeliveryResponse::error(403, "approving account is not active"),
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    }
    let chain = addition.authorization();
    let checker = crate::revocation::checker::IndexedRevocations(index);
    let environment = Environment::new(chain.proof_store(), DidKeyResolver, &checker);
    if let Err(error) = chain.verify(&VerificationContext::new(&environment)).await {
        return match error {
            dialog_ucan_core::ContainerError::Revoked { .. } => {
                DeliveryResponse::error(401, "approving authority withdrawn")
            }
            _ => DeliveryResponse::error(503, "approval authority unavailable"),
        };
    }
    let row = NewAddition {
        delivery_id: id.clone(),
        request_hash: addition.request_id().into(),
        recipient: addition.recipient().to_string(),
        account: addition.account().to_string(),
        addition_hex: encoded,
        created_at: addition.issued_at(),
    };
    match store.publish_addition(&row).await {
        Ok(AdditionPublication::Created) => receipt(&id, true),
        Ok(AdditionPublication::Existing) => receipt(&id, false),
        Ok(AdditionPublication::Conflict) => {
            DeliveryResponse::error(409, "delivery has different bytes")
        }
        Ok(AdditionPublication::UnknownMailbox) => {
            DeliveryResponse::error(404, "terminal mailbox not found")
        }
        Ok(AdditionPublication::Capacity) => {
            DeliveryResponse::error(429, "account delivery capacity reached")
        }
        Err(_) => DeliveryResponse::error(503, "delivery unavailable"),
    }
}
fn receipt(id: &str, recorded: bool) -> DeliveryResponse {
    DeliveryResponse {
        status: if recorded { 201 } else { 200 },
        content_type: "application/json",
        body: serde_json::to_vec(&serde_json::json!({"deliveryId":id,"recorded":recorded}))
            .expect("receipt JSON"),
    }
}
