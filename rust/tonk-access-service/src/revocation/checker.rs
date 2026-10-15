//! [`RevocationChecker`] over the revocation index.
//!
//! Dialog asks the question per link, and asks it in the only form that is
//! sound: *did any of THESE principals revoke this delegation?* A revocation
//! counts only when its revoker appears among the issuers of the chain being
//! checked, so a CID-keyed lookup would be the wrong shape — it could not
//! tell a revocation that governs this chain from one that governs somebody
//! else's.
//!
//! That is the same question [`RevocationIndex::revoked_by_any`] answers, so
//! this is a thin adapter rather than a second implementation.

use dialog_ucan_core::InvocationChain;
use dialog_ucan_core::revocation::{RevocationChecker, RevocationMatch, RevocationSelector};
use dialog_varsig::Did;
use std::collections::BTreeSet;

use super::index::{IndexError, RevocationIndex};

/// Answers dialog's revocation question from a [`RevocationIndex`].
pub struct IndexedRevocations<I>(pub I);

impl<I> RevocationChecker for IndexedRevocations<I>
where
    I: RevocationIndex + dialog_common::ConditionalSync,
{
    type Error = IndexError;

    async fn query(
        &self,
        selector: RevocationSelector<'_>,
    ) -> Result<Option<RevocationMatch>, Self::Error> {
        let target = selector.delegation.to_string();
        let candidates: BTreeSet<String> = selector.by.iter().map(ToString::to_string).collect();
        if candidates.is_empty() {
            return Ok(None);
        }

        // The index answers whether ANY candidate revoked it; naming which
        // one takes a second, narrower question. Asked only on a hit, so the
        // common path stays one lookup.
        if !self.0.revoked_by_any(&target, &candidates).await? {
            return Ok(None);
        }

        let recorded = self.0.subjects(&target).await?;
        let Some((subject, principal)) = candidates
            .iter()
            .filter(|candidate| recorded.contains(*candidate))
            .find_map(|did| Some((did, did.parse::<Did>().ok()?)))
        else {
            return Ok(None);
        };

        // The signed revocation, so the holder refused on it can verify it
        // rather than take our word. A fact recorded before documents were
        // kept has none: the refusal stands, and carries no evidence.
        let proof = self.0.document(&target, subject).await?;
        Ok(Some(RevocationMatch {
            // The revocation's own address when its document is kept and
            // reads as one. Otherwise the revoked delegation, which keeps the
            // field honest: it is what we can point at.
            revocation: proof
                .as_deref()
                .and_then(|document| InvocationChain::try_from(document).ok())
                .map_or(selector.delegation, |chain| chain.invocation.to_cid()),
            principal,
            proof,
        }))
    }
}

#[cfg(all(test, feature = "helpers", not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::revocation::index::MemoryRevocationIndex;
    use ipld_core::cid::Cid;

    const TARGET: &str = "bafyreidyasztqnjah3v2s5vr4sidcsgksbfvcgeegc4p37irqvps7a7jd4";
    const ALICE: &str = "did:key:z6MkrF2Jq3mNhFsEtYvQeTVZQfZ5fFPMj3DcbSt9uhzNcoVR";
    const BOB: &str = "did:key:z6MkuGiBdtP3ZdjU6H9fsvKJt6PJPMgpCsM9sYswpQvFQ3Pa";

    fn cid() -> Cid {
        TARGET.parse().expect("a valid CID")
    }

    fn did(text: &str) -> Did {
        text.parse().expect("a valid DID")
    }

    #[dialog_common::test]
    async fn it_reports_a_revocation_by_a_candidate() {
        let index = MemoryRevocationIndex::default();
        index
            .record(TARGET, ALICE, b"alice's revocation")
            .await
            .unwrap();

        let checker = IndexedRevocations(index);
        let by = [did(ALICE)];
        let found = checker
            .query(RevocationSelector::new(cid(), &by))
            .await
            .unwrap()
            .expect("alice revoked it and alice is a candidate");

        assert_eq!(found.principal, did(ALICE), "the revoker must be named");
        assert_eq!(
            found.proof.as_deref(),
            Some(b"alice's revocation".as_slice()),
            "the document the revocation was recorded with comes back with it"
        );
    }

    #[dialog_common::test]
    async fn it_reports_an_older_revocation_without_a_document() {
        // A fact recorded before documents were kept still refuses the
        // chain; it just has no evidence to hand the holder.
        let index = MemoryRevocationIndex::default();
        index.record_without_document(TARGET, ALICE).unwrap();

        let checker = IndexedRevocations(index);
        let by = [did(ALICE)];
        let found = checker
            .query(RevocationSelector::new(cid(), &by))
            .await
            .unwrap()
            .expect("the fact stands");
        assert_eq!(found.principal, did(ALICE));
        assert_eq!(found.proof, None);
        assert_eq!(found.revocation, cid(), "the revoked delegation stands in");
    }

    #[dialog_common::test]
    async fn it_ignores_a_revocation_by_a_principal_outside_the_candidates() {
        // The rule the whole design rests on: a revocation counts only
        // where its revoker could have granted the delegation. Bob revoked
        // it, but Bob issued nothing into the chain being checked, so this
        // is not a revocation OF that chain.
        let index = MemoryRevocationIndex::default();
        index
            .record(TARGET, BOB, b"bob's revocation")
            .await
            .unwrap();

        let checker = IndexedRevocations(index);
        let by = [did(ALICE)];
        assert!(
            checker
                .query(RevocationSelector::new(cid(), &by))
                .await
                .unwrap()
                .is_none(),
            "a revocation by a non-candidate must not be reported"
        );
    }

    #[dialog_common::test]
    async fn it_reports_nothing_for_an_unrevoked_delegation() {
        let checker = IndexedRevocations(MemoryRevocationIndex::default());
        let by = [did(ALICE)];
        assert!(
            checker
                .query(RevocationSelector::new(cid(), &by))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[dialog_common::test]
    async fn it_asks_nothing_when_there_are_no_candidates() {
        // A chain rooted in its own subject presents no issuers to match
        // against, so there is no question to ask.
        let index = MemoryRevocationIndex::default();
        index
            .record(TARGET, ALICE, b"alice's revocation")
            .await
            .unwrap();

        let checker = IndexedRevocations(index);
        assert!(
            checker
                .query(RevocationSelector::new(cid(), &[]))
                .await
                .unwrap()
                .is_none()
        );
    }
}
