//! A read-only snapshot of what the reactor is currently doing.
//!
//! The reactor's caches — repositories, their branches, and each branch's
//! subscription map — are the authority on which queries are live and who is
//! listening to them, but nothing outside the poll path could see them. This
//! module is the window: it walks those maps under their locks, copies out
//! plain owned values, and releases them.
//!
//! Deliberately a *snapshot*, not a live view. It holds no lock past the walk
//! and hands back no handles, so a caller can render it at leisure without
//! blocking a poll, a commit, or a subscriber attaching. What it describes was
//! true at the instant it was taken and may already have changed — which is
//! the honest shape for introspection, and why the console re-reads rather
//! than diffing.
//!
//! Everything here describes THIS process. A subscription lives in one
//! worker's memory; nothing in this module reaches another device.

use crate::Query;

/// One live subscription: a `(branch, query)` pair and the subscribers
/// attached to it.
#[derive(Debug, Clone)]
pub struct SubscriptionSnapshot {
    /// The repository the subscription's branch belongs to. The profile
    /// repository reports [`PROFILE_REPOSITORY`].
    pub repository: String,
    /// The branch within that repository.
    pub branch: String,
    /// The subscription's identity within the branch, hex-encoded.
    pub hash: String,
    /// The subscribed query, in the wire form a `/query` request carries.
    pub query: Query,
    /// How many subscriber channels are attached.
    pub subscribers: usize,
    /// How many of those have not yet been served their first snapshot.
    /// A subscription that keeps pending subscribers is one whose poll is
    /// not landing — the shape of a hung display.
    pub pending: usize,
    /// When the subscription was opened, in milliseconds since the Unix
    /// epoch.
    pub opened_at_ms: u64,
    /// How many updates this subscription has pushed since it opened —
    /// deltas that carried a real change, not the per-subscriber initial
    /// snapshot.
    pub updates: u64,
    /// When the last update was pushed, epoch milliseconds. `None` if the
    /// query has not changed since it was opened.
    pub last_update_ms: Option<u64>,
    /// Total bytes pushed to subscribers since the subscription opened.
    pub bytes_pushed: u64,
}

/// The repository name reported for the profile, which has no name in the
/// routing namespace (it is addressed as `profile:<name>`, not by key).
pub const PROFILE_REPOSITORY: &str = "profile";

impl crate::Reactor {
    /// Snapshot every live subscription across every cached repository and
    /// branch, including the profile's.
    ///
    /// Grouped by `(repository, branch)`, newest subscription first within
    /// each group, hash-tiebroken. The order is total and derived only from
    /// the data, so an unchanged reactor renders an unchanged list rather
    /// than shuffling with `HashMap` iteration order.
    pub fn subscription_snapshot(&self) -> Vec<SubscriptionSnapshot> {
        let mut out = Vec::new();

        // Named repositories. The locks are taken one level at a time and
        // the handles cloned out, so the walk never holds a repository lock
        // while taking a branch's.
        let repos: Vec<(String, std::sync::Arc<crate::RepositoryState>)> = {
            let map = self.repos().read();
            map.iter()
                .map(|(name, state)| (name.clone(), std::sync::Arc::clone(state)))
                .collect()
        };
        for (name, repo) in repos {
            collect_repository(&name, &repo, &mut out);
        }

        // The profile repository sits outside `repos` (it is a singleton with
        // no routing name), so it is walked separately or its subscriptions —
        // the Hub's, and the console's own — would be missing entirely.
        let profile = self.profile_repo_state();
        if let Some(repo) = profile {
            collect_repository(PROFILE_REPOSITORY, &repo, &mut out);
        }

        // Group by (repository, branch), and within a group put the NEWEST
        // subscription first: the console is read to see what just happened,
        // and the row you want is the one that just appeared. Ties fall back
        // to the hash so the order is total and rows never shuffle between
        // refreshes of an unchanged reactor.
        out.sort_by(|a, b| {
            (&a.repository, &a.branch)
                .cmp(&(&b.repository, &b.branch))
                .then(b.opened_at_ms.cmp(&a.opened_at_ms))
                .then(a.hash.cmp(&b.hash))
        });
        out
    }
}

/// Walk one repository's branches, appending a snapshot per subscription.
fn collect_repository(
    name: &str,
    repo: &crate::RepositoryState,
    out: &mut Vec<SubscriptionSnapshot>,
) {
    let branches: Vec<(String, std::sync::Arc<crate::BranchState>)> = {
        let map = repo.branches().read();
        map.iter()
            .map(|(branch, state)| (branch.clone(), std::sync::Arc::clone(state)))
            .collect()
    };

    for (branch, state) in branches {
        let subs = state.subscriptions().lock();
        for (hash, subscription) in subs.iter() {
            out.push(SubscriptionSnapshot {
                repository: name.to_owned(),
                branch: branch.clone(),
                hash: hash.to_hex(),
                query: subscription.query.clone(),
                subscribers: subscription.subscribers.len(),
                pending: subscription
                    .subscribers
                    .iter()
                    .filter(|s| s.is_pending())
                    .count(),
                opened_at_ms: subscription.opened_at_ms,
                updates: subscription.updates,
                last_update_ms: subscription.last_update_ms,
                bytes_pushed: subscription.bytes_pushed,
            });
        }
    }
}
