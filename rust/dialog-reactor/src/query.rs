//! [`Query`](QueryEffect) — one-shot read against a branch.
//!
//! The non-streaming counterpart to [`Subscribe`](crate::Subscribe):
//! acquire the (node-cache-warm) branch session, run the select once,
//! and return the projected [`Conclusion`]s. No subscriber is
//! registered, so nothing lingers on the branch after the read.
//!
//! Both the worker's `/query` route (non-`text/event-stream` arm) and
//! headless callers route through here so the one-shot read has a
//! single definition.

use dialog_query::ConceptQuery;
use dialog_query::Output as _;

use super::BranchReference;
use super::Conclusion;
use super::env::{BranchOpenProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// One-shot query effect. Built from
/// [`BranchReference::query`](super::BranchReference::query).
pub struct QueryEffect<'a> {
    /// The branch to read from.
    pub branch: BranchReference<'a>,
    /// The query to evaluate once.
    pub query: ConceptQuery,
}

impl<'a> QueryEffect<'a> {
    /// Build a new one-shot query effect.
    pub fn new(branch: BranchReference<'a>, query: ConceptQuery) -> Self {
        Self { branch, query }
    }

    /// Acquire the branch (opening it if needed), run the select
    /// once, and return the projected conclusions.
    pub async fn perform<Env>(self, env: &Env) -> Result<Vec<Conclusion>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + SelectProvider,
    {
        let session = self.branch.acquire(env).await?;
        let terms = self.query.terms.clone();
        // Fold in the branch's session overlay (ephemeral facts kept out
        // of storage, e.g. an invite's private seed) so the read sees
        // them alongside branch facts, and install the deductive-rule
        // source so stored `db.rule/*` rules resolve for this query.
        let conclusions = session
            .handle()
            .query()
            .with(session.overlay())
            .with_rules(std::sync::Arc::new(session.rule_source()))
            .select(tonk_schema::concept::QueryPlan::from(self.query))
            .perform(env)
            .try_vec()
            .await
            .map_err(ReactorError::QueryFailed)?;
        Ok(conclusions
            .iter()
            .map(|c| Conclusion::project(c, &terms))
            .collect())
    }
}
