//! [`Query`] — one-shot read effect.

use dialog_query::{ConceptConclusion, ConceptQuery};

use crate::worker::DefaultOperator;

use super::error::ReactorError;
use super::{TonkReactor, run_query};

/// One-shot read. `.perform(&op)` returns a
/// `Vec<ConceptConclusion>`. No subscription is registered.
pub struct Query<'a> {
    reactor: &'a TonkReactor,
    repo: &'a str,
    branch: &'a str,
    query: ConceptQuery,
}

impl<'a> Query<'a> {
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

    /// Execute the query against the branch.
    pub async fn perform(
        self,
        env: &DefaultOperator,
    ) -> Result<Vec<ConceptConclusion>, ReactorError> {
        let branch = self
            .reactor
            .resolve_branch(self.repo, self.branch, env)
            .await?;
        run_query(&branch, self.query, env).await
    }
}
