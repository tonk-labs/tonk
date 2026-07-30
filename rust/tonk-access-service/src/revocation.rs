//! Monotone revocation-set screening for presented UCAN containers.
//!
//! After cryptographic authorization succeeds, the presign path extracts every
//! delegation CID and screens it against a replicated set derived exclusively
//! from verified immutable artifacts. A refresh is authoritative only after a
//! complete listing, fetch, and verification pass succeeds.

#[cfg(target_arch = "wasm32")]
pub mod r2;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::fmt;

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::algorithm::eddsa::Ed25519Signature;

/// Credential CIDs and validity window presented to the presign endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCredentials {
    /// CIDs of every referenced or carried delegation.
    pub delegation_cids: Vec<String>,
    /// Latest start bound in unix seconds.
    pub not_before: Option<u64>,
    /// Earliest expiration bound in unix seconds.
    pub expires_at: Option<u64>,
}

/// Parse a UCAN container once for both expiry and revocation screens.
#[cfg_attr(test, allow(dead_code))]
pub fn collect_presented(container_bytes: &[u8]) -> Result<PresentedCredentials, ContainerError> {
    let tokens = Container::from_bytes(container_bytes)?.into_tokens();
    let Some(invocation_bytes) = tokens.first() else {
        return Err(ContainerError::Invocation(
            "container must contain at least an invocation".to_string(),
        ));
    };
    let invocation: Invocation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(invocation_bytes)
        .map_err(|error| {
            ContainerError::Invocation(format!("failed to decode invocation: {error}"))
        })?;
    let mut delegation_cids = BTreeSet::new();
    delegation_cids.extend(invocation.proofs().iter().map(ToString::to_string));
    let mut not_before: Option<u64> = None;
    let mut expires_at = invocation.expiration().map(|stamp| stamp.to_unix());
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(bytes)
            .map_err(|error| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {error}"))
            })?;
        delegation_cids.insert(delegation.to_cid().to_string());
        if let Some(stamp) = delegation.not_before() {
            not_before = Some(not_before.map_or(stamp.to_unix(), |seen| seen.max(stamp.to_unix())));
        }
        if let Some(stamp) = delegation.expiration() {
            expires_at = Some(expires_at.map_or(stamp.to_unix(), |seen| seen.min(stamp.to_unix())));
        }
    }
    Ok(PresentedCredentials {
        delegation_cids: delegation_cids.into_iter().collect(),
        not_before,
        expires_at,
    })
}

/// Fresh complete snapshots are reused for one minute.
pub const REVOCATION_TTL_MS: u64 = 60_000;
/// A previously complete clean snapshot may cover ten additional minutes.
pub const REVOCATION_GRACE_MS: u64 = 600_000;

/// One object returned by a complete source refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredArtifact {
    /// Full immutable object key.
    pub key: String,
    /// Artifact bytes.
    pub bytes: Vec<u8>,
}

/// Monotone, isolate-local view of verified revocations.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RevocationSnapshot {
    /// Every verified revoked target CID ever observed.
    pub revoked: HashSet<String>,
    /// Every verified artifact CID ever observed.
    pub seen_artifacts: HashSet<String>,
    /// Time of the last fully successful refresh.
    pub refreshed_at_ms: Option<u64>,
}

/// Source lookup failure.
#[derive(Debug, Clone)]
pub struct SourceError(pub String);

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Complete immutable artifact source.
pub trait RevocationSource {
    /// List every object and fetch bytes for artifacts absent from `seen`.
    /// Success means the listing was complete through its final page.
    async fn complete_listing(
        &self,
        seen: &HashSet<String>,
    ) -> Result<Vec<StoredArtifact>, SourceError>;
}

/// Revocation-set decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetVerdict {
    /// No presented CID is revoked and the snapshot is fresh.
    Allowed,
    /// Refresh failed, but a prior complete clean snapshot is inside grace.
    AllowedStale(String),
    /// A presented delegation CID is known revoked.
    Revoked,
    /// No complete snapshot can safely clear the request.
    Unavailable(String),
}

