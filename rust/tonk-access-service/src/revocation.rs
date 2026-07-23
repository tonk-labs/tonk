//! Revocation screening for presented UCAN containers.
//!
//! After cryptographic authorization succeeds, the presign path checks
//! whether any credential in the presented chain belongs to a revoked
//! device: the CID of every delegation, the issuer DID of every
//! delegation, and the invocation's issuer DID are matched against the
//! account registry. The decision logic here is pure and natively
//! tested; D1 glue lives in [`d1`] and is wasm-only.
//!
//! An entitlement lookup for billing later extends the registry trait
//! (or adds a sibling) — the collection and decision shapes here stay
//! as they are.

use std::collections::BTreeSet;

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::algorithm::eddsa::Ed25519Signature;

/// Every credential identity found in a presented UCAN container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCredentials {
    /// The DID that signed the invocation (the requesting operator).
    pub invocation_issuer: String,
    /// CIDs of every delegation: those referenced by the invocation's
    /// proof list and those carried as container tokens.
    pub delegation_cids: Vec<String>,
    /// Issuer DIDs of every delegation carried in the container.
    pub delegation_issuers: Vec<String>,
}

impl PresentedCredentials {
    /// The deduplicated set of registry lookup keys: delegation CIDs,
    /// delegation issuer DIDs, and the invocation issuer DID. CIDs and
    /// DIDs cannot collide (different prefixes), so one key space is
    /// safe.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: BTreeSet<String> = BTreeSet::new();
        keys.extend(self.delegation_cids.iter().cloned());
        keys.extend(self.delegation_issuers.iter().cloned());
        keys.insert(self.invocation_issuer.clone());
        keys.into_iter().collect()
    }
}

/// Parse a UCAN container and collect every presented credential
/// identity. Token 0 is the invocation; the remaining tokens are
/// delegations, exactly as `InvocationChain::try_from` consumes them.
pub fn collect_presented(container_bytes: &[u8]) -> Result<PresentedCredentials, ContainerError> {
    let tokens = Container::from_bytes(container_bytes)?.into_tokens();
    let Some(invocation_bytes) = tokens.first() else {
        return Err(ContainerError::Invocation(
            "container must contain at least an invocation".to_string(),
        ));
    };

    let invocation: Invocation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(invocation_bytes)
        .map_err(|err| ContainerError::Invocation(format!("failed to decode invocation: {err}")))?;

    let mut delegation_cids = BTreeSet::new();
    for cid in invocation.proofs() {
        delegation_cids.insert(cid.to_string());
    }

    let mut delegation_issuers = BTreeSet::new();
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(bytes)
            .map_err(|err| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {err}"))
            })?;
        delegation_cids.insert(delegation.to_cid().to_string());
        delegation_issuers.insert(delegation.issuer().to_string());
    }

    Ok(PresentedCredentials {
        invocation_issuer: invocation.issuer().to_string(),
        delegation_cids: delegation_cids.into_iter().collect(),
        delegation_issuers: delegation_issuers.into_iter().collect(),
    })
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Arc;

    use dialog_credentials::Ed25519Signer;
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, InvocationBuilder, InvocationChain};
    use dialog_varsig::Principal;

    const ROOT_SEED: [u8; 32] = [7u8; 32];
    const DEVICE_SEED: [u8; 32] = [8u8; 32];

    /// A container shaped like a linked device's presign request: one
    /// subject-open `root → device` delegation, invocation issued by the
    /// device. Returns (delegation cid, root did, device did, bytes).
    async fn device_container() -> (String, String, String, Vec<u8>) {
        let root = Ed25519Signer::import(&ROOT_SEED).await.unwrap();
        let device = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let root_did = root.did();

        let delegation = DelegationBuilder::new()
            .issuer(root.clone())
            .audience(&device.did())
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let cid = delegation.to_cid();

        let invocation = InvocationBuilder::new()
            .issuer(device.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(vec!["memory".to_string(), "resolve".to_string()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid])
            .try_build()
            .await
            .unwrap();

        let mut proofs = HashMap::new();
        proofs.insert(cid, Arc::new(delegation));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (
            cid.to_string(),
            root_did.to_string(),
            device.did().to_string(),
            bytes,
        )
    }

    #[dialog_common::test]
    async fn it_collects_cids_and_issuers_from_a_container() {
        let (cid, root_did, device_did, bytes) = device_container().await;

        let presented = collect_presented(&bytes).unwrap();

        assert_eq!(presented.invocation_issuer, device_did);
        assert!(presented.delegation_cids.contains(&cid));
        assert!(presented.delegation_issuers.contains(&root_did));
    }

    #[dialog_common::test]
    async fn it_unions_all_identities_into_the_key_set() {
        let (cid, root_did, device_did, bytes) = device_container().await;

        let keys = collect_presented(&bytes).unwrap().keys();

        assert!(keys.contains(&cid));
        assert!(keys.contains(&root_did));
        assert!(keys.contains(&device_did));
        let mut deduped = keys.clone();
        deduped.dedup();
        assert_eq!(deduped, keys, "keys must be deduplicated and sorted");
    }

    #[dialog_common::test]
    async fn it_rejects_an_empty_or_garbage_container() {
        assert!(collect_presented(&[]).is_err());
        assert!(collect_presented(&[0xde, 0xad, 0xbe, 0xef]).is_err());
    }
}
