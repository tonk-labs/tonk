//! [`Pull`] — wrap [`dialog_repository::Branch::pull`] so
//! subscriptions re-poll once remote artifacts have been
//! merged into the branch.

use dialog_repository::Revision;

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PullProvider, SelectProvider};
use super::error::ReactorError;

/// Pull-from-upstream effect.
pub struct Pull<'a> {
    /// The branch to pull into.
    pub branch: BranchReference<'a>,
    /// Materialize the adopted revision locally after the pull.
    download: bool,
    /// Restrict that materialization to the operational regions.
    operational: bool,
    /// Whether to ask a remote upstream where it stands before merging.
    confirm_upstream: bool,
}

impl<'a> Pull<'a> {
    /// Build a new `Pull` effect.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            download: false,
            operational: false,
            confirm_upstream: true,
        }
    }

    /// Merge the head last recorded for each remote upstream instead of
    /// asking it where it stands, for a caller that learns of heads as
    /// they move. See [`dialog_repository::Pull::assuming_upstream`]:
    /// nothing is risked but staleness.
    pub fn assuming_upstream(mut self) -> Self {
        self.confirm_upstream = false;
        self
    }

    /// Materialize every block and blob the adopted revision references
    /// before the head moves to it. Required for authorization-bearing
    /// branches (the access branch, the account): their walks read
    /// entirely locally, so a head adopted by reference with blocks still
    /// remote bricks the next boot.
    ///
    /// The download is ordered before the cell advance, as
    /// [`dialog_repository::Pull::download`] orders it: a download that
    /// fails, or a worker stopped before it completes, leaves the head
    /// where it was, never on blocks the store lacks.
    pub fn download(mut self) -> Self {
        self.download = true;
        self
    }

    /// Narrow the download to the regions a read can reach: the
    /// entity/attribute/value indexes and the blob index.
    ///
    /// That is every fact the branch holds — every delegation included —
    /// so an authorization walk still resolves entirely locally. What it
    /// leaves by reference is history and coverage, which no read path
    /// can reach and which grow with every edit ever made rather than
    /// with the live fact count. A history record that does turn out to
    /// be needed hydrates from the upstream on demand.
    ///
    /// No effect without [`download`](Self::download).
    pub fn operational(mut self) -> Self {
        self.operational = true;
        self
    }

    /// Execute the pull. Subscriptions are re-evaluated **only when the pull
    /// actually moved the branch tree** — a pull that finds no upstream change
    /// (the common case for a periodic background sync) leaves every query
    /// result identical, so re-polling would be pure waste. Skipping it is the
    /// difference between an idle sync being invisible and it re-running every
    /// subscription on the branch on every tick.
    ///
    /// Two phases: the network-bound fetch + rebase
    /// ([`prepare`](dialog_repository::PreparedPull)), and with
    /// [`download`](Self::download) the download, run lock-free, then the
    /// instant cell advance ([`commit`](dialog_repository::PreparedPull::commit))
    /// runs under the per-branch transactor lock — the same lock commits take.
    /// So a sync's advance and a commit serialize cleanly on the one contended
    /// step, while the expensive merge stays concurrent. The transactor lock is
    /// held only for the microsecond cell writes, so a transaction never waits
    /// on a sync's network round trip.
    pub async fn perform<Env>(self, env: &Env) -> Result<Option<Revision>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + PullProvider + SelectProvider,
    {
        // The profile's own repository holds the delegations that authorize
        // this device's reads, so whoever pulls it, the pull downloads what
        // it adopts: adopted by reference, a fast-forward's root is never
        // read, and the walk that would authorize fetching it reads the
        // branch it names.
        let (download, operational) = if self.branch.repository.name()
            == self.branch.reactor().profile_key()
        {
            if !self.download {
                dialog_common::log!(
                    "pull of the profile's branch {} asked for no download; downloading what it adopts",
                    self.branch.name
                );
            }
            (true, self.operational || !self.download)
        } else {
            (self.download, self.operational)
        };

        let cached = self.branch.acquire(env).await?;

        // The tree we're at before the pull. Compared against the post-commit
        // revision to decide whether anything actually changed.
        let before = cached.handle().revision().map(|r| r.tree);

        // Fetch + rebase + persist blocks — no cell writes, no lock.
        let pull = cached.handle().pull();
        let pull = if self.confirm_upstream {
            pull
        } else {
            pull.assuming_upstream()
        };
        let prepared = pull.prepare(env).await?;

        // Materialize what the advance would adopt before it is adopted: a
        // head that moved first, with its blocks still remote, is a branch
        // whose authorization walk cannot read itself if the download then
        // fails or never runs. With nothing new to adopt, the current head
        // is materialized instead.
        if download {
            let download = cached.handle().download();
            let download = match prepared.revision() {
                Some(revision) => download.of(revision.clone()),
                None => download,
            };
            let download = if operational {
                download.operational()
            } else {
                download
            };
            download.perform(env).await?;
        }

        // Advance the cells under the transactor lock, so this serializes with
        // commits on the same branch instead of racing the head CAS.
        let revision = {
            let _advancing = cached.state.transactor().lock().await;
            prepared.commit(env).await?
        };

        // Re-poll subscriptions only if the tree moved. A `None` revision (a
        // no-op pull) or a merge that netted the same tree changes no query
        // result, so the poll is skipped — an idle background sync does no
        // subscription work at all.
        let changed = match &revision {
            Some(after) => before.as_ref() != Some(&after.tree),
            None => false,
        };
        if changed {
            cached.poll(env).await;
        }

        Ok(revision)
    }
}
