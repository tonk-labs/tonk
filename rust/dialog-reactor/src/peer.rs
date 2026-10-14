//! Repositories another peer holds.
//!
//! A [`BranchReference`](crate::BranchReference) reads and writes a branch
//! this peer has mounted. A [`PeerBranchReference`] names a branch of a
//! repository some other peer holds, and its leaf effects are requests to
//! that peer: the query runs there, the transaction commits there, and a
//! subscription streams what that peer's reactor broadcasts.
//!
//! The chain mirrors the local one (`repository(r).branch(b).query(q)
//! .perform(&peer)`), with the connection to the peer where the local chain
//! takes the environment that performs storage effects. What carries a
//! request is the connection's business ([`PeerProvider`]); nothing here
//! knows whether it is a message port or anything else.

use std::pin::Pin;

use futures_util::Stream;
use thiserror::Error;
use tonk_schema::claim::SourceClaim;

use crate::{Conclusion, Frame, Query};

/// Why a request to a peer came to nothing.
#[derive(Debug, Error)]
pub enum PeerError {
    /// The peer could not be asked at all.
    #[error("the peer could not be reached: {0}")]
    Unreachable(String),
    /// The peer was asked and refused.
    #[error("the peer refused: {0}")]
    Refused(String),
    /// The peer answered something that is not an answer to the request.
    #[error("the peer's answer could not be read: {0}")]
    Malformed(String),
}

/// What a subscription on a peer's branch yields: a snapshot first, then a
/// frame for every change, until the peer closes it or the stream is
/// dropped.
#[cfg(not(target_arch = "wasm32"))]
pub type PeerFrames = Pin<Box<dyn Stream<Item = Result<Frame, PeerError>> + Send>>;

/// What a subscription on a peer's branch yields: a snapshot first, then a
/// frame for every change, until the peer closes it or the stream is
/// dropped.
#[cfg(target_arch = "wasm32")]
pub type PeerFrames = Pin<Box<dyn Stream<Item = Result<Frame, PeerError>>>>;

/// A connection to a peer that answers for repositories it holds.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait PeerProvider {
    /// Run `query` once on `branch` and answer its conclusions.
    async fn query(
        &self,
        branch: PeerBranchReference<'_>,
        query: &Query,
    ) -> Result<Vec<Conclusion>, PeerError>;

    /// Open a subscription to `query` on `branch`.
    async fn subscribe(
        &self,
        branch: PeerBranchReference<'_>,
        query: &Query,
    ) -> Result<PeerFrames, PeerError>;

    /// Commit `claims` on `branch` in one transaction.
    async fn transact(
        &self,
        branch: PeerBranchReference<'_>,
        claims: &[SourceClaim],
    ) -> Result<(), PeerError>;
}

/// Names a repository a peer holds.
#[derive(Clone, Copy, Debug)]
pub struct PeerRepositoryReference<'a> {
    /// The repository's name, as the peer that holds it knows it.
    pub name: &'a str,
}

impl<'a> PeerRepositoryReference<'a> {
    /// The repository a peer holds under `name`.
    pub fn new(name: &'a str) -> Self {
        Self { name }
    }

    /// A branch of this repository.
    pub fn branch(self, name: &'a str) -> PeerBranchReference<'a> {
        PeerBranchReference {
            repository: self,
            name,
        }
    }
}

/// Names a branch of a repository a peer holds. Chain to a leaf effect.
#[derive(Clone, Copy, Debug)]
pub struct PeerBranchReference<'a> {
    /// The repository the branch is in.
    pub repository: PeerRepositoryReference<'a>,
    /// The branch's name within the repository.
    pub name: &'a str,
}

impl<'a> PeerBranchReference<'a> {
    /// Read `query` once, where the branch is held.
    pub fn query(self, query: impl Into<Query>) -> PeerQuery<'a> {
        PeerQuery {
            branch: self,
            query: query.into(),
        }
    }

    /// Open a subscription to `query`, where the branch is held.
    pub fn subscribe(self, query: impl Into<Query>) -> PeerSubscribe<'a> {
        PeerSubscribe {
            branch: self,
            query: query.into(),
        }
    }

    /// Begin a transaction. Chain `.apply(…)`, then `.commit().perform(&peer)`.
    pub fn transaction(self) -> PeerTransaction<'a> {
        PeerTransaction {
            branch: self,
            claims: Vec::new(),
        }
    }
}

/// One-shot query on a peer's branch. Built from
/// [`PeerBranchReference::query`].
pub struct PeerQuery<'a> {
    /// The branch to read.
    pub branch: PeerBranchReference<'a>,
    /// The query the peer evaluates.
    pub query: Query,
}

impl PeerQuery<'_> {
    /// Have the peer run the query and answer its conclusions.
    pub async fn perform<Peer>(self, peer: &Peer) -> Result<Vec<Conclusion>, PeerError>
    where
        Peer: PeerProvider + ?Sized,
    {
        peer.query(self.branch, &self.query).await
    }
}

