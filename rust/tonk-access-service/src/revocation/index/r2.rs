//! Strongly consistent admission index, with a legacy KV mirror.
//!
//! R2 bindings bypass the HTTP cache. A completed put is visible globally to
//! subsequent head/list calls. Never cache absence. Existing KV facts remain
//! effective; new revocations are acknowledged only after KV and R2 writes.
//!
//! This is an admission barrier, not a drain of operations already in flight.
use super::{IndexError, RevocationIndex, kv::KvRevocationIndex, mirrored::MirroredRevocations};
use async_trait::async_trait;
use std::collections::BTreeSet;
use worker::{Bucket, Env};

const PREFIX: &str = "__tonk/revocations/";

pub struct R2Facts {
    bucket: Bucket,
}

pub type R2RevocationIndex = MirroredRevocations<R2Facts, KvRevocationIndex>;

impl R2RevocationIndex {
    pub fn from_env(env: &Env) -> Result<Self, IndexError> {
        Ok(Self {
            strong: R2Facts {
                bucket: env.bucket("BUCKET").map_err(failed)?,
            },
            legacy: KvRevocationIndex::new(env.kv("REVOCATIONS_KV").map_err(failed)?),
        })
    }
}

fn failed(error: impl std::fmt::Display) -> IndexError {
    IndexError(format!("revocation storage: {error}"))
}

#[async_trait(?Send)]
impl RevocationIndex for R2Facts {
    async fn record(&self, target: &str, subject: &str) -> Result<bool, IndexError> {
        let key = format!("{PREFIX}{target}/{subject}");
        // Facts are monotonic and identical concurrent writes cannot lose one.
        // The receipt's recorded flag is advisory under racing identical calls.
        self.bucket
            .put(&key, Vec::<u8>::new())
            .execute()
            .await
            .map_err(failed)?;
        Ok(true)
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        let prefix = format!("{PREFIX}{target}/");
        let mut subjects = BTreeSet::new();
        let mut cursor = None;
        loop {
            let mut request = self.bucket.list().prefix(&prefix);
            if let Some(cursor) = cursor.take() {
                request = request.cursor(cursor);
            }
            let page = request.execute().await.map_err(failed)?;
            for object in page.objects() {
                if let Some(subject) = object.key().strip_prefix(&prefix) {
                    subjects.insert(subject.to_owned());
                }
            }
            if !page.truncated() {
                break;
            }
            cursor =
                Some(page.cursor().ok_or_else(|| {
                    IndexError("truncated revocation listing has no cursor".into())
                })?);
        }
        Ok(subjects)
    }

    async fn revoked_by_any(
        &self,
        target: &str,
        subjects: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        // Bound point-read fan-out for unusually broad chains. Listing also
        // reads R2 without caching and follows every page before proving absence.
        if subjects.len() > 4 {
            return Ok(!self.subjects(target).await?.is_disjoint(subjects));
        }
        // Check the authoritative store first, so a stale negative in KV can
        // never undo a newly acknowledged revocation. No cache participates.
        let reads = subjects.iter().map(|subject| async move {
            self.bucket
                .head(format!("{PREFIX}{target}/{subject}"))
                .await
                .map_err(failed)
        });
        if futures_util::future::try_join_all(reads)
            .await?
            .iter()
            .any(Option::is_some)
        {
            return Ok(true);
        }
        Ok(false)
    }
}
