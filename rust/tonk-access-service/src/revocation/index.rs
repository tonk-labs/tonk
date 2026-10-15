//! The revocation index: which delegations were withdrawn, and by whom.
//!
//! One key per `(target, subject)` fact, because the key is the fact.
//! KV offers no compare-and-swap, so a set stored under one key would
//! let two principals revoking the same delegation clobber each other
//! with neither seeing a conflict. Distinct keys cannot collide, so a
//! write needs no read, no merge, and no retry.
//!
//! The value is the signed revocation that established the fact, the
//! container the subject posted, kept so a refusal can hand it to the
//! holder: a holder retracts a revoked grant only on a revocation it can
//! verify, never on the service's word alone. A key written before the
//! document was kept holds an empty value; the fact stands, and a refusal
//! on it carries no document until the revocation is posted again.
//!
//! Reads go the other way: for each delegation a chain presents, list
//! the prefix and intersect the subjects found with the issuers that
//! chain proves. A revocation matches only when its subject held
//! authority over the path in front of us — the spec's rule, with
//! [ucan-wg/revocation#4](https://github.com/ucan-wg/revocation/pull/4)
//! applied, so it binds on the subject rather than the invocation's
//! issuer.
//!
//! Keys are written without an expiry and are never pruned. A
//! revocation is permanent: it says an authority was withdrawn, and
//! that does not stop being true. Expiring the key would silently flip
//! the answer back to "not revoked", which is the one wrong answer this
//! index must never give.
//!
//! Bounding the key by the revoked delegation's own expiration looks
//! free — a chain past its expiry is refused on time anyway — but it is
//! not. The delegations most worth revoking are unexpiring (a device
//! grant carries no `exp`), so there is no bound to use for exactly the
//! cases that matter. The storage is a key per withdrawn delegation,
//! which is bounded by how many delegations were ever revoked.

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

/// The value a revocation key holds for `document`: the container's bytes
/// as unpadded base64url, so a backend that reads values as text (KV's
/// bulk read) can carry it.
pub fn encode_document(document: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(document)
}

/// The document a revocation key's value holds. `None` for the empty
/// value a key written before documents were kept holds.
///
/// # Errors
///
/// Returns an [`IndexError`] for a value that is not base64url: a
/// corrupt record, not a missing one.
pub fn decode_document(value: &str) -> Result<Option<Vec<u8>>, IndexError> {
    use base64::Engine as _;
    if value.is_empty() {
        return Ok(None);
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map(Some)
        .map_err(|error| IndexError(format!("revocation record is not base64url: {error}")))
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
    /// Record that `subject` withdrew `target`, keeping `document`, the
    /// signed revocation container that says so. Idempotent: re-recording
    /// the same fact answers `false` rather than failing, since the
    /// revocation is one fact however many times it arrives. The document
    /// is written either way, so posting a revocation again gives a fact
    /// recorded before documents were kept its document.
    async fn record(
        &self,
        target: &str,
        subject: &str,
        document: &[u8],
    ) -> Result<bool, IndexError>;

    /// The signed revocation recorded for `subject` withdrawing `target`.
    /// `None` when no such fact is recorded, or when it was recorded
    /// before documents were kept.
    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError>;

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

/// A borrowed index is itself an index.
///
/// Lets a holder lend its index to a checker for the length of one
/// verification rather than give it away.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<I: RevocationIndex + dialog_common::ConditionalSync> RevocationIndex for &I {
    async fn record(
        &self,
        target: &str,
        subject: &str,
        document: &[u8],
    ) -> Result<bool, IndexError> {
        (*self).record(target, subject, document).await
    }

    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError> {
        (*self).document(target, subject).await
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        (*self).subjects(target).await
    }

    async fn revoked_by_any(
        &self,
        target: &str,
        subjects: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        (*self).revoked_by_any(target, subjects).await
    }
}

/// Shared ownership of an index is itself an index.
///
/// The native server hands one index to two holders at once: the
/// revoke handler that writes it, and the authorizer that reads it
/// while verifying. Neither can own it outright, and copying would
/// give them separate sets.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl<I: RevocationIndex + dialog_common::ConditionalSync> RevocationIndex for std::sync::Arc<I> {
    async fn record(
        &self,
        target: &str,
        subject: &str,
        document: &[u8],
    ) -> Result<bool, IndexError> {
        self.as_ref().record(target, subject, document).await
    }

    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError> {
        self.as_ref().document(target, subject).await
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        self.as_ref().subjects(target).await
    }

    async fn revoked_by_any(
        &self,
        target: &str,
        subjects: &BTreeSet<String>,
    ) -> Result<bool, IndexError> {
        self.as_ref().revoked_by_any(target, subjects).await
    }
}

/// An in-memory index, for tests and the native server.
///
/// Keys to the value KV would hold, so the empty value of a record kept
/// before documents were is represented the same way.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct MemoryRevocationIndex(std::sync::Mutex<std::collections::BTreeMap<String, String>>);

#[cfg(any(test, feature = "helpers"))]
impl MemoryRevocationIndex {
    /// Record a fact with an empty value, as a key written before
    /// documents were kept holds.
    pub fn record_without_document(&self, target: &str, subject: &str) -> Result<bool, IndexError> {
        let mut keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        Ok(keys
            .insert(revocation_key(target, subject), String::new())
            .is_none())
    }
}

