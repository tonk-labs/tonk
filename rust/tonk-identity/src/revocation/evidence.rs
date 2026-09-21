//! Cached revocation evidence, shared by local verifiers and access services.
//!
//! Exact approvals are scoped to the trusted service, delegation and candidate
//! principals. Complete service target snapshots also establish absence for
//! fresh delegation CIDs, without bypassing any link in the proof chain.
//! A learned revocation is permanent and overrides every approval,
//! including approvals from another service or an older concurrent lookup.
//! This says nothing about signatures, scope, or token expiry: the normal UCAN
//! verifier must still check those on every invocation.

use std::collections::{BTreeMap, BTreeSet};

use dialog_varsig::Did;
use ipld_core::cid::Cid;
use serde::{Deserialize, Serialize};

/// Wire and cache format version.
pub const VERSION: u8 = 1;
/// Maximum lookup body accepted before decoding.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Bounded answer, including optional complete absence evidence.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Largest complete target inventory carried by one service response.
pub const MAX_SNAPSHOT_TARGETS: usize = 8192;
/// Maximum number of authorized revokers in one lookup.
pub const MAX_PRINCIPALS: usize = 64;
/// Approvals are expendable; revoked facts are never evicted.
const MAX_APPROVALS: usize = 4096;

/// The precise question Dialog's chain walk asks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// Protocol version.
    pub version: u8,
    /// Canonical delegation CID, not an invocation's nonce or subject.
    pub delegation: String,
    /// Canonical, sorted, distinct principals entitled to revoke this link.
    pub by: BTreeSet<String>,
}

/// An invalid or mismatched lookup must not warm the approval cache.
#[derive(Debug, thiserror::Error)]
#[error("invalid revocation evidence: {0}")]
pub struct Invalid(pub String);

impl Query {
    /// Make a canonical query from the verifier's selector.
    pub fn new(delegation: Cid, by: &[Did]) -> Result<Self, Invalid> {
        let query = Self {
            version: VERSION,
            delegation: delegation.to_string(),
            by: by.iter().map(ToString::to_string).collect(),
        };
        query.validate()?;
        Ok(query)
    }

    /// Refuse oversized, noncanonical or unsupported questions.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.version != VERSION || self.delegation.len() > 128 || self.by.len() > MAX_PRINCIPALS
        {
            return Err(Invalid("unsupported version or oversized selector".into()));
        }
        if self
            .delegation
            .parse::<Cid>()
            .ok()
            .map(|cid| cid.to_string())
            .as_ref()
            != Some(&self.delegation)
        {
            return Err(Invalid("delegation must be a canonical CID".into()));
        }
        for principal in &self.by {
            if principal.len() > 512
                || principal
                    .parse::<Did>()
                    .ok()
                    .map(|did| did.to_string())
                    .as_ref()
                    != Some(principal)
            {
                return Err(Invalid(
                    "candidate must be a canonical DID of at most 512 bytes".into(),
                ));
            }
        }
        Ok(())
    }
}

/// An answer from an explicitly configured, trusted access service.
///
/// The query is echoed to prevent a mismatched answer being cached. HTTP
/// adapters authenticate the service and must not follow untrusted redirects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    /// The selector answered by this response.
    pub query: Query,
    /// The subset of candidates recorded in the service's revocation index.
    pub revoked_by: BTreeSet<String>,
    /// Every delegation target in this service's revocation index. `None`
    /// means completeness could not be established, NEVER a truncated list.
    /// This contains only opaque CIDs, not principals or delegation bytes.
    /// Absence covers fresh session proofs too; presence still requires the
    /// exact candidate-principal check above and never grants authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<BTreeSet<String>>,
}

impl Answer {
    /// Validate both the echoed question and the authority of each revoker.
    pub fn validate_for(&self, query: &Query) -> Result<(), Invalid> {
        query.validate()?;
        if &self.query != query || !self.revoked_by.is_subset(&query.by) {
            return Err(Invalid(
                "response does not answer this exact selector".into(),
            ));
        }
        if let Some(targets) = &self.targets {
            if targets.len() > MAX_SNAPSHOT_TARGETS {
                return Err(Invalid("oversized target snapshot".into()));
            }
            validate_targets(targets)?;
            if !self.revoked_by.is_empty() && !targets.contains(&query.delegation) {
                return Err(Invalid("snapshot contradicts the revocation answer".into()));
            }
        }
        Ok(())
    }
}

fn validate_targets(targets: &BTreeSet<String>) -> Result<(), Invalid> {
    for delegation in targets {
        Query {
            version: VERSION,
            delegation: delegation.clone(),
            by: BTreeSet::new(),
        }
        .validate()?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    source: String,
    query: Query,
}

/// What locally retained evidence can establish without a network lookup.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// This source answered the exact question, or established that the target
    /// was absent from its complete index snapshot.
    Approved,
    /// A principal authorized for this link has withdrawn it.
    Revoked(String),
    /// No verified evidence exists; absence is not approval.
    Unknown,
}