fn key_parts(key: &str) -> Result<(&str, &str), SourceError> {
    let mut parts = key.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("revocations"), Some(target), Some(artifact), None)
            if !target.is_empty() && !artifact.is_empty() =>
        {
            Ok((target, artifact))
        }
        _ => Err(SourceError(format!(
            "malformed revocation object key: {key}"
        ))),
    }
}

fn presented_revoked(snapshot: &RevocationSnapshot, cids: &[String]) -> bool {
    cids.iter().any(|cid| snapshot.revoked.contains(cid))
}

fn failed_verdict(
    snapshot: &RevocationSnapshot,
    cids: &[String],
    now_ms: u64,
    reason: String,
) -> SetVerdict {
    if presented_revoked(snapshot, cids) {
        return SetVerdict::Revoked;
    }
    match snapshot.refreshed_at_ms {
        Some(refreshed)
            if now_ms
                <= refreshed
                    .saturating_add(REVOCATION_TTL_MS)
                    .saturating_add(REVOCATION_GRACE_MS) =>
        {
            SetVerdict::AllowedStale(reason)
        }
        _ => SetVerdict::Unavailable(reason),
    }
}

/// Screen CIDs through a caller-owned snapshot and complete source.
pub async fn assess_with<S: RevocationSource>(
    snapshot: &mut RevocationSnapshot,
    source: &S,
    cids: &[String],
    now_ms: u64,
) -> SetVerdict {
    if presented_revoked(snapshot, cids) {
        return SetVerdict::Revoked;
    }
    if snapshot
        .refreshed_at_ms
        .is_some_and(|at| now_ms < at.saturating_add(REVOCATION_TTL_MS))
    {
        return SetVerdict::Allowed;
    }
    let listed = match source.complete_listing(&snapshot.seen_artifacts).await {
        Ok(listed) => listed,
        Err(error) => return failed_verdict(snapshot, cids, now_ms, error.to_string()),
    };
    for stored in listed {
        let (key_target, key_artifact) = match key_parts(&stored.key) {
            Ok(parts) => parts,
            Err(error) => return failed_verdict(snapshot, cids, now_ms, error.to_string()),
        };
        let verified = match tonk_identity::revocation::verify(&stored.bytes).await {
            Ok(verified) => verified,
            Err(error) => return failed_verdict(snapshot, cids, now_ms, error.to_string()),
        };
        if verified.target_cid != key_target || verified.artifact_cid != key_artifact {
            return failed_verdict(
                snapshot,
                cids,
                now_ms,
                format!(
                    "revocation object key does not match verified content: {}",
                    stored.key
                ),
            );
        }
        snapshot.revoked.insert(verified.target_cid.to_string());
        snapshot
            .seen_artifacts
            .insert(verified.artifact_cid.to_string());
    }
    snapshot.refreshed_at_ms = Some(now_ms);
    if presented_revoked(snapshot, cids) {
        SetVerdict::Revoked
    } else {
        SetVerdict::Allowed
    }
}

thread_local! {
    static SNAPSHOT: RefCell<RevocationSnapshot> = RefCell::new(RevocationSnapshot::default());
}

