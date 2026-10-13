//! Strong authoritative facts with a legacy mirror retained across rollout.
use super::{IndexError, RevocationIndex};
use async_trait::async_trait;
use dialog_common::ConditionalSync;
use std::collections::BTreeSet;

pub struct MirroredRevocations<Strong, Legacy> {
    pub strong: Strong,
    pub legacy: Legacy,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<S: RevocationIndex + ConditionalSync, L: RevocationIndex + ConditionalSync> RevocationIndex
    for MirroredRevocations<S, L>
{
    async fn record(&self, target: &str, subject: &str) -> Result<bool, IndexError> {
        let by = BTreeSet::from([subject.to_owned()]);
        if self.strong.revoked_by_any(target, &by).await? {
            return Ok(false);
        }
        // KV first: every acknowledged strong fact has an acknowledged legacy
        // mirror. Failure after the first write is safe to retry. Never infer a
        // strong fact from legacy presence, including replay of old receipts.
        self.legacy.record(target, subject).await?;
        self.strong.record(target, subject).await
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        let mut subjects = self.strong.subjects(target).await?;
        subjects.extend(self.legacy.subjects(target).await?);
        Ok(subjects)
    }

    async fn revoked_by_any(
        &self,
        target: &str,
        by: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        if by.is_empty() {
            return Ok(false);
        }
        if self.strong.revoked_by_any(target, by).await? {
            return Ok(true);
        }
        self.legacy.revoked_by_any(target, by).await
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::revocation::index::MemoryRevocationIndex;
    use std::sync::{Arc, Mutex};

    struct Unavailable;
    #[async_trait]
    impl RevocationIndex for Unavailable {
        async fn record(&self, _: &str, _: &str) -> Result<bool, IndexError> {
            Err(IndexError("unavailable".into()))
        }
        async fn subjects(&self, _: &str) -> Result<BTreeSet<String>, IndexError> {
            Err(IndexError("unavailable".into()))
        }
    }

    #[dialog_common::test]
    async fn failed_reads_never_fall_back_to_authorizing_on_absence() {
        let by = BTreeSet::from(["owner".into()]);
        let strong_failed = MirroredRevocations {
            strong: Unavailable,
            legacy: MemoryRevocationIndex::default(),
        };
        assert!(strong_failed.revoked_by_any("grant", &by).await.is_err());
        let legacy_failed = MirroredRevocations {
            strong: MemoryRevocationIndex::default(),
            legacy: Unavailable,
        };
        assert!(legacy_failed.revoked_by_any("grant", &by).await.is_err());
        legacy_failed.strong.record("grant", "owner").await.unwrap();
        assert!(
            legacy_failed.revoked_by_any("grant", &by).await.unwrap(),
            "an authoritative revocation never requires a successful legacy read"
        );
    }

    /// Models cached negative reads even after a successful KV write.
    #[derive(Default)]
    struct Legacy {
        writes: Mutex<usize>,
        fail: Mutex<bool>,
    }
    #[async_trait]
    impl RevocationIndex for Legacy {
        async fn record(&self, _: &str, _: &str) -> Result<bool, IndexError> {
            if *self.fail.lock().unwrap() {
                return Err(IndexError("legacy failed".into()));
            }
            *self.writes.lock().unwrap() += 1;
            Ok(true)
        }
        async fn subjects(&self, _: &str) -> Result<BTreeSet<String>, IndexError> {
            Ok(BTreeSet::new())
        }
    }

    #[dialog_common::test]
    async fn acknowledged_facts_override_warmed_legacy_absence_and_repeat_without_writes() {
        let index = MirroredRevocations {
            strong: MemoryRevocationIndex::default(),
            legacy: Legacy::default(),
        };
        let by = BTreeSet::from(["owner".into()]);
        assert!(!index.revoked_by_any("grant", &by).await.unwrap());
        assert!(index.record("grant", "owner").await.unwrap());
        assert!(index.revoked_by_any("grant", &by).await.unwrap());
        assert!(!index.record("grant", "owner").await.unwrap());
        assert_eq!(*index.legacy.writes.lock().unwrap(), 1);
        assert_eq!(index.subjects("grant").await.unwrap(), by);
    }

    #[dialog_common::test]
    async fn legacy_failure_does_not_acknowledge_or_write_a_strong_fact() {
        let index = MirroredRevocations {
            strong: MemoryRevocationIndex::default(),
            legacy: Legacy::default(),
        };
        *index.legacy.fail.lock().unwrap() = true;
        assert!(index.record("grant", "owner").await.is_err());
        assert!(index.strong.subjects("grant").await.unwrap().is_empty());
        *index.legacy.fail.lock().unwrap() = false;
        assert!(index.record("grant", "owner").await.unwrap());
    }

    #[dialog_common::test]
    async fn existing_legacy_facts_are_kept_and_backfilled_on_replay() {
        let index = MirroredRevocations {
            strong: MemoryRevocationIndex::default(),
            legacy: MemoryRevocationIndex::default(),
        };
        index.legacy.record("grant", "owner").await.unwrap();
        let by = BTreeSet::from(["owner".into()]);
        assert!(index.revoked_by_any("grant", &by).await.unwrap());
        index.record("grant", "owner").await.unwrap();
        assert!(index.strong.revoked_by_any("grant", &by).await.unwrap());
    }

    struct FallibleStrong {
        memory: MemoryRevocationIndex,
        fail: Arc<Mutex<bool>>,
    }
    #[async_trait]
    impl RevocationIndex for FallibleStrong {
        async fn record(&self, target: &str, subject: &str) -> Result<bool, IndexError> {
            if *self.fail.lock().unwrap() {
                return Err(IndexError("strong write failed".into()));
            }
            self.memory.record(target, subject).await
        }
        async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
            self.memory.subjects(target).await
        }
    }
    #[dialog_common::test]
    async fn strong_failure_after_legacy_write_is_reported_and_retry_finishes() {
        let fail = Arc::new(Mutex::new(true));
        let index = MirroredRevocations {
            strong: FallibleStrong {
                memory: MemoryRevocationIndex::default(),
                fail: fail.clone(),
            },
            legacy: MemoryRevocationIndex::default(),
        };
        assert!(index.record("grant", "owner").await.is_err());
        assert!(
            index
                .legacy
                .subjects("grant")
                .await
                .unwrap()
                .contains("owner")
        );
        assert!(index.strong.subjects("grant").await.unwrap().is_empty());
        *fail.lock().unwrap() = false;
        assert!(index.record("grant", "owner").await.unwrap());
        assert!(
            index
                .strong
                .subjects("grant")
                .await
                .unwrap()
                .contains("owner")
        );
    }
}
