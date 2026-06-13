//! [`BranchReference`] — chain handle for a branch.
//!
//! Pure description: holds the branch name by reference plus the
//! parent [`RepositoryReference`]. Nothing touches dialog handles
//! until `.acquire(&env)` is called or a leaf effect is
//! `.perform(&env)`d.

use std::sync::Arc;

use dialog_query::ConceptQuery;

use dialog_artifacts::Exporter;
use dialog_common::ConditionalSend;
use dialog_repository::Importer;

use crate::reactor::env::{BranchOpenProvider, LoadProvider};
use crate::reactor::error::ReactorError;
use crate::reactor::export::Export;
use crate::reactor::import::Import;
use crate::reactor::pull::Pull;
use crate::reactor::push::Push;
use crate::reactor::subscribe::Subscribe;
use crate::reactor::transaction::TransactionBuilder;
use crate::reactor::{BranchSession, BranchState, RepositoryReference};

/// Names a branch within a repository. Acquire the underlying
/// handle with [`Self::acquire`] or chain to a leaf effect.
#[derive(Clone, Copy)]
pub struct BranchReference<'a> {
    /// The parent repository handle.
    pub repository: RepositoryReference<'a>,
    /// Branch name within the repository.
    pub name: &'a str,
}

impl<'a> BranchReference<'a> {
    /// Resolve and cache the underlying branch. Returns a
    /// [`BranchSession`] carrying the dialog handle and the
    /// subscription state for this branch — operations on it
    /// don't have to round-trip through the reactor.
    pub async fn acquire<Env>(&self, env: &Env) -> Result<BranchSession, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider,
    {
        let name = self.name;

        // Resolve the repo entry (may open the repository).
        let repository = self.repository.acquire(env).await?;

        // Fast path: branch already cached.
        if let Some(state) = repository.branches().lock().get(name) {
            return Ok(BranchSession {
                state: Arc::clone(state),
            });
        }

        // Open the branch outside the lock — `branch().open()` is
        // async.
        let branch = repository
            .repository()
            .branch(name)
            .open()
            .perform(env)
            .await
            .map_err(|e| ReactorError::BranchNotFound {
                repo: self.repository.name().to_owned(),
                branch: name.to_owned(),
                reason: e.to_string(),
            })?;

        let mut branches = repository.branches().lock();
        let entry = branches
            .entry(name.to_owned())
            .or_insert_with(|| Arc::new(BranchState::new(branch)));

        Ok(BranchSession {
            state: Arc::clone(entry),
        })
    }

    /// Open or attach to a standing subscription for `query`.
    pub fn subscribe(self, query: ConceptQuery) -> Subscribe<'a> {
        Subscribe::new(self, query)
    }

    /// Begin a transaction. Chain `.assert(…)` / `.retract(…)`,
    /// then `.commit().perform(&op)` to apply atomically. Commit
    /// re-polls every subscription on the branch so changed query
    /// results fan out without callers having to remember.
    pub fn transaction(self) -> TransactionBuilder<'a> {
        TransactionBuilder::new(self)
    }

    /// Pull from upstream. On success, subscriptions re-poll.
    pub fn pull(self) -> Pull<'a> {
        Pull::new(self)
    }

    /// Push to upstream. No re-poll — push doesn't change local
    /// branch state.
    pub fn push(self) -> Push<'a> {
        Push::new(self)
    }

    /// Stream every artifact on the branch into `exporter`. Chain
    /// `.perform(&op)`. Read-only — no re-poll.
    pub fn export<E: Exporter>(self, exporter: E) -> Export<'a, E> {
        Export::new(self, exporter)
    }

    /// Commit every artifact `importer` yields as an assertion, in
    /// one transaction. Chain `.perform(&op)`. Re-polls every
    /// subscription on the branch so changed results fan out, the
    /// same way a transaction commit does.
    pub fn import<I: Importer + Unpin + ConditionalSend>(self, importer: I) -> Import<'a, I> {
        Import::new(self, importer)
    }
}