#[cfg(any(test, feature = "helpers"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl RevocationIndex for MemoryRevocationIndex {
    async fn record(
        &self,
        target: &str,
        subject: &str,
        document: &[u8],
    ) -> Result<bool, IndexError> {
        let mut keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        Ok(keys
            .insert(revocation_key(target, subject), encode_document(document))
            .is_none())
    }

    async fn document(&self, target: &str, subject: &str) -> Result<Option<Vec<u8>>, IndexError> {
        let keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        match keys.get(&revocation_key(target, subject)) {
            Some(value) => decode_document(value),
            None => Ok(None),
        }
    }

    async fn subjects(&self, target: &str) -> Result<BTreeSet<String>, IndexError> {
        let keys = self
            .0
            .lock()
            .map_err(|_| IndexError("revocation index lock poisoned".to_string()))?;
        let prefix = target_prefix(target);
        Ok(keys
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .filter_map(|(key, _)| key.strip_prefix(&prefix).map(ToString::to_string))
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
        assert!(
            index
                .record("bafyTarget", "did:key:zAlice", b"a revocation")
                .await
                .unwrap()
        );
        assert!(
            index
                .record("bafyTarget", "did:key:zKarl", b"a revocation")
                .await
                .unwrap()
        );

        let subjects = index.subjects("bafyTarget").await.unwrap();
        assert_eq!(
            subjects,
            BTreeSet::from(["did:key:zAlice".to_string(), "did:key:zKarl".to_string()])
        );
    }

    #[dialog_common::test]
    async fn it_reports_a_repeated_revocation_as_already_recorded() {
        let index = MemoryRevocationIndex::default();
        assert!(
            index
                .record("bafyTarget", "did:key:zAlice", b"a revocation")
                .await
                .unwrap()
        );
        assert!(
            !index
                .record("bafyTarget", "did:key:zAlice", b"a revocation")
                .await
                .unwrap()
        );
        assert_eq!(index.subjects("bafyTarget").await.unwrap().len(), 1);
    }

    #[dialog_common::test]
    async fn it_keeps_targets_apart() {
        // Prefix listing must not bleed between targets whose CIDs share
        // a leading substring.
        let index = MemoryRevocationIndex::default();
        index
            .record("bafyTarget", "did:key:zAlice", b"a revocation")
            .await
            .unwrap();
        index
            .record("bafyTargetLonger", "did:key:zKarl", b"a revocation")
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
        index
            .record("bafyTarget", "did:key:zAlice", b"a revocation")
            .await
            .unwrap();

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
    async fn it_keeps_the_document_each_revocation_was_recorded_with() {
        let index = MemoryRevocationIndex::default();
        index
            .record("bafyTarget", "did:key:zAlice", b"alice's revocation")
            .await
            .unwrap();
        index
            .record("bafyTarget", "did:key:zKarl", b"karl's revocation")
            .await
            .unwrap();

        assert_eq!(
            index
                .document("bafyTarget", "did:key:zAlice")
                .await
                .unwrap(),
            Some(b"alice's revocation".to_vec())
        );
        assert_eq!(
            index.document("bafyTarget", "did:key:zKarl").await.unwrap(),
            Some(b"karl's revocation".to_vec())
        );
        assert_eq!(
            index.document("bafyTarget", "did:key:zBob").await.unwrap(),
            None,
            "no fact, no document"
        );
    }

    #[dialog_common::test]
    async fn it_gives_an_older_record_its_document_when_posted_again() {
        // A key written before documents were kept holds an empty value:
        // the fact stands with no document, and re-recording it keeps the
        // document without making it a new fact.
        let index = MemoryRevocationIndex::default();
        assert!(
            index
                .record_without_document("bafyTarget", "did:key:zAlice")
                .unwrap()
        );
        assert_eq!(
            index
                .document("bafyTarget", "did:key:zAlice")
                .await
                .unwrap(),
            None
        );
        assert!(
            index
                .revoked_by_any(
                    "bafyTarget",
                    &BTreeSet::from(["did:key:zAlice".to_string()])
                )
                .await
                .unwrap(),
            "the fact stands without its document"
        );

        assert!(
            !index
                .record("bafyTarget", "did:key:zAlice", b"the revocation")
                .await
                .unwrap(),
            "already recorded"
        );
        assert_eq!(
            index
                .document("bafyTarget", "did:key:zAlice")
                .await
                .unwrap(),
            Some(b"the revocation".to_vec())
        );
    }

    #[dialog_common::test]
    async fn it_reads_an_empty_value_as_no_document_and_refuses_a_corrupt_one() {
        assert_eq!(decode_document("").unwrap(), None);
        assert_eq!(
            decode_document(&encode_document(b"\x00\xffbytes")).unwrap(),
            Some(b"\x00\xffbytes".to_vec())
        );
        assert!(decode_document("not base64url!").is_err());
    }

    #[dialog_common::test]
    async fn it_answers_an_unrevoked_target_with_nothing() {
        let index = MemoryRevocationIndex::default();
        assert!(index.subjects("bafyNeverRevoked").await.unwrap().is_empty());
    }
}
