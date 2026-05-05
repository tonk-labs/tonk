//! [`RepositoryState`] — cached repository handle plus its
//! branches. Held inside the reactor's cache as
//! `Arc<RepositoryState>`.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_repository::Repository;
use parking_lot::Mutex;

use crate::reactor::BranchState;

/// Cached repository handle plus its branches.
pub struct RepositoryState {
    /// `Arc` so the cache hands out a shared reference without
    /// cloning the underlying credential. Reused on cache hits
    /// to skip the repository load when opening another branch
    /// under the same repo.
    repository: Arc<Repository>,
    branches: Mutex<HashMap<String, Arc<BranchState>>>,
}

impl RepositoryState {
    /// Construct a fresh repository cache entry.
    pub fn new(repository: Arc<Repository>) -> Self {
        Self {
            repository,
            branches: Mutex::new(HashMap::new()),
        }
    }

    /// Cloned `Arc` to the open repository handle.
    pub fn repository(&self) -> Arc<Repository> {
        Arc::clone(&self.repository)
    }

    /// Borrow the branch cache so the chain's
    /// `BranchReference::acquire` can run lookup-and-open
    /// directly.
    pub fn branches(&self) -> &Mutex<HashMap<String, Arc<BranchState>>> {
        &self.branches
    }
}
