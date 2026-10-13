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

/// What [`BranchReference::migrate_once`] moved.
#[derive(Debug, Clone)]
pub struct Upgraded {
    /// The revision the storage rewrite published: the head's tree laid
    /// out in the number band this release writes.
    pub storage: Option<dialog_repository::Revision>,
    /// The attribute and concept definitions written again.
    pub definitions: tonk_schema::upgrade::DefinitionsUpgraded,
    /// The rules re-installed.
    pub rules: dialog_repository::RulesUpgraded,
}

/// Move what an earlier release stored in `branch` to the shape this
/// release reads and writes, if its head is laid out the earlier way: the
/// storage rewrite under the number band ([`Branch::migrate_storage`]),
/// then the attribute and concept definitions recorded the earlier way
/// ([`tonk_schema::upgrade::upgrade_definitions`]), then the rules stored
/// under an identity an earlier dialog release gave them
/// ([`Branch::upgrade_rules`]). `None` when the head is already in this
/// release's layout, after one root read ([`Branch::needs_migration`]).
///
/// For a write into a branch opened outside the reactor (a meta branch a
/// repository route records into, a blob import, the CLI's account branch),
/// which the gate in [`BranchReference::migrate_once`] does not cover: a
/// commit into a tree laid out the earlier way is refused, so run this
/// first. Through the reactor, prefer `migrate_once`, which also runs once
/// per process and tells pages that the upgrade is taking place.
///
/// [`Branch::migrate_storage`]: dialog_repository::Branch::migrate_storage
/// [`Branch::needs_migration`]: dialog_repository::Branch::needs_migration
/// [`Branch::upgrade_rules`]: dialog_repository::Branch::upgrade_rules
pub async fn migrate<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
) -> Result<Option<Upgraded>, ReactorError>
where
    Env: LoadProvider + BranchOpenProvider + CommitProvider,
{
    let needed = branch
        .needs_migration(env)
        .await
        .map_err(|error| ReactorError::Upgrade {
            reason: error.to_string(),
        })?;
    if !needed {
        return Ok(None);
    }
    migrate_branch(branch, env).await.map(Some)
}

/// The three steps of a migration, unconditionally.
async fn migrate_branch<Env>(
    branch: &dialog_repository::Branch,
    env: &Env,
) -> Result<Upgraded, ReactorError>
where
    Env: LoadProvider + BranchOpenProvider + CommitProvider,
{
    let storage = branch.migrate_storage().perform(env).await?;
    let definitions = tonk_schema::upgrade::upgrade_definitions(branch, env)
        .await
        .map_err(|reason| ReactorError::Upgrade { reason })?;
    let rules = branch.upgrade_rules().perform(env).await?;
    Ok(Upgraded {
        storage,
        definitions,
        rules,
    })
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

    /// Move what an earlier release stored to the shape this one reads
    /// and writes, once per branch while the reactor holds it open: the
    /// storage rewrite under the number band
    /// ([`Branch::migrate_storage`]), then the attribute and concept
    /// definitions recorded the earlier way
    /// ([`tonk_schema::upgrade::upgrade_definitions`]), then the rules
    /// stored under an identity an earlier dialog release gave them
    /// ([`Branch::upgrade_rules`]). Everything runs under the branch's
    /// transactor lock, as a commit does, so a write that reaches the
    /// branch meanwhile waits for it.
    ///
    /// Gated on the head's storage layout ([`Branch::needs_migration`]):
    /// a branch this release wrote has nothing an earlier release left, so
    /// the gate is one root read, paid by every call while nothing has
    /// run, so a head laid out the earlier way that a pull adopts later in
    /// the process is still moved. Once a migration has run, a later call
    /// returns `None` at once; a second caller during the migration waits
    /// for it and then returns `None`. A failed migration is not retried
    /// until the branch is opened again (the next worker or CLI start): a
    /// cause like an unreachable remote would otherwise repeat on every
    /// read. Schedules a poll when it commits.
    ///
    /// Every commit through the reactor runs this first (see
    /// [`crate::transaction::Commit`]), since a commit into a tree laid
    /// out the earlier way is refused; a read does not need it, and the
    /// worker's query route runs it to tell the page what is happening.
    ///
    /// [`Branch::migrate_storage`]: dialog_repository::Branch::migrate_storage
    /// [`Branch::needs_migration`]: dialog_repository::Branch::needs_migration
    /// [`Branch::upgrade_rules`]: dialog_repository::Branch::upgrade_rules
    pub async fn migrate_once<Env>(&self, env: &Env) -> Result<Option<Upgraded>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider,
    {
        let session = self.acquire(env).await?;
        // Under the lock first: a caller arriving while the migration
        // runs waits here, and then finds it has run.
        let _transacting = session.state.transactor().lock().await;
        if session.state.migration_claimed() {
            return Ok(None);
        }
        let branch = &session.state.branch;
        let needed = branch
            .needs_migration(env)
            .await
            .map_err(|error| ReactorError::Upgrade {
                reason: error.to_string(),
            })?;
        if !needed {
            return Ok(None);
        }
        // Claimed only now: a branch with nothing to move stays open to a
        // later head that has.
        session.state.claim_migration();
        session.state.set_migrating(true);
        let outcome = migrate_branch(branch, env).await;
        session.state.set_migrating(false);
        let upgraded = outcome?;
        self.reactor().schedule_poll(Arc::clone(&session.state));
        Ok(Some(upgraded))
    }

    /// Whether [`migrate_once`](Self::migrate_once) would move anything,
    /// or is moving it now: the branch's head is laid out the way an
    /// earlier release wrote and nothing has migrated it yet in this
    /// process. One root read when the branch is open; `false` once the
    /// migration has run. A page reads this to show that an upgrade is
    /// taking place before it waits for its rows.
    pub async fn migration_pending<Env>(&self, env: &Env) -> Result<bool, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider,
    {
        let session = self.acquire(env).await?;
        if session.state.is_migrating() {
            return Ok(true);
        }
        if session.state.migration_claimed() {
            return Ok(false);
        }
        session
            .state
            .branch
            .needs_migration(env)
            .await
            .map_err(|error| ReactorError::Upgrade {
                reason: error.to_string(),
            })
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
