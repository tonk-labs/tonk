//! Revocation screening for presented UCAN containers.
//!
//! A presented container is parsed once into the CIDs, issuers, and
//! validity bounds that the expiry and revocation screens both need.
//! The revocation screen then decides whether the chain rests on a
//! delegation that one of its own issuers withdrew, by asking the
//! revocation index.

pub mod index;

use std::collections::BTreeSet;

use dialog_ucan_core::container::{Container, ContainerError};
use dialog_ucan_core::delegation::Delegation;
use dialog_ucan_core::invocation::Invocation;
use dialog_varsig::AnySignature;

/// Credential CIDs and validity window presented to the presign endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCredentials {
    /// The invocation's subject — the space whose access the chain
    /// exercises. Carried so a refusal can name whose authority was
    /// withdrawn.
    pub subject: dialog_varsig::Did,
    /// CIDs of every referenced or carried delegation.
    pub delegation_cids: Vec<String>,
    /// Every principal that issued a delegation in this chain.
    ///
    /// A revocation applies here only when its subject is one of these:
    /// a principal who never issued into this chain held no authority
    /// over it, so their revocation was never about it. This is the
    /// spec's `delegators` set, with
    /// [ucan-wg/revocation#4](https://github.com/ucan-wg/revocation/pull/4)
    /// applied so the match binds on the revocation's subject rather
    /// than the invocation's issuer.
    pub delegators: BTreeSet<String>,
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
    let invocation: Invocation<AnySignature> = serde_ipld_dagcbor::from_slice(invocation_bytes)
        .map_err(|error| {
            ContainerError::Invocation(format!("failed to decode invocation: {error}"))
        })?;
    let mut delegation_cids = BTreeSet::new();
    let mut delegators = BTreeSet::new();
    delegation_cids.extend(invocation.proofs().iter().map(ToString::to_string));
    let mut not_before: Option<u64> = None;
    let mut expires_at = invocation.expiration().map(|stamp| stamp.to_unix());
    for (index, bytes) in tokens.iter().skip(1).enumerate() {
        let delegation: Delegation<AnySignature> =
            serde_ipld_dagcbor::from_slice(bytes).map_err(|error| {
                ContainerError::Invocation(format!("failed to decode delegation {index}: {error}"))
            })?;
        delegation_cids.insert(delegation.to_cid().to_string());
        delegators.insert(delegation.issuer().to_string());
        if let Some(stamp) = delegation.not_before() {
            not_before = Some(not_before.map_or(stamp.to_unix(), |seen| seen.max(stamp.to_unix())));
        }
        if let Some(stamp) = delegation.expiration() {
            expires_at = Some(expires_at.map_or(stamp.to_unix(), |seen| seen.min(stamp.to_unix())));
        }
    }
    Ok(PresentedCredentials {
        subject: invocation.subject().clone(),
        delegation_cids: delegation_cids.into_iter().collect(),
        delegators,
        not_before,
        expires_at,
    })
}

/// Whether a presented chain rests on a delegation someone with
/// authority over it withdrew.
///
/// The spec's rule, per delegation: look the CID up, and if any subject
/// that revoked it is among this chain's issuers, ignore that
/// delegation. We hold one chain rather than a set of candidate paths,
/// so ignoring the only path we were given is a refusal.
///
/// An index failure is not a verdict. It surfaces as an error for the
/// caller to answer as its own unavailability, rather than as a claim
/// that anything was revoked.
pub async fn screen_revoked<I: index::RevocationIndex + dialog_common::ConditionalSync>(
    revocations: &I,
    presented: &PresentedCredentials,
) -> Result<Option<String>, index::IndexError> {
    if presented.delegators.is_empty() {
        // A chain with no delegations is rooted directly in its subject,
        // so there is no proof to withdraw.
        return Ok(None);
    }
    for cid in &presented.delegation_cids {
        if revocations
            .revoked_by_any(cid, &presented.delegators)
            .await?
        {
            return Ok(Some(cid.clone()));
        }
    }
    Ok(None)
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::index::RevocationIndex as _;
    use super::*;

    /// A chain presenting `cids`, issued by `delegators`.
    fn presented(cids: &[&str], delegators: &[&str]) -> PresentedCredentials {
        PresentedCredentials {
            subject: "did:key:zSubject".parse().expect("test subject parses"),
            delegation_cids: cids.iter().map(|cid| (*cid).to_string()).collect(),
            delegators: delegators.iter().map(|did| (*did).to_string()).collect(),
            not_before: None,
            expires_at: None,
        }
    }

    #[dialog_common::test]
    async fn it_passes_a_chain_nothing_revoked() {
        let revocations = index::MemoryRevocationIndex::default();
        let chain = presented(&["bafyA", "bafyB"], &["did:key:zAlice"]);
        assert_eq!(screen_revoked(&revocations, &chain).await.unwrap(), None);
    }

    #[dialog_common::test]
    async fn it_rejects_a_chain_resting_on_a_withdrawn_delegation() {
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zAlice").await.unwrap();

        // Alice issued into this chain, so her revocation applies to it.
        let chain = presented(&["bafyA", "bafyB"], &["did:key:zAlice", "did:key:zBob"]);
        assert_eq!(
            screen_revoked(&revocations, &chain).await.unwrap(),
            Some("bafyB".to_string())
        );
    }

    #[dialog_common::test]
    async fn it_ignores_a_revocation_by_someone_outside_the_chain() {
        // The discriminating case. One target, one stored revocation,
        // two chains, two verdicts: `a` revoked `b`, so a chain `a` had
        // a hand in is refused, and a chain rooted elsewhere is not.
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zA").await.unwrap();

        let through_a = presented(&["bafyB"], &["did:key:zA", "did:key:zB"]);
        assert_eq!(
            screen_revoked(&revocations, &through_a).await.unwrap(),
            Some("bafyB".to_string())
        );

        let through_k = presented(&["bafyB"], &["did:key:zK", "did:key:zB"]);
        assert_eq!(
            screen_revoked(&revocations, &through_k).await.unwrap(),
            None,
            "a principal who never issued into this chain held no authority over it"
        );
    }

    #[dialog_common::test]
    async fn it_covers_every_delegation_the_chain_presents() {
        // Revoking the root of a chain refuses it just as revoking the
        // leaf does: the walk checks all of them, not only the last.
        let revocations = index::MemoryRevocationIndex::default();
        revocations
            .record("bafyRoot", "did:key:zAlice")
            .await
            .unwrap();

        let chain = presented(&["bafyRoot", "bafyLeaf"], &["did:key:zAlice"]);
        assert_eq!(
            screen_revoked(&revocations, &chain).await.unwrap(),
            Some("bafyRoot".to_string())
        );
    }

    #[dialog_common::test]
    async fn it_passes_a_chain_with_no_delegations_at_all() {
        // Rooted directly in its subject, so there is no proof to
        // withdraw and nothing to look up.
        let revocations = index::MemoryRevocationIndex::default();
        revocations.record("bafyB", "did:key:zA").await.unwrap();

        let direct = presented(&[], &[]);
        assert_eq!(screen_revoked(&revocations, &direct).await.unwrap(), None);
    }
}
