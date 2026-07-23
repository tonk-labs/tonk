//! Revocation screening for presented UCAN containers.
//!
//! After cryptographic authorization succeeds, the presign path checks
//! whether any credential in the presented chain belongs to a revoked
//! device: the CID of every delegation, the issuer DID of every
//! delegation, and the invocation's issuer DID are matched against the
//! account registry. The decision logic here is pure and natively
//! tested; D1 glue lives in d1 and is wasm-only.
//!
//! An entitlement lookup for billing later extends the registry trait
//! (or adds a sibling) — the collection and decision shapes here stay
//! as they are.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

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

/// How long a revocation verdict may be served from the per-isolate
/// cache. Short on purpose: this bounds how stale enforcement can be,
/// and the design accepts up to a minute of lag after a revoke.
pub const REVOCATION_TTL_MS: u64 = 60_000;

/// A cached per-key verdict.
#[derive(Debug, Clone, Copy)]
pub struct CachedVerdict {
    /// Whether the key matched a revoked device when last queried.
    pub revoked: bool,
    /// Absolute expiry, in the caller's millisecond clock.
    pub expires_at_ms: u64,
}

/// The result of probing the cache for a key set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheProbe {
    /// True if any key has a live cached `revoked = true` verdict.
    pub cached_revoked: bool,
    /// Keys with no live cached verdict, needing a registry query.
    pub misses: Vec<String>,
}

/// Probe `map` for `keys` at `now_ms`, evicting expired entries.
pub fn split_with(
    map: &mut HashMap<String, CachedVerdict>,
    keys: &[String],
    now_ms: u64,
) -> CacheProbe {
    map.retain(|_, verdict| verdict.expires_at_ms > now_ms);
    let mut cached_revoked = false;
    let mut misses = Vec::new();
    for key in keys {
        match map.get(key) {
            Some(verdict) => cached_revoked |= verdict.revoked,
            None => misses.push(key.clone()),
        }
    }
    CacheProbe {
        cached_revoked,
        misses,
    }
}

/// Record fresh verdicts: every `queried` key gets an entry, `true` for
/// those in `revoked`.
pub fn store_with(
    map: &mut HashMap<String, CachedVerdict>,
    queried: &[String],
    revoked: &[String],
    now_ms: u64,
) {
    let expires_at_ms = now_ms + REVOCATION_TTL_MS;
    for key in queried {
        map.insert(
            key.clone(),
            CachedVerdict {
                revoked: revoked.contains(key),
                expires_at_ms,
            },
        );
    }
}

thread_local! {
    static VERDICTS: RefCell<HashMap<String, CachedVerdict>> = RefCell::new(HashMap::new());
}

/// Probe the per-isolate verdict cache.
pub fn cache_probe(keys: &[String], now_ms: u64) -> CacheProbe {
    VERDICTS.with(|cell| split_with(&mut cell.borrow_mut(), keys, now_ms))
}

/// Record fresh verdicts in the per-isolate cache.
pub fn cache_record(queried: &[String], revoked: &[String], now_ms: u64) {
    VERDICTS.with(|cell| store_with(&mut cell.borrow_mut(), queried, revoked, now_ms));
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
    const DEVICE2_SEED: [u8; 32] = [9u8; 32];

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

    /// A container shaped like a two-hop redelegation chain: one
    /// subject-open `root → device1` delegation (A), one subject-open
    /// `device1 → device2` delegation (B), invocation issued by device2
    /// with both delegations as proofs. Returns (delegation A cid,
    /// delegation B cid, root did, device1 did, device2 did, bytes).
    async fn two_hop_device_container() -> (String, String, String, String, String, Vec<u8>) {
        let root = Ed25519Signer::import(&ROOT_SEED).await.unwrap();
        let device1 = Ed25519Signer::import(&DEVICE_SEED).await.unwrap();
        let device2 = Ed25519Signer::import(&DEVICE2_SEED).await.unwrap();
        let root_did = root.did();
        let device1_did = device1.did();
        let device2_did = device2.did();

        let delegation_a = DelegationBuilder::new()
            .issuer(root.clone())
            .audience(&device1_did)
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let cid_a = delegation_a.to_cid();

        let delegation_b = DelegationBuilder::new()
            .issuer(device1.clone())
            .audience(&device2_did)
            .subject(Subject::Any)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let cid_b = delegation_b.to_cid();

        let invocation = InvocationBuilder::new()
            .issuer(device2.clone())
            .audience(&root_did)
            .subject(&root_did)
            .command(vec!["memory".to_string(), "resolve".to_string()])
            .arguments(BTreeMap::new())
            .proofs(vec![cid_a, cid_b])
            .try_build()
            .await
            .unwrap();

        let mut proofs = HashMap::new();
        proofs.insert(cid_a, Arc::new(delegation_a));
        proofs.insert(cid_b, Arc::new(delegation_b));
        let bytes = InvocationChain::new(invocation, proofs).to_bytes().unwrap();
        (
            cid_a.to_string(),
            cid_b.to_string(),
            root_did.to_string(),
            device1_did.to_string(),
            device2_did.to_string(),
            bytes,
        )
    }

    #[dialog_common::test]
    async fn it_collects_every_hop_of_a_multi_delegation_chain() {
        let (cid_a, cid_b, root_did, device1_did, device2_did, bytes) =
            two_hop_device_container().await;

        let presented = collect_presented(&bytes).unwrap();

        assert_eq!(presented.invocation_issuer, device2_did);
        assert!(presented.delegation_cids.contains(&cid_a));
        assert!(presented.delegation_cids.contains(&cid_b));
        assert!(presented.delegation_issuers.contains(&root_did));
        assert!(presented.delegation_issuers.contains(&device1_did));

        let keys = presented.keys();
        assert!(keys.contains(&cid_a));
        assert!(keys.contains(&cid_b));
        assert!(keys.contains(&root_did));
        assert!(keys.contains(&device1_did));
        assert!(keys.contains(&device2_did));
    }

    #[dialog_common::test]
    async fn it_splits_unseen_keys_into_misses() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string(), "b".to_string()];

        let probe = split_with(&mut map, &keys, 1_000);

        assert!(!probe.cached_revoked);
        assert_eq!(probe.misses, keys);
    }

    #[dialog_common::test]
    async fn it_serves_cached_verdicts_within_ttl() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string(), "b".to_string()];
        store_with(&mut map, &keys, &["b".to_string()], 1_000);

        let probe = split_with(&mut map, &keys, 1_000 + REVOCATION_TTL_MS - 1);

        assert!(probe.cached_revoked, "b is cached revoked");
        assert!(probe.misses.is_empty());
    }

    #[dialog_common::test]
    async fn it_expires_cached_verdicts_after_ttl() {
        let mut map = HashMap::new();
        let keys = vec!["a".to_string()];
        store_with(&mut map, &keys, &[], 1_000);

        let probe = split_with(&mut map, &keys, 1_000 + REVOCATION_TTL_MS + 1);

        assert!(!probe.cached_revoked);
        assert_eq!(probe.misses, keys);
    }
}
