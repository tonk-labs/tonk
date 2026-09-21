//! A read-only query over the same index used by invocation verification.
//!
//! Only the supplied CID/principal pairs are answered; this does not enumerate
//! a registry or confer authority. The relying peer chooses which services it
//! trusts for its spaces. An index error is unavailable, never a negative answer.

use super::index::RevocationIndex;
use tonk_identity::revocation::evidence::{Answer, MAX_BODY_BYTES, MAX_SNAPSHOT_TARGETS, Query};

/// A malformed query and an unavailable index are deliberately distinct.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request exceeded the fixed body limit.
    #[error("revocation query is too large")]
    TooLarge,
    /// Unsupported or malformed selector.
    #[error("invalid revocation query: {0}")]
    Invalid(String),
    /// The index could not establish an answer.
    #[error("revocation index is unavailable")]
    Unavailable,
}

impl Error {
    /// HTTP classification, shared by native and worker adapters.
    pub fn status(&self) -> u16 {
        match self {
            Self::TooLarge => 413,
            Self::Invalid(_) => 400,
            Self::Unavailable => 503,
        }
    }
}

/// Validate before reading the index; answer only authorized candidate pairs.
pub async fn answer(
    index: &(impl RevocationIndex + dialog_common::ConditionalSync),
    body: &[u8],
) -> Result<Answer, Error> {
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::TooLarge);
    }
    let query: Query = serde_json::from_slice(body).map_err(|e| Error::Invalid(e.to_string()))?;
    query
        .validate()
        .map_err(|e| Error::Invalid(e.to_string()))?;
    // Read the snapshot first, then the exact check. A revocation appearing
    // between the two must still be recorded, not mistaken for an old approval.
    let mut targets = index
        .targets(MAX_SNAPSHOT_TARGETS)
        .await
        .map_err(|_| Error::Unavailable)?;
    let revoked_by = index
        .matching(&query.delegation, &query.by)
        .await
        .map_err(|_| Error::Unavailable)?;
    if !revoked_by.is_empty()
        && let Some(targets) = &mut targets
    {
        targets.insert(query.delegation.clone());
    }
    if targets
        .as_ref()
        .is_some_and(|targets| targets.len() > MAX_SNAPSHOT_TARGETS)
    {
        targets = None;
    }
    let answer = Answer {
        query,
        revoked_by,
        targets,
    };
    answer
        .validate_for(&answer.query)
        .map_err(|_| Error::Unavailable)?;
    Ok(answer)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::super::index::{IndexError, MemoryRevocationIndex};
    use super::*;
    use std::collections::BTreeSet;

    const CID: &str = "bafyreidyasztqnjah3v2s5vr4sidcsgksbfvcgeegc4p37irqvps7a7jd4";
    const ALICE: &str = "did:key:z6MkrF2Jq3mNhFsEtYvQeTVZQfZ5fFPMj3DcbSt9uhzNcoVR";
    const BOB: &str = "did:key:z6MkuGiBdtP3ZdjU6H9fsvKJt6PJPMgpCsM9sYswpQvFQ3Pa";

    fn body() -> Vec<u8> {
        serde_json::to_vec(&Query::new(CID.parse().unwrap(), &[ALICE.parse().unwrap()]).unwrap())
            .unwrap()
    }

    #[dialog_common::test]
    async fn snapshot_contains_targets_only_and_never_returns_a_truncated_inventory() {
        let index = MemoryRevocationIndex::default();
        assert_eq!(index.targets(0).await.unwrap(), Some(Default::default()));
        index.record(CID, BOB).await.unwrap();
        assert!(index.targets(0).await.unwrap().is_none());
        let result = answer(&index, &body()).await.unwrap();
        assert!(
            result.revoked_by.is_empty(),
            "BOB is not a candidate in the question"
        );
        assert_eq!(result.targets, Some(BTreeSet::from([CID.into()])));
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(
            !encoded.contains(BOB),
            "the snapshot must not disclose other principals"
        );
    }

    #[dialog_common::test]
    async fn lookup_reads_the_existing_index_without_disclosing_other_principals() {
        let index = MemoryRevocationIndex::default();
        index.record(CID, BOB).await.unwrap();
        assert!(answer(&index, &body()).await.unwrap().revoked_by.is_empty());
        index.record(CID, ALICE).await.unwrap();
        assert_eq!(
            answer(&index, &body()).await.unwrap().revoked_by,
            BTreeSet::from([ALICE.into()])
        );
        assert_eq!(
            answer(&index, &vec![0; MAX_BODY_BYTES + 1])
                .await
                .unwrap_err()
                .status(),
            413
        );
        assert_eq!(answer(&index, b"{}").await.unwrap_err().status(), 400);
    }

    struct Broken;
    #[async_trait::async_trait]
    impl RevocationIndex for Broken {
        async fn record(&self, _: &str, _: &str) -> Result<bool, IndexError> {
            unreachable!()
        }
        async fn subjects(&self, _: &str) -> Result<BTreeSet<String>, IndexError> {
            Err(IndexError("unavailable".into()))
        }
    }
    #[dialog_common::test]
    async fn an_unreadable_index_never_answers_not_revoked() {
        assert_eq!(answer(&Broken, &body()).await.unwrap_err().status(), 503);
    }
}
