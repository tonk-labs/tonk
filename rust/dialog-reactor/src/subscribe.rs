//! [`Subscribe`] — subscribe a connection to a standing query.

use dialog_query::ConceptQuery;

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, SettleProvider};
use super::error::ReactorError;
use super::subscription::Subscriber;

/// Subscribe a connection to a query on a branch, settle the branch,
/// and return the [`Subscriber`]. Its first frame is the current
/// snapshot; later frames are the changes.
pub struct Subscribe<'a> {
    /// The branch the subscription is scoped to.
    pub branch: BranchReference<'a>,
    /// The query the subscription re-evaluates on every change.
    pub query: ConceptQuery,
    /// The client this subscriber serves, when known — see
    /// [`BranchState::retain_subscribers`](crate::BranchState::retain_subscribers).
    pub client: Option<String>,
    /// Consumer nesting level, delivered outermost first — see
    /// [`BranchState::deliver`](crate::BranchState::deliver).
    pub level: u32,
}

impl<'a> Subscribe<'a> {
    /// Build a new `Subscribe` effect.
    pub fn new(branch: BranchReference<'a>, query: ConceptQuery) -> Self {
        Self {
            branch,
            query,
            client: None,
            level: 0,
        }
    }

    /// Tag the subscriber with the client it serves, so the owner
    /// can prune it once that client is no longer alive.
    pub fn client(mut self, client: impl Into<String>) -> Self {
        self.client = Some(client.into());
        self
    }

    /// Set how deeply the consumer is nested, so its branch delivers a
    /// change to outer consumers before inner ones.
    pub fn level(mut self, level: u32) -> Self {
        self.level = level;
        self
    }

    /// Resolve the branch (opening it if needed), subscribe a fresh
    /// connection, and settle the branch so the connection's first frame
    /// is the current snapshot.
    pub async fn perform<Env>(self, env: &Env) -> Result<Subscriber, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + SettleProvider,
    {
        let session = self.branch.acquire(env).await?;
        let subscriber = self.branch.reactor().register_subscription(
            &session,
            self.query,
            self.client,
            self.level,
        )?;
        session.state.settle(env).await;
        Ok(subscriber)
    }
}
