//! Immutable delivery of public signed connection approvals.
//!
//! Storage is deliberately separate from data authorization. Callers must
//! authenticate publication and exact-recipient reads before using this trait.

use crate::store::StoreError;

pub mod additions;
pub(crate) mod chunks;
use async_trait::async_trait;

pub const PUBLISH_PATH: &str = "/connection/delivery";
pub const READ_PATH: &str = "/connection/read";
pub const MAX_APPROVAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_READ_BYTES: usize = 4096;

/// HTTP-neutral bounded result, shared by native and Worker adapters.
pub struct DeliveryResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl DeliveryResponse {
    pub fn error(status: u16, message: &'static str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&serde_json::json!({"error": message})).expect("static JSON"),
        }
    }
}

/// Process an addressed delivery. No operation enumerates account contents or
/// consults these records to decide data access.
pub async fn execute<S, I>(
    store: &S,
    index: &I,
    service: &dialog_varsig::Did,
    path: &str,
    body: &[u8],
    now: u64,
) -> DeliveryResponse
where
    S: crate::store::Store + DeliveryStore + additions::AdditionStore,
    I: crate::revocation::index::RevocationIndex + dialog_common::ConditionalSync,
{
    if matches!(
        path,
        additions::PUBLISH_ADDITION_PATH | additions::READ_ADDITIONS_PATH
    ) {
        return additions::execute(store, index, service, path, body, now).await;
    }
    use dialog_credentials::DidKeyResolver;
    use dialog_ucan_core::{Environment, VerificationContext};
    use tonk_invite::terminal::{Approval, ReadRequest};
    if path == READ_PATH {
        if body.len() > MAX_READ_BYTES {
            return DeliveryResponse::error(413, "read request too large");
        }
        let read = match ReadRequest::validate(body, now).await {
            Ok(read) => read,
            Err(_) => return DeliveryResponse::error(401, "invalid signed read request"),
        };
        return match store
            .approval_for(read.request_id(), read.recipient().as_str())
            .await
        {
            Ok(Some(row)) => match hex::decode(row.approval_hex) {
                Ok(bytes) => DeliveryResponse {
                    status: 200,
                    content_type: "application/cbor",
                    body: bytes,
                },
                Err(_) => DeliveryResponse::error(500, "delivery unavailable"),
            },
            Ok(None) => DeliveryResponse {
                status: 204,
                content_type: "application/cbor",
                body: Vec::new(),
            },
            Err(_) => DeliveryResponse::error(503, "delivery unavailable"),
        };
    }
    if path != PUBLISH_PATH {
        return DeliveryResponse::error(404, "not found");
    }
    if body.len() > MAX_APPROVAL_BYTES {
        return DeliveryResponse::error(413, "approval too large");
    }
    let approval = match Approval::validate(body, now).await {
        Ok(approval) => approval,
        Err(_) => return DeliveryResponse::error(401, "invalid signed approval"),
    };
    if approval.request().service() != service {
        return DeliveryResponse::error(401, "approval addresses another service");
    }
    let request_id = approval.request().id();
    let encoded = hex::encode(body);
    // Returning an existing immutable fact requires no new write authority.
    match store
        .approval_for(&request_id, approval.request().recipient().as_str())
        .await
    {
        Ok(Some(row)) if row.approval_hex == encoded => {
            return publication_receipt(&request_id, false);
        }
        Ok(Some(_)) => {
            return DeliveryResponse::error(409, "request already has a different decision");
        }
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
        Ok(None) => {}
    }
    if now >= approval.request().deadline() {
        return DeliveryResponse::error(410, "approval request expired");
    }
    let customer = match store.customer(approval.account().as_str()).await {
        Ok(Some(customer)) => customer,
        Ok(None) => return DeliveryResponse::error(404, "approving account is not registered"),
        Err(_) => return DeliveryResponse::error(503, "delivery unavailable"),
    };
    if !matches!(
        customer.status,
        tonk_account::customer::CustomerStatus::Active
    ) {
        return DeliveryResponse::error(403, "approving account is not active");
    }
    // The codec binds this ordinary invocation to every payload byte, the
    // request's service audience, and the authenticated approving account.
    // Use the same chain verifier/revocation checker as ordinary UCAN access.
    let chain = approval.authorization();
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
    let row = StoredApproval {
        request_hash: request_id.clone(),
        recipient: approval.request().recipient().to_string(),
        account: approval.account().to_string(),
        approval_hex: encoded,
        created_at: approval.issued_at(),
        approval_deadline: approval.request().deadline(),
    };
    match store.publish_approval(&row).await {
        Ok(Publication::Created) => publication_receipt(&request_id, true),
        Ok(Publication::Existing) => publication_receipt(&request_id, false),
        Ok(Publication::Conflict) => {
            DeliveryResponse::error(409, "request already has a different decision")
        }
        Ok(Publication::Capacity) => {
            DeliveryResponse::error(429, "account delivery capacity reached")
        }
        Ok(Publication::AccountUnavailable) => {
            DeliveryResponse::error(403, "approving account is no longer active")
        }
        Err(_) => DeliveryResponse::error(503, "delivery unavailable"),
    }
}