/// Serializable, monotonic knowledge. Persist atomically after each update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    version: u8,
    approvals: BTreeSet<Approval>,
    revoked: BTreeMap<String, BTreeSet<String>>,
    /// Complete service inventories, unioned monotonically. A service cannot
    /// resurrect absence by serving an older snapshot after a newer one.
    #[serde(default)]
    snapshots: BTreeMap<String, BTreeSet<String>>,
}

impl Default for Evidence {
    fn default() -> Self {
        Self {
            version: VERSION,
            approvals: BTreeSet::new(),
            revoked: BTreeMap::new(),
            snapshots: BTreeMap::new(),
        }
    }
}

impl Evidence {
    /// Check persisted data before using it, failing closed on corrupt data.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.version != VERSION || self.approvals.len() > MAX_APPROVALS {
            return Err(Invalid("unsupported or oversized cache".into()));
        }
        for approval in &self.approvals {
            approval.query.validate()?;
        }
        for targets in self.snapshots.values() {
            validate_targets(targets)?;
        }
        for (delegation, principals) in &self.revoked {
            // The accumulated set may exceed one query's candidate limit.
            for principal in principals {
                Query {
                    version: VERSION,
                    delegation: delegation.clone(),
                    by: BTreeSet::from([principal.clone()]),
                }
                .validate()?;
            }
        }
        Ok(())
    }

    /// Return known revocations first, independent of positive cache entries.
    pub fn verdict(&self, source: &str, query: &Query) -> Verdict {
        if let Some(principals) = self.revoked.get(&query.delegation)
            && let Some(principal) = principals.intersection(&query.by).next()
        {
            return Verdict::Revoked(principal.clone());
        }
        if self.approvals.contains(&Approval {
            source: source.into(),
            query: query.clone(),
        }) || self
            .snapshots
            .get(source)
            .is_some_and(|targets| !targets.contains(&query.delegation))
        {
            Verdict::Approved
        } else {
            Verdict::Unknown
        }
    }

    /// Retain an authenticated service response. Callers must only invoke this
    /// after validating the service response, never for an unavailable lookup.
    pub fn record(&mut self, source: &str, query: &Query, answer: &Answer) -> Result<(), Invalid> {
        answer.validate_for(query)?;
        if let Some(targets) = &answer.targets {
            let known = self.snapshots.entry(source.into()).or_default();
            // A newly observed target invalidates old exact approvals until
            // a fresh candidate check establishes whether that revoker binds.
            self.approvals.retain(|approval| {
                approval.source != source
                    || !targets.contains(&approval.query.delegation)
                    || known.contains(&approval.query.delegation)
            });
            known.extend(targets.iter().cloned());
        }
        if answer.revoked_by.is_empty() {
            let known_target = self
                .snapshots
                .get(source)
                .is_some_and(|targets| targets.contains(&query.delegation));
            // An old snapshot cannot vouch for a newly observed target. Exact
            // scoped approval is usable only alongside a snapshot that knows
            // that target (or when we have never observed it in a snapshot).
            if !known_target
                || answer
                    .targets
                    .as_ref()
                    .is_some_and(|targets| targets.contains(&query.delegation))
            {
                self.approvals.insert(Approval {
                    source: source.into(),
                    query: query.clone(),
                });
            }
            while self.approvals.len() > MAX_APPROVALS {
                self.approvals.pop_first();
            }
        } else {
            self.revoked
                .entry(query.delegation.clone())
                .or_default()
                .extend(answer.revoked_by.iter().cloned());
            // Once withdrawn, approvals for this delegation can never help.
            self.approvals
                .retain(|approval| approval.query.delegation != query.delegation);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "bafyreidyasztqnjah3v2s5vr4sidcsgksbfvcgeegc4p37irqvps7a7jd4";
    const ALICE: &str = "did:key:z6MkrF2Jq3mNhFsEtYvQeTVZQfZ5fFPMj3DcbSt9uhzNcoVR";
    const BOB: &str = "did:key:z6MkuGiBdtP3ZdjU6H9fsvKJt6PJPMgpCsM9sYswpQvFQ3Pa";

    fn query(by: &[&str]) -> Query {
        Query::new(
            CID.parse().unwrap(),
            &by.iter()
                .map(|did| did.parse().unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn answer(query: &Query, revoked: &[&str]) -> Answer {
        Answer {
            query: query.clone(),
            revoked_by: revoked.iter().map(|s| s.to_string()).collect(),
            targets: None,
        }
    }

    #[test]
    fn complete_snapshot_covers_a_fresh_cid_and_fresh_session_principal() {
        let initial = query(&[ALICE]);
        let mut fresh = query(&[BOB]);
        fresh.delegation = Cid::new_v1(0x55, *CID.parse::<Cid>().unwrap().hash()).to_string();
        let mut evidence = Evidence::default();
        evidence
            .record("service", &initial, &answer(&initial, &[]))
            .unwrap();
        assert_eq!(
            evidence.verdict("service", &fresh),
            Verdict::Unknown,
            "an exact or partial answer cannot approve a different proof"
        );
        let mut complete = answer(&initial, &[]);
        complete.targets = Some(BTreeSet::new());
        evidence.record("service", &initial, &complete).unwrap();
        let restored: Evidence =
            serde_json::from_slice(&serde_json::to_vec(&evidence).unwrap()).unwrap();
        assert_eq!(restored.verdict("service", &fresh), Verdict::Approved);
        assert_eq!(restored.verdict("other service", &fresh), Verdict::Unknown);
    }

    #[test]
    fn learning_a_target_invalidates_old_approval_even_without_its_revoker() {
        let q = query(&[ALICE]);
        let mut unrelated = query(&[BOB]);
        unrelated.delegation = Cid::new_v1(0x55, *CID.parse::<Cid>().unwrap().hash()).to_string();
        let mut evidence = Evidence::default();
        let mut old = answer(&q, &[]);
        old.targets = Some(BTreeSet::new());
        evidence.record("service", &q, &old).unwrap();
        let mut observed = answer(&unrelated, &[]);
        observed.targets = Some(BTreeSet::from([q.delegation.clone()]));
        evidence.record("service", &unrelated, &observed).unwrap();
        assert_eq!(evidence.verdict("service", &q), Verdict::Unknown);
        evidence.record("service", &q, &old).unwrap();
        assert_eq!(
            evidence.verdict("service", &q),
            Verdict::Unknown,
            "a late old reply cannot resurrect absence"
        );
        let mut fresh_scoped = answer(&q, &[]);
        fresh_scoped.targets = observed.targets.clone();
        evidence.record("service", &q, &fresh_scoped).unwrap();
        assert_eq!(
            evidence.verdict("service", &q),
            Verdict::Approved,
            "a fresh exact check can establish the revoker is outside this chain"
        );
        let mut withdrawn = answer(&q, &[ALICE]);
        withdrawn.targets = observed.targets;
        evidence.record("service", &q, &withdrawn).unwrap();
        evidence.record("service", &q, &fresh_scoped).unwrap();
        assert_eq!(
            evidence.verdict("service", &q),
            Verdict::Revoked(ALICE.into())
        );
    }

    #[test]
    fn inconsistent_snapshot_never_becomes_absence_evidence() {
        let q = query(&[ALICE]);
        let mut inconsistent = answer(&q, &[ALICE]);
        inconsistent.targets = Some(BTreeSet::new());
        assert!(
            Evidence::default()
                .record("service", &q, &inconsistent)
                .is_err()
        );
        inconsistent = answer(&q, &[]);
        inconsistent.targets = Some(BTreeSet::from(["not a CID".into()]));
        assert!(
            Evidence::default()
                .record("service", &q, &inconsistent)
                .is_err()
        );
    }

    #[test]
    fn approval_is_exact_and_survives_restart_without_a_cache_ttl() {
        let q = query(&[ALICE]);
        let mut evidence = Evidence::default();
        assert_eq!(evidence.verdict("service-a", &q), Verdict::Unknown);
        evidence.record("service-a", &q, &answer(&q, &[])).unwrap();
        let restored: Evidence =
            serde_json::from_slice(&serde_json::to_vec(&evidence).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.verdict("service-a", &q), Verdict::Approved);
        assert_eq!(restored.verdict("service-b", &q), Verdict::Unknown);
        assert_eq!(
            restored.verdict("service-a", &query(&[ALICE, BOB])),
            Verdict::Unknown
        );
    }

    #[test]
    fn a_learned_revocation_dominates_late_approvals_and_restart() {
        let q = query(&[ALICE, BOB]);
        let mut evidence = Evidence::default();
        evidence.record("service-a", &q, &answer(&q, &[])).unwrap();
        evidence
            .record("service-a", &q, &answer(&q, &[ALICE]))
            .unwrap();
        evidence.record("service-b", &q, &answer(&q, &[])).unwrap();
        let restored: Evidence =
            serde_json::from_slice(&serde_json::to_vec(&evidence).unwrap()).unwrap();
        assert_eq!(
            restored.verdict("service-b", &q),
            Verdict::Revoked(ALICE.into())
        );
        assert_eq!(
            restored.verdict("service-c", &query(&[BOB])),
            Verdict::Unknown
        );
    }

    #[test]
    fn mismatched_or_unauthorized_answers_never_warm_the_cache() {
        let q = query(&[ALICE]);
        let mut evidence = Evidence::default();
        assert!(evidence.record("service", &q, &answer(&q, &[BOB])).is_err());
        assert!(
            evidence
                .record("service", &q, &answer(&query(&[BOB]), &[]))
                .is_err()
        );
        assert_eq!(evidence.verdict("service", &q), Verdict::Unknown);
    }

    #[test]
    fn malformed_selectors_and_future_cache_versions_fail_closed() {
        let mut q = query(&[ALICE]);
        q.delegation = "not-a-cid".into();
        assert!(q.validate().is_err());
        let mut evidence = Evidence::default();
        evidence.version = 2;
        assert!(evidence.validate().is_err());
        assert!(serde_json::from_slice::<Evidence>(b"corrupt").is_err());
    }
}
