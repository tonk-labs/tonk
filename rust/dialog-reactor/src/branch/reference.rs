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

use crate::env::{BranchOpenProvider, LoadProvider};
use crate::error::ReactorError;
use crate::export::Export;
use crate::import::Import;
use crate::overlay::OverlayBuilder;
use crate::pull::Pull;
use crate::push::Push;
use crate::query::QueryEffect;
use crate::subscribe::Subscribe;
use crate::transaction::TransactionBuilder;
use crate::{BranchSession, BranchState, RepositoryReference};

/// Names a branch within a repository. Acquire the underlying
/// handle with [`Self::acquire`] or chain to a leaf effect.
#[derive(Clone, Copy)]
pub struct BranchReference<'a> {
    /// The parent repository handle.
    pub repository: RepositoryReference<'a>,
    /// Branch name within the repository.
    pub name: &'a str,
}

/// Move every subscription waiting on this branch out of the reactor's
/// waiting room and onto the live [`BranchState`].
///
/// This is the whole hand-off. A page that subscribed before the repo
/// (or the branch) existed was answered with the empty set and kept its
/// stream open; adopting its sender here means the very next poll
/// delivers real rows into that same stream. Nothing polls and nothing
/// retries — the branch coming into existence IS the event, so a space
/// joined in another tab and a branch created later behave identically.
///
/// A no-op, and just one map lookup, when nothing was waiting.
fn adopt_waiting(reference: &BranchReference<'_>, state: &Arc<BranchState>, name: &str) {
    let reactor = reference.reactor();
    let repo = reference.repository.name();
    reactor.adopt_pending(repo, name, state);
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

        // Fast path: branch already cached. Still drains the waiting
        // room — a subscriber can register while the branch is absent
        // and the branch appear via a DIFFERENT path (another request
        // acquiring it first), so the cached case is a real arrival too.
        let cached = {
            let branches = repository.branches().read();
            branches.get(name).cloned()
        };
        if let Some(state) = cached {
            adopt_waiting(self, &state, name);
            return Ok(BranchSession { state });
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

        let state = {
            let mut branches = repository.branches().write();
            let entry = branches
                .entry(name.to_owned())
                .or_insert_with(|| Arc::new(BranchState::new(branch)));
            Arc::clone(entry)
        };

        adopt_waiting(self, &state, name);
        Ok(BranchSession { state })
    }

    /// The reactor that owns this branch's cache — so leaf effects can
    /// schedule a poll on the affected branch instead of polling inline.
    pub(crate) fn reactor(&self) -> &'a crate::Reactor {
        self.repository.reactor()
    }

    /// Open or attach to a standing subscription for `query`.
    pub fn subscribe(self, query: ConceptQuery) -> Subscribe<'a> {
        Subscribe::new(self, query)
    }

    /// Read `query` once and return the projected conclusions. The
    /// non-streaming counterpart to [`Self::subscribe`] — no
    /// subscriber is registered on the branch.
    pub fn query(self, query: ConceptQuery) -> QueryEffect<'a> {
        QueryEffect::new(self, query)
    }

    /// Begin a transaction. Chain `.assert(…)` / `.retract(…)`,
    /// then `.commit().perform(&op)` to apply atomically. Commit
    /// re-polls every subscription on the branch so changed query
    /// results fan out without callers having to remember.
    pub fn transaction(self) -> TransactionBuilder<'a> {
        TransactionBuilder::new(self)
    }

    /// Begin a **session-overlay** write — the ephemeral counterpart to
    /// [`Self::transaction`]. Chain `.assert(…)` / `.retract(…)`, then
    /// `.write().perform(&op)`. The changes land in the in-memory overlay
    /// (never committed, never replicated) and, like a commit, schedule a poll
    /// so subscribers see the change — callers don't drive the poll themselves.
    pub fn overlay(self) -> OverlayBuilder<'a> {
        OverlayBuilder::new(self)
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