fn publication_receipt(request_id: &str, recorded: bool) -> DeliveryResponse {
    DeliveryResponse {
        status: if recorded { 201 } else { 200 },
        content_type: "application/json",
        body: serde_json::to_vec(&serde_json::json!({"requestId":request_id,"recorded":recorded}))
            .expect("receipt JSON"),
    }
}

/// A complete authenticated approval, stored atomically with its address.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct StoredApproval {
    pub request_hash: String,
    pub recipient: String,
    pub account: String,
    pub approval_hex: String,
    pub created_at: u64,
    pub approval_deadline: u64,
}

/// Result of immutable publication. A conflict never changes existing bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    Created,
    Existing,
    Conflict,
    Capacity,
    AccountUnavailable,
}

/// Bounded public delivery storage; no pending-request or catalogue methods.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait DeliveryStore {
    async fn publish_approval(&self, approval: &StoredApproval) -> Result<Publication, StoreError>;
    async fn approval_for(
        &self,
        request_hash: &str,
        recipient: &str,
    ) -> Result<Option<StoredApproval>, StoreError>;
}

// A write statement is the atomic boundary in both SQLite and D1. The quota
// predicate is evaluated inside the write, not in a racy preliminary read.
pub(crate) const INSERT_APPROVAL: &str = "INSERT INTO connection_delivery
    (request_hash, recipient, account, approval_hex, created_at, approval_deadline, payload_size)
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
    WHERE EXISTS(SELECT 1 FROM customer WHERE account=?3 AND status='Active')
    AND (SELECT COUNT(*) FROM connection_delivery WHERE account = ?3) < 1024
    AND (SELECT COALESCE(SUM(payload_size), 0) FROM connection_delivery
        WHERE account = ?3) + ?7 <= 134217728
    ON CONFLICT(request_hash) DO NOTHING";
pub(crate) const SELECT_APPROVAL: &str = "SELECT request_hash, recipient, account,
    approval_hex, created_at, approval_deadline FROM connection_delivery
    WHERE request_hash = ?1 AND recipient = ?2";
pub(crate) const SELECT_BY_HASH: &str = "SELECT request_hash, recipient, account,
    approval_hex, created_at, approval_deadline FROM connection_delivery
    WHERE request_hash = ?1";

pub(crate) fn valid_storage_shape(approval: &StoredApproval) -> Result<(), StoreError> {
    if approval.request_hash.len() > 128
        || approval.recipient.len() > 256
        || approval.account.len() > 256
        || approval.approval_hex.len() > 8 * 1024 * 1024
        || approval.created_at > 9_007_199_254_740_991
        || approval.approval_deadline > 9_007_199_254_740_991
    {
        return Err(StoreError::Internal(
            "approval exceeds storage bounds".into(),
        ));
    }
    Ok(())
}
