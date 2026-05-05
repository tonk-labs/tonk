//! [`TonkReactor`] — the worker's reactive layer over branches.
//!
//! See `reactor-spec.md` next to the crate's `Cargo.toml` for the
//! full design rationale. The short version: routes mutate
//! branches through a chain (`reactor.repository(r).branch(b)
//! .transaction().assert(…).commit().perform(&op).await?`); the
//! leaf effect's `perform` re-evaluates every subscription on the
//! branch and broadcasts the result to whoever is listening,
//! deduplicating by hash so unchanged results don't fire spurious
//! broadcasts.
//!
//! The reactor caches `Repository` and `Branch` handles — first
//! reference opens, subsequent references reuse — so per-request
//! load+open overhead is paid once per repo/branch lifetime.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use dialog_common::Blake3Hash;
use dialog_operator::Profile;
use dialog_query::{ConceptConclusion, ConceptQuery, Output as _};
use dialog_repository::{Branch, Repository, RepositoryExt as _};
use tokio::sync::{Mutex, mpsc};
use tonk_common::log;

use crate::worker::DefaultOperator;

mod error;
mod query;
mod repository;
mod subscribe;
mod subscription;
mod wire;

pub use error::ReactorError;
pub use query::Query;
pub use repository::{BranchHandle, RepositoryHandle};
pub use subscribe::Subscribe;
pub use wire::{WireConclusion, WireQuery};

use subscription::{QueryHash, Status, Subscriber, Subscription};

/// Cached repository handle plus its branches.
pub(crate) struct RepoEntry {
    /// `Arc` so the cache hands out a shared reference without
    /// cloning the underlying credential. Held even when no
    /// branch lookup needs it directly so a future `branch(name)`
    /// against the same repo skips the load step.
    #[allow(dead_code)]
    repository: Arc<Repository>,
    branches: HashMap<String, BranchEntry>,
}

/// Cached branch handle plus the subscriptions registered against
/// it.
pub(crate) struct BranchEntry {
    branch: Branch,
    subscriptions: HashMap<QueryHash, Subscription>,
}

/// The worker's reactive layer. Owned by `TonkState`.
pub struct TonkReactor {
    profile: Profile,
    repos: Mutex<HashMap<String, RepoEntry>>,
}

impl TonkReactor {
    /// Construct a reactor over the given profile. The reactor
    /// doesn't own an operator — every effect takes one at
    /// `perform` time, matching dialog's command/perform pattern.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            repos: Mutex::new(HashMap::new()),
        }
    }

    /// Drop every cached handle so open SSE response bodies
    /// finish. Called from the SW upgrade path so the old worker
    /// can be replaced.
    pub async fn shutdown(&self) {
        self.repos.lock().await.clear();
    }

    /// Begin a chain scoped to the named repository.
    pub fn repository<'a>(&'a self, name: &'a str) -> RepositoryHandle<'a> {
        RepositoryHandle::new(self, name)
    }
}

// ---------------------------------------------------------------- //
// Internal: branch resolution + polling                            //
// ---------------------------------------------------------------- //

impl TonkReactor {
    /// Resolve a `(repo, branch)` pair against the cache, opening
    /// either or both on first reference. Returns a clone of the
    /// branch handle (cheap — `Branch` is `Clone`).
    pub(crate) async fn resolve_branch(
        &self,
        repo: &str,
        branch: &str,
        env: &DefaultOperator,
    ) -> Result<Branch, ReactorError> {
        // Fast path: already cached.
        {
            let repos = self.repos.lock().await;
            if let Some(repo_entry) = repos.get(repo)
                && let Some(branch_entry) = repo_entry.branches.get(branch)
            {
                return Ok(branch_entry.branch.clone());
            }
        }

        // Slow path: open whichever level is missing. Opens
        // happen outside the lock — `repository().load()` and
        // `branch().open()` are async.
        let repository = self
            .profile
            .repository(repo)
            .load()
            .perform(env)
            .await
            .map_err(|e| ReactorError::RepositoryNotFound {
                repo: repo.to_owned(),
                reason: e.to_string(),
            })?;
        let repository = Arc::new(repository);

        let branch_handle = repository
            .branch(branch)
            .open()
            .perform(env)
            .await
            .map_err(|e| ReactorError::BranchNotFound {
                repo: repo.to_owned(),
                branch: branch.to_owned(),
                reason: e.to_string(),
            })?;

        // Insert under the lock — another caller may have raced
        // and inserted; we use their entry rather than overwriting.
        let mut repos = self.repos.lock().await;
        let repo_entry = repos.entry(repo.to_owned()).or_insert_with(|| RepoEntry {
            repository: Arc::clone(&repository),
            branches: HashMap::new(),
        });
        let branch_entry = repo_entry
            .branches
            .entry(branch.to_owned())
            .or_insert_with(|| BranchEntry {
                branch: branch_handle.clone(),
                subscriptions: HashMap::new(),
            });
        Ok(branch_entry.branch.clone())
    }

