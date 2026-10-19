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

/// The whole migration, as the [`Upgrade`] a profile is opened with.
///
/// A profile's state branch is its account branch, which keeps the
/// profile library's definitions, and onboarding writes it directly when
/// the profile opens. Moving only its storage there would leave the later
/// [`migrate`] gate, which asks only about storage, nothing to do, and
/// the definitions under their earlier identities. So this moves the
/// branch when its storage or any of its definitions is in the earlier
/// form, and a move an earlier open left half done is finished.
///
/// [`Upgrade`]: tonk_account::peer::Upgrade
pub struct ProfileUpgrade;

impl<S> tonk_account::peer::Upgrade<S> for ProfileUpgrade
where
    S: dialog_peer::PeerSpace,
    dialog_peer::Peer<S>: LoadProvider + BranchOpenProvider + CommitProvider,
{
    fn upgrade<'a>(
        &'a self,
        branch: &'a dialog_repository::Branch,
        peer: &'a dialog_peer::Peer<S>,
    ) -> dialog_peer::StepFuture<'a> {
        Box::pin(async move {
            let failed = |error: ReactorError| dialog_peer::PeerError::Upgrade(error.to_string());
            let needed = branch
                .needs_migration(peer)
                .await
                .map_err(|error| failed(error.into()))?
                || tonk_schema::upgrade::has_legacy_definitions(branch, peer)
                    .await
                    .map_err(|reason| failed(ReactorError::Upgrade { reason }))?;
            if !needed {
                return Ok(());
            }
            migrate_branch(branch, peer)
                .await
                .map(|_| ())
                .map_err(failed)
        })
    }
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
        let branch = &session.state.branch;
        let needs_migration = || async {
            branch
                .needs_migration(env)
                .await
                .map_err(|error| ReactorError::Upgrade {
                    reason: error.to_string(),
                })
        };
        // Without the lock first: a branch already moved, or one with
        // nothing to move, is the common case, and a read that only
        // asks must not wait on a writer holding the lock.
        if session.state.migration_claimed() || !needs_migration().await? {
            return Ok(None);
        }
        // Under the lock: a caller arriving while the migration runs
        // waits here, and then finds it has run.
        let _transacting = session.state.transactor().lock().await;
        if session.state.migration_claimed() || !needs_migration().await? {
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::ProfileUpgrade;
    use dialog_artifacts::{Entity, Pick, Relation, Statement, Update, Value};
    use dialog_capability::Principal as _;
    use dialog_effects::storage::{Directory, Location};
    use dialog_peer::{Peer, PeerError, StepFuture};
    use dialog_repository::{Branch, Repository};
    use dialog_storage::provider::storage::{NativeSpace, Storage};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tonk_account::peer::{Upgrade, open_peer, open_system};

    /// A claim by its raw relation, as the earlier release recorded
    /// definitions.
    struct Raw {
        the: Relation,
        of: Entity,
        is: Value,
    }

    impl Statement for Raw {
        fn assert(self, update: &mut impl Update) {
            update.associate(self.the, self.of, self.is, Pick::Last);
        }

        fn retract(self, update: &mut impl Update) {
            update.dissociate(self.the, self.of, self.is);
        }
    }

    /// Open the profile `tonk` kept in `directory`, moving its state with
    /// `upgrade`.
    async fn open(
        directory: &Directory,
        upgrade: &dyn Upgrade<NativeSpace>,
    ) -> anyhow::Result<Peer<NativeSpace>> {
        let (credentials, system) = open_system::<NativeSpace>(directory.clone()).await?;
        Ok(open_peer(
            Location::new(directory.clone(), "tonk"),
            directory.clone(),
            Storage::<NativeSpace>::default().owned_by(system.did()),
            &credentials,
            &system,
            true,
            upgrade,
        )
        .await?)
    }

    /// Leave in `directory` a profile as an earlier release wrote it: a
    /// library definition recorded that release's way, on a branch laid
    /// out before the number band.
    async fn write_earlier_profile(directory: &Directory) -> anyhow::Result<()> {
        let peer = open(directory, &ProfileUpgrade).await?;
        let attribute: dialog_query::AttributeDescriptor = serde_json::from_value(
            serde_json::json!({ "the": "xyz.tonk.note/title", "as": "text:" }),
        )?;
        let earlier: Entity = dialog_query::migration::attribute_uri_v0(&attribute).parse()?;
        let mut transaction = peer.state().transaction();
        for (the, is) in [
            ("db.attribute/id", "xyz.tonk.note/title"),
            ("db.attribute/type", "Text"),
            ("db.attribute/cardinality", "one"),
        ] {
            transaction = transaction.assert(Raw {
                the: the.parse()?,
                of: earlier.clone(),
                is: Value::String(is.into()),
            });
        }
        transaction.commit().publish().perform(&peer).await?;
        peer.state().rekey_storage(0).perform(&peer).await?;
        Ok(())
    }

    /// Whether `peer`'s state branch is wholly in this release's form.
    async fn assert_moved(peer: &Peer<NativeSpace>) -> anyhow::Result<()> {
        assert!(!peer.state().needs_migration(peer).await?);
        let left = tonk_schema::upgrade::upgrade_definitions(peer.state(), peer)
            .await
            .map_err(anyhow::Error::msg)?;
        assert!(
            left.is_empty(),
            "the definitions were upgraded when the profile opened: {left:?}"
        );
        Ok(())
    }

    /// Moves the branch through a handle of its own, as another process
    /// opening the same profile does, and then loses its own commit once.
    struct Raced(AtomicBool);

    impl Upgrade<NativeSpace> for Raced {
        fn upgrade<'a>(
            &'a self,
            branch: &'a Branch,
            peer: &'a Peer<NativeSpace>,
        ) -> StepFuture<'a> {
            Box::pin(async move {
                if self.0.swap(true, Ordering::SeqCst) {
                    return ProfileUpgrade.upgrade(branch, peer).await;
                }
                let other = Repository::from(peer.did())
                    .branch(branch.name())
                    .open()
                    .perform(peer)
                    .await
                    .map_err(|error| PeerError::Upgrade(error.to_string()))?;
                ProfileUpgrade.upgrade(&other, peer).await?;
                Err(PeerError::Upgrade("version mismatch".into()))
            })
        }
    }

    /// Rewrites the storage and then fails, as a move interrupted after
    /// the rewrite does.
    struct Stranded;

    impl Upgrade<NativeSpace> for Stranded {
        fn upgrade<'a>(
            &'a self,
            branch: &'a Branch,
            peer: &'a Peer<NativeSpace>,
        ) -> StepFuture<'a> {
            Box::pin(async move {
                branch
                    .migrate_storage()
                    .perform(peer)
                    .await
                    .map_err(|error| PeerError::Upgrade(error.to_string()))?;
                Err(PeerError::Upgrade("the definitions did not move".into()))
            })
        }
    }

    /// A profile an earlier release wrote keeps library definitions on its
    /// state branch in that release's form. Opening it moves the storage
    /// and the definitions together, so the later storage-gated
    /// [`super::migrate`] finding nothing to do leaves nothing behind.
    #[dialog_common::test]
    async fn it_upgrades_the_definitions_of_a_profile_an_earlier_release_wrote()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        write_earlier_profile(&directory).await?;

        assert_moved(&open(&directory, &ProfileUpgrade).await?).await
    }

    /// A move that rewrites the storage and then fails leaves a branch the
    /// storage gate no longer flags. Opening reports the failure, and the
    /// next open finishes the move from the definitions still left.
    #[dialog_common::test]
    async fn it_finishes_a_move_an_earlier_open_left_half_done() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        write_earlier_profile(&directory).await?;

        let Err(error) = open(&directory, &Stranded).await else {
            panic!("opening reports the failed move");
        };
        assert!(
            error.to_string().contains("the definitions did not move"),
            "{error}"
        );

        assert_moved(&open(&directory, &ProfileUpgrade).await?).await
    }

    /// Two processes opening a profile an earlier release wrote both find
    /// it needing to move, and only one move lands. The other opens the
    /// moved profile instead of failing.
    #[dialog_common::test]
    async fn it_opens_a_profile_another_process_migrated_first() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let directory = Directory::At(temp.path().to_string_lossy().into_owned());
        write_earlier_profile(&directory).await?;

        let peer = open(&directory, &Raced(AtomicBool::new(false))).await?;
        assert_moved(&peer).await?;
        peer.secrets()
            .site("tonk-after-race-v1")
            .save(b"kept".to_vec())
            .perform(&peer)
            .await?;
        Ok(())
    }
}
