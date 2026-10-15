//! KV-backed revocation index.
//!
//! Writes are a bare `put`: the key carries the whole fact and the value
//! the signed revocation that established it, so there is nothing to
//! merge and no read to race against. The value is the document as
//! base64url text ([`encode_document`]), so the bulk read, which decodes
//! values as text, still reads it; a key written before documents were
//! kept holds an empty value.
//!
//! Reads take one of two shapes, chosen by how many keys the question
//! needs. The presign path knows both halves of every key it cares
//! about, so it asks pairwise and one bulk fetch answers the whole
//! chain. Past [`BULK_LIMIT`] pairs that stops being possible, and
//! listing each target's prefix is cheaper than several bulk rounds.

use std::collections::BTreeSet;

use async_trait::async_trait;
use worker::kv::KvStore;

use super::{
    IndexError, RevocationIndex, decode_document, encode_document, revocation_key, target_prefix,
};

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
        // load-bearing: two writers racing here both write the same key,
        // each with a valid revocation of the same fact, so the worst case
        // is both reporting `true` for one fact, never a lost revocation.
        let existed = self
            .store
            .get(&key)
            .text()
            .await
            .map_err(|error| failed("revocation index read failed", error))?
            .is_some();
        self.store
            .put(&key, encode_document(document))
            .map_err(|error| failed("revocation index write failed", error))?
            .execute()
            .await
            .map_err(|error| failed("revocation index write failed", error))?;
        Ok(!existed)
    }

    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError> {
        match self
            .store
            .get(&revocation_key(target, subject))
            .text()
            .await
            .map_err(|error| failed("revocation index read failed", error))?
        {
            Some(value) => decode_document(&value),
            None => Ok(None),
        }
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
        // key is absent. A present entry is the revocation, whatever its
        // value: the document is read separately, and only on a hit.
        Ok(keys
            .iter()
            .any(|key| found.get(key).is_some_and(Option::is_some)))
    }
}
