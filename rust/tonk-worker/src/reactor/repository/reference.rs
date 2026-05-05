//! [`RepositoryReference`] — chain handle for a repository.
//!
//! Pure description: holds the repo name by reference and a
//! back-pointer to the reactor. Nothing touches dialog handles
//! until either `.acquire(&env)` is called or a leaf effect on a
//! branch is `.perform(&env)`d.

use std::sync::Arc;

use dialog_repository::RepositoryExt as _;

use crate::reactor::env::LoadProvider;
use crate::reactor::error::ReactorError;
use crate::reactor::{BranchReference, RepositoryState, TonkReactor};

/// Names a repository by name. Acquire the underlying handle
/// with [`Self::acquire`] or chain to a branch with
/// [`Self::branch`].
#[derive(Clone, Copy)]
pub struct RepositoryReference<'a> {
    /// Back-pointer to the reactor that owns the cache.
    pub reactor: &'a TonkReactor,
    /// Repository name.
    pub name: &'a str,
}

impl<'a> RepositoryReference<'a> {
    /// Resolve and cache the underlying repository state. Cache
    /// hit returns the cached `Arc<RepositoryState>`; miss loads
    /// the repository via the profile and inserts.
    pub async fn acquire<Env: LoadProvider>(
        &self,
        env: &Env,
    ) -> Result<Arc<RepositoryState>, ReactorError> {
        // Fast path: cached.
        if let Some(entry) = self.reactor.repos().lock().get(self.name) {
            return Ok(Arc::clone(entry));
        }

        // Slow path: load the repository outside the lock.
        let repository = self
            .reactor
            .profile()
            .repository(self.name)
            .load()
            .perform(env)
            .await
            .map_err(|e| ReactorError::RepositoryNotFound {
                repo: self.name.to_owned(),
                reason: e.to_string(),
            })?;

        // Insert under the lock — another caller may have raced;
        // their entry wins.
        let mut repos = self.reactor.repos().lock();
        let entry = repos
            .entry(self.name.to_owned())
            .or_insert_with(|| Arc::new(RepositoryState::new(Arc::new(repository))));
        Ok(Arc::clone(entry))
    }

    /// Narrow to a specific branch.
    pub fn branch(self, name: &'a str) -> BranchReference<'a> {
        BranchReference {
            repository: self,
            name,
        }
    }
}