/// A subscription on a peer's branch. Built from
/// [`PeerBranchReference::subscribe`].
pub struct PeerSubscribe<'a> {
    /// The branch the subscription is scoped to.
    pub branch: PeerBranchReference<'a>,
    /// The query the peer re-evaluates on every change.
    pub query: Query,
}

impl PeerSubscribe<'_> {
    /// Have the peer open the subscription.
    pub async fn perform<Peer>(self, peer: &Peer) -> Result<PeerFrames, PeerError>
    where
        Peer: PeerProvider + ?Sized,
    {
        peer.subscribe(self.branch, &self.query).await
    }
}

/// Accumulates claims for a peer to commit. Built from
/// [`PeerBranchReference::transaction`].
pub struct PeerTransaction<'a> {
    /// The branch the transaction commits to.
    pub branch: PeerBranchReference<'a>,
    /// The claims accumulated so far.
    pub claims: Vec<SourceClaim>,
}

impl<'a> PeerTransaction<'a> {
    /// Add a claim. The peer validates it against what it holds, as it
    /// would one of its own.
    pub fn apply(mut self, claim: SourceClaim) -> Self {
        self.claims.push(claim);
        self
    }

    /// Finish the batch.
    pub fn commit(self) -> PeerCommit<'a> {
        PeerCommit {
            branch: self.branch,
            claims: self.claims,
        }
    }
}

/// A transaction ready for a peer to commit.
pub struct PeerCommit<'a> {
    /// The branch the transaction commits to.
    pub branch: PeerBranchReference<'a>,
    /// The claims to commit.
    pub claims: Vec<SourceClaim>,
}

impl PeerCommit<'_> {
    /// Have the peer commit the claims in one transaction. A commit there
    /// re-polls that peer's subscriptions, the ones opened from here
    /// included.
    pub async fn perform<Peer>(self, peer: &Peer) -> Result<(), PeerError>
    where
        Peer: PeerProvider + ?Sized,
    {
        peer.transact(self.branch, &self.claims).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::StreamExt as _;
    use futures_util::stream;
    use serde_json::json;

    use super::*;

    /// A peer that records what it was asked and answers from a script.
    #[derive(Default)]
    struct Recorded {
        asked: Mutex<Vec<String>>,
    }

    impl Recorded {
        fn note(&self, what: &str, branch: PeerBranchReference<'_>) {
            self.asked
                .lock()
                .unwrap()
                .push(format!("{what} {}@{}", branch.name, branch.repository.name));
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl PeerProvider for Recorded {
        async fn query(
            &self,
            branch: PeerBranchReference<'_>,
            _query: &Query,
        ) -> Result<Vec<Conclusion>, PeerError> {
            self.note("query", branch);
            Ok(Vec::new())
        }

        async fn subscribe(
            &self,
            branch: PeerBranchReference<'_>,
            _query: &Query,
        ) -> Result<PeerFrames, PeerError> {
            self.note("subscribe", branch);
            Ok(Box::pin(stream::iter([Ok(Frame::Snapshot {
                conclusions: Vec::new(),
            })])))
        }

        async fn transact(
            &self,
            branch: PeerBranchReference<'_>,
            claims: &[SourceClaim],
        ) -> Result<(), PeerError> {
            self.note(&format!("transact {}", claims.len()), branch);
            Ok(())
        }
    }

    fn query() -> Query {
        serde_json::from_value(json!({
            "predicate": { "with": { "name": { "the": "xyz.tonk.probe/name", "as": "Text" } } },
            "terms": { "this": { "?": { "name": "this" } }, "name": { "?": { "name": "name" } } }
        }))
        .expect("the query parses")
    }

    fn claim() -> SourceClaim {
        serde_json::from_value(json!({
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": { "with": { "name": { "the": "xyz.tonk.probe/name", "as": "Text" } } }
                },
                "parameters": { "this": "id:probe", "name": "a name" }
            }
        }))
        .expect("the claim parses")
    }

    #[dialog_common::test]
    async fn it_asks_the_peer_for_each_effect_on_the_branch_it_names() {
        let peer = Recorded::default();
        let branch = PeerRepositoryReference::new("did:key:zSpace").branch("main");

        branch.query(query()).perform(&peer).await.unwrap();
        let mut frames = branch.subscribe(query()).perform(&peer).await.unwrap();
        assert!(matches!(
            frames.next().await,
            Some(Ok(Frame::Snapshot { .. }))
        ));
        branch
            .transaction()
            .apply(claim())
            .apply(claim())
            .commit()
            .perform(&peer)
            .await
            .unwrap();

        assert_eq!(
            *peer.asked.lock().unwrap(),
            [
                "query main@did:key:zSpace",
                "subscribe main@did:key:zSpace",
                "transact 2 main@did:key:zSpace",
            ]
        );
    }
}
