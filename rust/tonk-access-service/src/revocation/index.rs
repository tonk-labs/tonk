//! The revocation index: which delegations were withdrawn, and by whom.
//!
//! One key per `(target, subject)` fact, value empty, because the key is
//! the fact. KV offers no compare-and-swap, so a set stored under one
//! key would let two principals revoking the same delegation clobber
//! each other with neither seeing a conflict. Distinct keys cannot
//! collide, so a write needs no read, no merge, and no retry.
//!
//! Reads go the other way: for each delegation a chain presents, list
//! the prefix and intersect the subjects found with the issuers that
//! chain proves. A revocation matches only when its subject held
//! authority over the path in front of us — the spec's rule, with
//! [ucan-wg/revocation#4](https://github.com/ucan-wg/revocation/pull/4)
//! applied, so it binds on the subject rather than the invocation's
//! issuer.

#[cfg(target_arch = "wasm32")]
pub mod kv;

use std::collections::BTreeSet;

use async_trait::async_trait;

/// Prefix every revocation key sits under.
pub const REVOKED_PREFIX: &str = "revoked/";

/// The key recording that `subject` withdrew `target`.
pub fn revocation_key(target: &str, subject: &str) -> String {
    format!("{REVOKED_PREFIX}{target}/{subject}")
}

/// The prefix listing every revocation of `target`.
pub fn target_prefix(target: &str) -> String {
    format!("{REVOKED_PREFIX}{target}/")
}

/// Why an index operation failed.
///
/// Deliberately distinct from a revocation verdict: an index we cannot
/// read is our own unavailability, not a statement that anything was
/// withdrawn. ucanto conflates the two and carries a TODO about it.
#[derive(Debug)]
pub struct IndexError(pub String);

impl std::fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for IndexError {}

/// Storage for revocation facts.
///
/// Declared through the dual `async_trait` forms dialog uses, so the
/// trait promises `Send` futures natively and nothing on wasm32, and
/// callers are written once against both backends.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait RevocationIndex {
    /// Record that `subject` withdrew `target`. Idempotent: re-recording
    /// the same fact answers `false` rather than failing, since the
    /// revocation is one fact however many times it arrives.
    async fn record(&self, target: &str, subject: &str) -> Result<bool, IndexError>;

    /// Every subject that withdrew `target`.
    ///
    /// The general answer, for when the candidate subjects are not known
    /// in advance. On the presign path they are, and
    /// [`revoked_by_any`](Self::revoked_by_any) is the cheaper question.
    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError>;

    /// Whether any of `subjects` withdrew `target`.
    ///
    /// The presign path knows both halves before it reads: the CIDs come
    /// from the presented chain and the subjects are the issuers that
    /// chain proves. So the exact keys can be computed and fetched,
    /// rather than listing a prefix and discarding everything that is
    /// not one of those issuers.
    ///
    /// Chains here are short (root to device, occasionally one more), so
    /// the product stays small. The default implementation lists, which
    /// is correct for any backend; KV overrides it with point reads.
    async fn revoked_by_any(
        &self,
        target: &str,
        subjects: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        let recorded = self.subjects(target).await?;
        Ok(recorded.intersection(subjects).next().is_some())
    }
}

/// An in-memory index, for tests and the native server.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct MemoryRevocationIndex(std::sync::Mutex<BTreeSet<String>>);

#[cfg(any(test, feature = "helpers"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl RevocationIndex for MemoryRevocationIndex {
    async fn record(&self, target: &str, subject: &str) -> Result<bool, IndexError> {
        let mut keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        Ok(keys.insert(revocation_key(target, subject)))
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        let keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        let prefix = target_prefix(target);
        Ok(keys
            .range(prefix.clone()..)
            .take_while(|key| key.starts_with(&prefix))
            .filter_map(|key| key.strip_prefix(&prefix).map(ToString::to_string))
            .collect())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    async fn it_records_each_subject_separately_for_one_target() {
        // The property key-per-pair exists for: two principals revoking
        // the same delegation must both survive, with no read-modify-
        // write between them to lose one.
        let index = MemoryRevocationIndex::default();
        assert!(index.record("bafyTarget", "did:key:zAlice").await.unwrap());
        assert!(index.record("bafyTarget", "did:key:zKarl").await.unwrap());

        let subjects = index.subjects("bafyTarget").await.unwrap();
        assert_eq!(
            subjects,
            BTreeSet::from(["did:key:zAlice".to_string(), "did:key:zKarl".to_string()])
        );
    }

    #[dialog_common::test]
    async fn it_reports_a_repeated_revocation_as_already_recorded() {
        let index = MemoryRevocationIndex::default();
        assert!(index.record("bafyTarget", "did:key:zAlice").await.unwrap());
        assert!(!index.record("bafyTarget", "did:key:zAlice").await.unwrap());
        assert_eq!(index.subjects("bafyTarget").await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_keeps_targets_apart() {
        // Prefix listing must not bleed between targets whose CIDs share
        // a leading substring.
        let index = MemoryRevocationIndex::default();
        index.record("bafyTarget", "did:key:zAlice").await.unwrap();
        index
            .record("bafyTargetLonger", "did:key:zKarl")
            .await
            .unwrap();

        assert_eq!(
            index.subjects("bafyTarget").await.unwrap(),
            BTreeSet::from(["did:key:zAlice".to_string()])
        );
        assert_eq!(
            index.subjects("bafyTargetLonger").await.unwrap(),
            BTreeSet::from(["did:key:zKarl".to_string()])
        );
    }

    #[dialog_common::test]
    async fn it_answers_the_pairwise_question_without_listing_everything() {
        // What the presign path actually asks: not "who revoked this"
        // but "did anyone in this chain revoke it". A subject that
        // revoked the target but is not in the presented chain must not
        // make the answer true.
        let index = MemoryRevocationIndex::default();
        index.record("bafyTarget", "did:key:zAlice").await.unwrap();

        let in_chain = BTreeSet::from(["did:key:zAlice".to_string(), "did:key:zBob".to_string()]);
        assert!(index.revoked_by_any("bafyTarget", &in_chain).await.unwrap());

        let other_chain = BTreeSet::from(["did:key:zKarl".to_string()]);
        assert!(
            !index
                .revoked_by_any("bafyTarget", &other_chain)
                .await
                .unwrap(),
            "a revocation by a principal outside this chain does not apply to it"
        );

        assert!(
            !index.revoked_by_any("bafyOther", &in_chain).await.unwrap(),
            "an unrevoked target is not revoked by anyone"
        );
    }

    #[dialog_common::test]
    async fn it_answers_an_unrevoked_target_with_nothing() {
        let index = MemoryRevocationIndex::default();
        assert!(index.subjects("bafyNeverRevoked").await.unwrap().is_empty());
    }
}
