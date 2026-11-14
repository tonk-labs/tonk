//! KV-backed revocation index.
//!
//! Writes are a bare `put`: the key carries the whole fact, so there is
//! nothing to merge and no read to race against. The value is the signed
//! revocation, base64url-encoded so the bulk text read still answers
//! presence; an empty value is a fact recorded before documents were kept.
//!
//! Reads take one of two shapes, chosen by how many keys the question
//! needs. The presign path knows both halves of every key it cares
//! about, so it asks pairwise and one bulk fetch answers the whole
//! chain. Past [`BULK_LIMIT`] pairs that stops being possible, and
//! listing each target's prefix is cheaper than several bulk rounds.

use std::collections::BTreeSet;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use worker::kv::KvStore;

use super::{IndexError, RevocationIndex, revocation_key, target_prefix};

/// Keys Cloudflare accepts in one `get_bulk`. Past this a pairwise
/// question needs several round trips, and listing wins.
pub const BULK_LIMIT: usize = 100;

/// Revocations in Workers KV.
pub struct KvRevocationIndex {
    store: KvStore,
}

impl KvRevocationIndex {
    /// Wrap a bound KV namespace.
    pub fn new(store: KvStore) -> Self {
        Self { store }
    }
}

fn failed(context: &str, error: impl std::fmt::Display) -> IndexError {
    IndexError(format!("{context}: {error}"))
}

#[async_trait(?Send)]
impl RevocationIndex for KvRevocationIndex {
    async fn record(
        &self,
        target: &str,
        subject: &str,
        document: &[u8],
    ) -> Result<bool, IndexError> {
        let key = revocation_key(target, subject);
        // Answer whether this call is what recorded it. The read is not
        // load-bearing: two writers racing here both write the same key
        // with a revocation of the same fact, so the worst case is both
        // reporting `true` for one fact, never a lost revocation.
        let existed = self
            .store
            .get(&key)
            .text()
            .await
            .map_err(|error| failed("revocation index read failed", error))?
            .is_some();
        self.store
            .put(&key, URL_SAFE_NO_PAD.encode(document))
            .map_err(|error| failed("revocation index write failed", error))?
            .execute()
            .await
            .map_err(|error| failed("revocation index write failed", error))?;
        Ok(!existed)
    }

    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError> {
        let Some(encoded) = self
            .store
            .get(&revocation_key(target, subject))
            .text()
            .await
            .map_err(|error| failed("revocation index read failed", error))?
        else {
            return Ok(None);
        };
        if encoded.is_empty() {
            return Ok(None);
        }
        URL_SAFE_NO_PAD
            .decode(encoded)
            .map(Some)
            .map_err(|error| failed("revocation document does not decode", error))
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        let prefix = target_prefix(target);
        let mut subjects = BTreeSet::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut listing = self.store.list().prefix(prefix.clone());
            if let Some(cursor) = cursor.take() {
                listing = listing.cursor(cursor);
            }
            let page = listing
                .execute()
                .await
                .map_err(|error| failed("revocation index listing failed", error))?;
            for key in &page.keys {
                if let Some(subject) = key.name.strip_prefix(&prefix) {
                    subjects.insert(subject.to_string());
                }
            }
            match page.cursor {
                // A partial listing cannot prove absence, so follow every
                // page rather than answering from the first one.
                Some(next) if !page.list_complete => cursor = Some(next),
                _ => break,
            }
        }
        Ok(subjects)
    }

    async fn revoked_by_any(
        &self,
        target: &str,
        subjects: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        if subjects.is_empty() {
            return Ok(false);
        }
        // Past the bulk limit the pairwise question needs several round
        // trips, and one listing of the target's prefix is cheaper.
        if subjects.len() > BULK_LIMIT {
            let recorded = self.subjects(target).await?;
            return Ok(recorded.intersection(subjects).next().is_some());
        }
        let keys: Vec<String> = subjects
            .iter()
            .map(|subject| revocation_key(target, subject))
            .collect();
        let found = self
            .store
            .get_bulk(&keys)
            .text()
            .await
            .map_err(|error| failed("revocation index bulk read failed", error))?;
        // The map carries an entry per requested key, `None` where the
        // key is absent. A present entry is the revocation whatever its
        // value, so presence is the whole signal.
        Ok(keys
            .iter()
            .any(|key| found.get(key).is_some_and(Option::is_some)))
    }
}
