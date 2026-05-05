//! [`Subscribe`] — open or attach to a subscription.

use bytes::Bytes;
use dialog_query::ConceptQuery;
use tokio::sync::mpsc;

use crate::worker::DefaultOperator;

use super::TonkReactor;
use super::error::ReactorError;

/// Open or attach to a subscription, run an initial poll, and
/// return the receiver. The receiver's first message is the
/// current snapshot (delivered by the initial poll); subsequent
/// messages are change broadcasts.
pub struct Subscribe<'a> {
    reactor: &'a TonkReactor,
    repo: &'a str,
    branch: &'a str,
    query: ConceptQuery,
}

impl<'a> Subscribe<'a> {
    pub(super) fn new(
        reactor: &'a TonkReactor,
        repo: &'a str,
        branch: &'a str,
        query: ConceptQuery,
    ) -> Self {
        Self {
            reactor,
            repo,
            branch,
            query,
        }
    }

    /// Resolve the branch (opening it if needed), attach a
    /// fresh subscriber to the subscription, and poll so the
    /// new subscriber's first message is the current snapshot.
    pub async fn perform(
        self,
        env: &DefaultOperator,
    ) -> Result<mpsc::UnboundedReceiver<Bytes>, ReactorError> {
        let branch = self
            .reactor
            .resolve_branch(self.repo, self.branch, env)
            .await?;
        let (hash, receiver) = self
            .reactor
            .attach_subscriber(self.repo, self.branch, self.query)
            .await?;
        self.reactor
            .poll_subscription(self.repo, self.branch, hash.clone(), &branch, env)
            .await;
        Ok(receiver)
    }
}