    /// Attach a fresh subscriber to the subscription identified
    /// by `(repo, branch, query)`. Creates the subscription if
    /// it doesn't exist. Returns the subscription's hash and the
    /// new receiver so the caller can poll and frame the body.
    pub(crate) async fn attach_subscriber(
        &self,
        repo: &str,
        branch: &str,
        query: ConceptQuery,
    ) -> Result<(QueryHash, mpsc::UnboundedReceiver<Bytes>), ReactorError> {
        let hash = QueryHash::of(&query);
        let (sender, receiver) = mpsc::unbounded_channel();

        let mut repos = self.repos.lock().await;
        let repo_entry = repos.get_mut(repo).expect("repository was just resolved");
        let branch_entry = repo_entry
            .branches
            .get_mut(branch)
            .expect("branch was just resolved");

        let entry = branch_entry.subscriptions.entry(hash.clone());
        let subscription = entry.or_insert_with(|| Subscription {
            query: query.clone(),
            last_hash: None,
            subscribers: Vec::new(),
        });
        if subscription.query != query {
            return Err(ReactorError::QueryHashCollision);
        }
        subscription.subscribers.push(Subscriber {
            sender,
            status: Status::Pending,
        });

        Ok((hash, receiver))
    }

    /// Poll one subscription: re-evaluate, decide who receives
    /// the bytes (Pending vs Established), update `last_hash`,
    /// drop dead subscribers, drop the subscription if empty.
    pub(crate) async fn poll_subscription(
        &self,
        repo: &str,
        branch: &str,
        hash: QueryHash,
        branch_handle: &Branch,
        env: &DefaultOperator,
    ) {
        // Snapshot the query out of the lock so we can run it
        // without holding the cache mutex across await.
        let query = {
            let repos = self.repos.lock().await;
            let Some(repo_entry) = repos.get(repo) else {
                return;
            };
            let Some(branch_entry) = repo_entry.branches.get(branch) else {
                return;
            };
            let Some(subscription) = branch_entry.subscriptions.get(&hash) else {
                return;
            };
            subscription.query.clone()
        };

        let conclusions = match run_query(branch_handle, query, env).await {
            Ok(c) => c,
            Err(err) => {
                log!("[reactor] subscription poll failed: {err}");
                return;
            }
        };

        let wire: Vec<WireConclusion> = conclusions.iter().map(WireConclusion::from).collect();
        let bytes = match serde_json::to_vec(&wire) {
            Ok(b) => Bytes::from(b),
            Err(err) => {
                log!("[reactor] failed to serialize conclusions: {err}");
                return;
            }
        };
        let new_hash = Blake3Hash::hash(&bytes);

        let mut repos = self.repos.lock().await;
        let Some(repo_entry) = repos.get_mut(repo) else {
            return;
        };
        let Some(branch_entry) = repo_entry.branches.get_mut(branch) else {
            return;
        };
        let Some(subscription) = branch_entry.subscriptions.get_mut(&hash) else {
            return;
        };

        let changed = subscription.last_hash.as_ref() != Some(&new_hash);
        if changed {
            subscription.last_hash = Some(new_hash);
        }

        // Walk subscribers, sending where required, dropping any
        // whose receiver closed.
        subscription.subscribers.retain_mut(|sub| {
            let needs_send = changed || sub.status == Status::Pending;
            if !needs_send {
                return true;
            }
            match sub.sender.send(bytes.clone()) {
                Ok(()) => {
                    sub.status = Status::Established;
                    true
                }
                Err(_) => false,
            }
        });

        if subscription.subscribers.is_empty() {
            branch_entry.subscriptions.remove(&hash);
        }
    }

    /// Re-poll every subscription on a branch. Called from the
    /// success path of mutating effects (commit/pull/sync) once
    /// the chain wraps them.
    #[allow(dead_code)]
    pub(crate) async fn poll_all(&self, repo: &str, branch: &str, env: &DefaultOperator) {
        let (branch_handle, hashes) = {
            let repos = self.repos.lock().await;
            let Some(repo_entry) = repos.get(repo) else {
                return;
            };
            let Some(branch_entry) = repo_entry.branches.get(branch) else {
                return;
            };
            let hashes: Vec<QueryHash> = branch_entry.subscriptions.keys().cloned().collect();
            (branch_entry.branch.clone(), hashes)
        };
        for hash in hashes {
            self.poll_subscription(repo, branch, hash, &branch_handle, env)
                .await;
        }
    }
}

/// Run a [`ConceptQuery`] against a branch and collect every
/// matching conclusion.
pub(crate) async fn run_query(
    branch: &Branch,
    query: ConceptQuery,
    env: &DefaultOperator,
) -> Result<Vec<ConceptConclusion>, ReactorError> {
    branch
        .query()
        .select(query)
        .perform(env)
        .try_vec()
        .await
        .map_err(|e| ReactorError::QueryFailed(format!("{e:?}")))
}
