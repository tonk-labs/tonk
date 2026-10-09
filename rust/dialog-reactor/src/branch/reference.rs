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

use crate::env::{BranchOpenProvider, CommitProvider, LoadProvider};
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

/// What [`BranchReference::upgrade_once`] moved.
#[derive(Debug, Clone)]
pub struct Upgraded {
    /// The attribute and concept definitions written again.
    pub definitions: tonk_schema::upgrade::DefinitionsUpgraded,
    /// The rules re-installed.
    pub rules: dialog_repository::RulesUpgraded,
}

/// Until when [`BranchReference::upgrade_once`] moves what an earlier
/// release stored: 2026-12-09, in unix seconds.
/// After it the upgrade is not attempted; remove the upgrade then.
pub const RULE_UPGRADE_UNTIL: u64 = 1_796_774_400;

/// Whether the rule upgrade window is still open.
fn rule_upgrade_open() -> bool {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .is_ok_and(|now| now.as_secs() < RULE_UPGRADE_UNTIL)
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

    /// Move what an earlier release stored to the shape this one reads,
    /// once per branch while the reactor holds it open, and only until
    /// [`RULE_UPGRADE_UNTIL`]: first the attribute and concept
    /// definitions recorded the earlier way
    /// ([`tonk_schema::upgrade::upgrade_definitions`]), then the rules
    /// stored under an identity an earlier dialog release gave them
    /// ([`Branch::upgrade_rules`]). A later call returns `None` without
    /// reading anything, since both decode every definition and rule body
    /// the branch holds. A failed upgrade is not retried until the branch
    /// is opened again (the next worker or CLI start): a cause like an
    /// unreachable remote would otherwise repeat on every read. Runs under
    /// the branch's transactor lock, as a commit does, and schedules a
    /// poll when it commits.
    ///
    /// [`Branch::upgrade_rules`]: dialog_repository::Branch::upgrade_rules
    pub async fn upgrade_once<Env>(&self, env: &Env) -> Result<Option<Upgraded>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider,
    {
        if !rule_upgrade_open() {
            return Ok(None);
        }
        let session = self.acquire(env).await?;
        if !session.state.claim_rules_upgrade() {
            return Ok(None);
        }
        let upgraded = {
            let _transacting = session.state.transactor().lock().await;
            let definitions = tonk_schema::upgrade::upgrade_definitions(&session.state.branch, env)
                .await
                .map_err(|reason| ReactorError::Upgrade { reason })?;
            let rules = session.state.branch.upgrade_rules().perform(env).await?;
            Upgraded { definitions, rules }
        };
        if !upgraded.definitions.is_empty() || upgraded.rules.revision.is_some() {
            self.reactor().schedule_poll(Arc::clone(&session.state));
        }
        Ok(Some(upgraded))
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
