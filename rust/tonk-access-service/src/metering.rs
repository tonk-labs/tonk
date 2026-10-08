//! Metering collection: turn a presented container into an invocation
//! record for the ingest database.
//!
//! Pure parsing, shared by the worker's hot path and the native helpers
//! server. Metering is authorization-based: an issued permit is billed
//! whether or not the client uses it, and denied invocations are
//! recorded too, since a client retrying against a blocked consumer
//! still costs invocations.

use dialog_ucan_core::{Container, Delegation, Invocation};
use dialog_varsig::AnySignature;

use crate::store::ingest::InvocationRecord;

/// Parse `container_bytes` into a record. `None` when the container does
/// not parse: there is no consumer to attribute, and the authorizer
/// refused it without spending anything attributable.
pub fn collect(
    container_bytes: &[u8],
    outcome: &'static str,
    reason: Option<String>,
    bytes: u64,
    now: u64,
) -> Option<InvocationRecord> {
    let tokens = Container::from_bytes(container_bytes).ok()?.into_tokens();
    let mut tokens = tokens.into_iter();
    let body = tokens.next()?;
    let invocation: Invocation<AnySignature> = serde_ipld_dagcbor::from_slice(&body).ok()?;

    let mut proofs = Vec::new();
    for token in tokens {
        let delegation: Delegation<AnySignature> = serde_ipld_dagcbor::from_slice(&token).ok()?;
        proofs.push((delegation.to_cid().to_string(), token));
    }
    // The flattened set's identity: a hash over its sorted members, so
    // the same presented chain lands on the same rows regardless of
    // token order.
    let mut cids: Vec<&str> = proofs.iter().map(|(cid, _)| cid.as_str()).collect();
    cids.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    for cid in cids {
        hasher.update(cid.as_bytes());
        hasher.update(b"\n");
    }
    let chain = hasher.finalize().to_hex().to_string();

    Some(InvocationRecord {
        ts: now,
        cid: invocation.to_cid().to_string(),
        consumer: invocation.subject().to_string(),
        issuer: invocation.issuer().to_string(),
        cmd: format!("/{}", invocation.command().0.join("/")),
        outcome,
        reason,
        bytes,
        chain,
        body,
        proofs,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    async fn it_collects_a_record_with_a_stable_chain_identity() {
        use dialog_credentials::Ed25519Signer;
        use dialog_ucan_core::time::timestamp::Timestamp;
        use dialog_ucan_core::{DelegationBuilder, InvocationBuilder, InvocationChain};
        use dialog_varsig::Principal;
        use std::collections::HashMap;

        let root = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(root.clone()))
            .audience(&device.did())
            .subject(dialog_ucan_core::subject::Subject::Specific(root.did()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let cid = delegation.to_cid();
        let invocation = InvocationBuilder::new()
            .issuer(dialog_credentials::Signer::from(device))
            .audience(&root.did())
            .subject(&root.did())
            .command(vec![
                "use".into(),
                "get".into(),
                "memory".into(),
                "cell".into(),
            ])
            .proofs(vec![cid])
            .expiration(Timestamp::five_minutes_from_now())
            .try_build()
            .await
            .unwrap();
        let container = InvocationChain::new(
            invocation,
            HashMap::from([(cid, std::sync::Arc::new(delegation))]),
        )
        .to_bytes()
        .unwrap();

        let record = collect(&container, "ok", None, 0, 1_755_000_000).unwrap();
        assert_eq!(record.cmd, "/use/get/memory/cell");
        assert_eq!(record.consumer, root.did().to_string());
        assert_eq!(record.proofs.len(), 1);
        let again = collect(&container, "ok", None, 0, 1_755_000_000).unwrap();
        assert_eq!(record.chain, again.chain);

        assert!(collect(b"not a container", "ok", None, 0, 0).is_none());
    }
}
