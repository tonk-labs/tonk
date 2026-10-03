//! [`RepositoryReference`] — chain handle for a repository.
//!
//! Pure description: names a repository by its key, the DID it is stored
//! under. The profile's own repository is one of them, named by the
//! profile's DID; it flows through the same `BranchReference` chain and
//! the same cache as every other, so handler bodies that take a branch
//! never care which one they are operating on.

use std::sync::Arc;

use dialog_capability::Principal as _;
use dialog_credentials::Credential;
use dialog_peer::SpaceHandle;
use dialog_repository::{Repository, RepositoryExt as _};

use crate::env::LoadProvider;
use crate::error::ReactorError;
use crate::{BranchReference, Reactor, RepositoryState};

/// Names a repository by its key. Acquire the underlying handle with
/// [`Self::acquire`] or chain to a branch with [`Self::branch`].
#[derive(Clone, Copy)]
pub struct RepositoryReference<'a> {
    /// Back-pointer to the reactor that owns the cache.
    pub(crate) reactor: &'a Reactor,
    /// The repository's key.
    pub(crate) name: &'a str,
}

impl<'a> RepositoryReference<'a> {
    /// The key this reference names its repository by.
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Whether this names the profile's own repository.
    pub fn is_profile(&self) -> bool {
        self.name == self.reactor.profile_key()
    }

    pub(crate) fn reactor(&self) -> &'a Reactor {
        self.reactor
    }

    /// Resolve and cache the underlying repository state.
    ///
    /// A cache hit returns the cached `Arc<RepositoryState>`. On a miss
    /// the profile's own repository is built over the profile's key, and
    /// any other is loaded through the profile; either is then cached
    /// under its key.
    pub async fn acquire<Env: LoadProvider>(
        &self,
        env: &Env,
    ) -> Result<Arc<RepositoryState>, ReactorError> {
        let reactor = self.reactor;
        // Fast path: cached.
        if let Some(entry) = reactor.repos().read().get(self.name) {
            return Ok(Arc::clone(entry));
        }

        // Slow path: build or load the repository outside the lock.
        let repository: Repository = if self.is_profile() {
            // The profile's repository is its own key's: wrapped as a
            // `Credential::Signer` so it is the `Repository<Credential>`
            // the cache stores, wherever the profile's space is kept.
            Repository::from(Credential::Signer(reactor.profile().clone()))
        } else {
            SpaceHandle {
                peer: reactor.profile().did(),
                name: self.name.to_string(),
            }
            .load()
            .perform(env)
            .await
            .map_err(|e| ReactorError::RepositoryNotFound {
                repo: self.name.to_string(),
                reason: e.to_string(),
            })?
        };

        // Insert under the lock — another caller may have raced; their
        // entry wins.
        let mut repos = reactor.repos().write();
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
