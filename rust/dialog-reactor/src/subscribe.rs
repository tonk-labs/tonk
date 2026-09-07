//! [`Subscribe`] — open or attach to a subscription.

use dialog_query::ConceptQuery;

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;
use super::subscription::Subscriber;

/// Open or attach to a subscription, run an initial poll, and
/// return the [`Subscriber`] handle. The receiver's first message
/// is the current snapshot (delivered by the initial poll);
/// subsequent messages are change broadcasts.
pub struct Subscribe<'a> {
    /// The branch the subscription is scoped to.
    pub branch: BranchReference<'a>,
    /// The query the subscription re-evaluates on every change.
    pub query: ConceptQuery,
    /// The client this subscriber serves, when known — see
    /// [`BranchState::retain_subscribers`](crate::BranchState::retain_subscribers).
    pub client: Option<String>,
}

impl<'a> Subscribe<'a> {
    /// Build a new `Subscribe` effect.
    pub fn new(branch: BranchReference<'a>, query: ConceptQuery) -> Self {
        Self {
            branch,
            query,
            client: None,
        }
    }

    /// Tag the subscriber with the client it serves, so the owner
    /// can prune it once that client is no longer alive.
    pub fn client(mut self, client: impl Into<String>) -> Self {
        self.client = Some(client.into());
        self
    }

    /// Resolve the branch (opening it if needed), attach a
    /// fresh subscriber to the subscription, and poll so the
    /// new subscriber's first message is the current snapshot.
    pub async fn perform<Env>(self, env: &Env) -> Result<Subscriber, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + SelectProvider,
    {
        let session = self.branch.acquire(env).await?;
        let subscriber =
            self.branch
                .reactor()
                .register_subscription(&session, self.query, self.client)?;
        session
            .subscription(subscriber.hash.clone())
            .poll()
            .perform(env)
            .await;
        Ok(subscriber)
    }
}