/// Screen through the isolate-local production snapshot.
#[cfg_attr(test, allow(dead_code))]
pub async fn assess<S: RevocationSource>(
    source: &S,
    presented: &PresentedCredentials,
    now_ms: u64,
) -> SetVerdict {
    // Avoid holding a RefCell borrow over await.
    let mut snapshot = SNAPSHOT.with(|cell| cell.borrow().clone());
    let verdict = assess_with(&mut snapshot, source, &presented.delegation_cids, now_ms).await;
    SNAPSHOT.with(|cell| {
        let mut current = cell.borrow_mut();
        current.revoked.extend(snapshot.revoked);
        current.seen_artifacts.extend(snapshot.seen_artifacts);
        current.refreshed_at_ms = match (current.refreshed_at_ms, snapshot.refreshed_at_ms) {
            (Some(current), Some(incoming)) => Some(current.max(incoming)),
            (current, incoming) => current.or(incoming),
        };
    });
    verdict
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_varsig::Principal;
    use std::sync::Mutex;

    struct ScriptedSource(Mutex<Result<Vec<StoredArtifact>, SourceError>>);

    impl RevocationSource for ScriptedSource {
        async fn complete_listing(
            &self,
            _seen: &HashSet<String>,
        ) -> Result<Vec<StoredArtifact>, SourceError> {
            self.0.lock().unwrap().clone()
        }
    }

    async fn artifact() -> (String, String, Vec<u8>) {
        let root = Ed25519Signer::import(&[41u8; 32]).await.unwrap();
        let device = Ed25519Signer::import(&[42u8; 32]).await.unwrap();
        let grant = tonk_identity::delegation::mint_device_delegation(root.clone(), &device.did())
            .await
            .unwrap();
        let target = grant.proof_cids()[0];
        let bytes = tonk_identity::revocation::mint_root_revocation(root, &grant, &target)
            .await
            .unwrap();
        let verified = tonk_identity::revocation::verify(&bytes).await.unwrap();
        (
            verified.target_cid.to_string(),
            verified.artifact_cid.to_string(),
            bytes,
        )
    }

    #[dialog_common::test]
    async fn it_unions_verified_target_cids_without_removing_old_entries() {
        let (target, artifact_cid, bytes) = artifact().await;
        let source = ScriptedSource(Mutex::new(Ok(vec![StoredArtifact {
            key: format!("revocations/{target}/{artifact_cid}"),
            bytes,
        }])));
        let mut snapshot = RevocationSnapshot::default();
        snapshot.revoked.insert("old".into());
        assert_eq!(
            assess_with(&mut snapshot, &source, std::slice::from_ref(&target), 1).await,
            SetVerdict::Revoked
        );
        assert!(snapshot.revoked.contains("old"));
        assert!(snapshot.revoked.contains(&target));
    }

    #[dialog_common::test]
    async fn it_does_not_advance_freshness_after_an_invalid_artifact() {
        let source = ScriptedSource(Mutex::new(Ok(vec![StoredArtifact {
            key: "revocations/target/artifact".into(),
            bytes: b"invalid".to_vec(),
        }])));
        let mut snapshot = RevocationSnapshot::default();
        assert!(matches!(
            assess_with(&mut snapshot, &source, &["clean".into()], 5).await,
            SetVerdict::Unavailable(_)
        ));
        assert_eq!(snapshot.refreshed_at_ms, None);
    }

    #[dialog_common::test]
    async fn it_rejects_a_known_revoked_cid_even_during_an_outage() {
        let source = ScriptedSource(Mutex::new(Err(SourceError("outage".into()))));
        let mut snapshot = RevocationSnapshot::default();
        snapshot.revoked.insert("revoked".into());
        assert_eq!(
            assess_with(&mut snapshot, &source, &["revoked".into()], u64::MAX).await,
            SetVerdict::Revoked
        );
    }

    #[dialog_common::test]
    async fn it_serves_a_complete_stale_set_inside_the_grace_window() {
        let source = ScriptedSource(Mutex::new(Err(SourceError("outage".into()))));
        let mut snapshot = RevocationSnapshot {
            refreshed_at_ms: Some(1),
            ..Default::default()
        };
        assert!(matches!(
            assess_with(
                &mut snapshot,
                &source,
                &["clean".into()],
                REVOCATION_TTL_MS + 2
            )
            .await,
            SetVerdict::AllowedStale(_)
        ));
    }

    #[dialog_common::test]
    async fn it_fails_closed_without_a_complete_set_past_grace() {
        let source = ScriptedSource(Mutex::new(Err(SourceError("outage".into()))));
        let mut snapshot = RevocationSnapshot {
            refreshed_at_ms: Some(1),
            ..Default::default()
        };
        assert!(matches!(
            assess_with(
                &mut snapshot,
                &source,
                &["clean".into()],
                REVOCATION_TTL_MS + REVOCATION_GRACE_MS + 2
            )
            .await,
            SetVerdict::Unavailable(_)
        ));
    }
}
