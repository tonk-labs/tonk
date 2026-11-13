//! [`Pull`] — wrap [`dialog_repository::Branch::pull`] so
//! subscriptions re-poll once remote artifacts have been
//! merged into the branch.

use dialog_capability::Provider;
use dialog_common::{Blake3Hash, ConditionalSync};
use dialog_corpus::DialogCorpusError;
use dialog_effects::archive::Load;
use dialog_repository::{Branch, Download, PullError, Revision};

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PullProvider, SelectProvider};
use super::error::ReactorError;

/// Pull-from-upstream effect.
pub struct Pull<'a> {
    /// The branch to pull into.
    pub branch: BranchReference<'a>,
    /// What the pull brings local of the head it would adopt, before the
    /// head moves there.
    download: Download,
    /// Whether to ask a remote upstream where it stands before merging.
    confirm_upstream: bool,
}

impl<'a> Pull<'a> {
    /// Build a new `Pull` effect that brings `download` local of the head
    /// it would adopt, before the head moves there. See
    /// [`dialog_repository::Branch::pull`]: a download that fails, or a
    /// worker stopped before it completes, leaves the head where it was,
    /// never on blocks the download named and the store lacks.
    ///
    /// Authorization-bearing branches (the access branch, the account)
    /// take [`Download::Operational`]: their walks read entirely locally,
    /// so a head adopted by reference with blocks still remote bricks the
    /// next boot. A space takes [`Download::QueriedRanges`], so that what
    /// a page read or an edit wrote from this device's store it still
    /// reads and writes from it after the pull.
    pub fn new(branch: BranchReference<'a>, download: Download) -> Self {
        Self {
            branch,
            download,
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
        // reads and authorization touch: adopted by reference, a
        // fast-forward's root is never read, and the walk that would
        // authorize fetching it reads the branch it names.
        let download = if self.branch.repository.name() == self.branch.reactor().profile_key()
            && !matches!(self.download, Download::Operational | Download::Everything)
        {
            dialog_common::log!(
                "pull of the profile's branch {} asked for {:?}; downloading its operational regions",
                self.branch.name,
                self.download
            );
            Download::Operational
        } else {
            self.download
        };

        let cached = self.branch.acquire(env).await?;

        // The tree we're at before the pull. Compared against the post-commit
        // revision to decide whether anything actually changed.
        let before = cached.handle().revision().map(|r| r.tree);

        // Fetch + rebase + persist blocks — no cell writes, no lock.
        // The download runs in `prepare`, between the merge and any cell
        // write.
        let pull = cached.handle().pull(download);
        let pull = if self.confirm_upstream {
            pull
        } else {
            pull.assuming_upstream()
        };
        let prepared = match pull.prepare(env).await {
            Ok(prepared) => prepared,
            // Since dialog 0.4.0-rc.4 preparing a pull reads the upstream
            // head's tree through the env (its context region, to order
            // the two heads), so a head whose blocks this replica can no
            // longer fetch fails here, before `hold_root` would refuse it.
            // It is the same refusal: nothing was adopted and the head
            // stays where it was.
            Err(PullError::Fact(error)) => {
                return Err(ReactorError::UnreadableHead {
                    branch: self.branch.name.to_string(),
                    root: unreadable_block(&error),
                    reason: error.to_string(),
                });
            }
            Err(error) => return Err(error.into()),
        };

        // Hold the root of what the advance would adopt before adopting
        // it. A fast-forward adopts by reference and never reads the root,
        // so a head this replica can no longer fetch (its grant was
        // revoked after a watch told it of the head, say) would otherwise
        // become the branch's head, and every later read and commit of
        // the branch would fail on that root. Refused here, the head
        // stays where it was and the replica keeps working on what it
        // holds.
        if let Some(revision) = prepared.revision()
            && before.as_ref() != Some(&revision.tree)
        {
            hold_root(cached.handle(), revision, env).await?;
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

/// The block an artifact read named as unreadable, from the message a
/// hydration failure carries (`block <hash> is not in the local archive
/// ...`), or `unknown` when the error names none.
fn unreadable_block(error: &DialogCorpusError) -> String {
    let text = error.to_string();
    text.split_whitespace()
        .skip_while(|word| *word != "block")
        .nth(1)
        .map(|hash| hash.trim_end_matches(':').to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Read the root block of `revision`'s tree into `branch`'s archive: a
/// local hit when it is already held, else a fetch from the upstream the
/// pull prepared from, which is the first the branch pulls from (see
/// [`dialog_repository::Pull::prepare`]).
///
/// Only the root: the rest of a tree adopted by reference hydrates as
/// reads reach it, and the root is what every read and commit of the
/// branch reads first.
async fn hold_root<Env>(branch: &Branch, revision: &Revision, env: &Env) -> Result<(), ReactorError>
where
    Env: Provider<Load> + ConditionalSync,
{
    // The load hydrates a miss from the remotes the branch registered as
    // it opened (its pulls first), and fails naming one it cannot reach.
    let root = Blake3Hash::from(*revision.tree.hash());
    match Load::new(branch.archive().index(), root.clone())
        .perform(env)
        .await
    {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ReactorError::UnreadableHead {
            branch: branch.name().to_string(),
            root: root.to_string(),
            reason: "the block is neither held nor reachable".to_string(),
        }),
        Err(error) => Err(ReactorError::UnreadableHead {
            branch: branch.name().to_string(),
            root: root.to_string(),
            reason: error.to_string(),
        }),
    }
}
