//! [`RepositoryReference`] — chain handle for a repository.
//!
//! Pure description: holds either a named repository (loaded as
//! a child of the profile) or *the profile-as-repository*. Both
//! shapes flow through the same `BranchReference` chain so the
//! handler bodies that take a branch don't need to care which
//! one they're operating on.

use std::sync::Arc;

use dialog_credentials::Credential;
use dialog_repository::{Repository, RepositoryExt as _};

use crate::reactor::env::LoadProvider;
use crate::reactor::error::ReactorError;
use crate::reactor::{BranchReference, RepositoryState, TonkReactor};

/// Reserved name used in error messages for the
/// profile-as-repository. Not reachable through the named
/// `repository(name)` path — the profile lives outside the
/// child-repo namespace.
const PROFILE_LABEL: &str = "<profile>";

/// Names a repository, either by lookup name (a child of the
/// profile) or as the profile itself. Acquire the underlying
/// handle with [`Self::acquire`] or chain to a branch with
/// [`Self::branch`].
#[derive(Clone, Copy)]
pub enum RepositoryReference<'a> {
    /// A named repository — opened as a child of the profile.
    Named {
        /// Back-pointer to the reactor that owns the cache.
        reactor: &'a TonkReactor,
        /// Repository name.
        name: &'a str,
    },
    /// The profile-as-repository. Doesn't share the named-repo
    /// namespace — `Repository::from(&profile)` opens it
    /// directly from the profile's signer credential.
    Profile {
        /// Back-pointer to the reactor that owns the profile
        /// state.
        reactor: &'a TonkReactor,
    },
}

impl<'a> RepositoryReference<'a> {
    /// Display label for this reference. The named variant
    /// returns its lookup name; the profile variant returns the
    /// reserved `<profile>` placeholder used in error messages
    /// when there's no other identifier to report.
    pub fn name(&self) -> &str {
        match self {
            Self::Named { name, .. } => name,
            Self::Profile { .. } => PROFILE_LABEL,
        }
    }

    fn reactor(&self) -> &'a TonkReactor {
        match self {
            Self::Named { reactor, .. } | Self::Profile { reactor } => reactor,
        }
    }

    /// Resolve and cache the underlying repository state.
    ///
    /// `Named`: cache hit returns the cached `Arc<RepositoryState>`;
    /// miss loads the repository via the profile and inserts.
    ///
    /// `Profile`: returns the reactor's profile-as-repository
    /// state, lazily constructed on first call.
    pub async fn acquire<Env: LoadProvider>(
        &self,
        env: &Env,
    ) -> Result<Arc<RepositoryState>, ReactorError> {
        let reactor = self.reactor();
        match self {
            Self::Named { name, .. } => {
                // Fast path: cached.
                if let Some(entry) = reactor.repos().lock().get(*name) {
                    return Ok(Arc::clone(entry));
                }

                // Slow path: load the repository outside the lock.
                let repository = reactor
                    .profile()
                    .repository(*name)
                    .load()
                    .perform(env)
                    .await
                    .map_err(|e| ReactorError::RepositoryNotFound {
                        repo: (*name).to_string(),
                        reason: e.to_string(),
                    })?;

                // Insert under the lock — another caller may have
                // raced; their entry wins.
                let mut repos = reactor.repos().lock();
                let entry = repos
                    .entry((*name).to_owned())
                    .or_insert_with(|| Arc::new(RepositoryState::new(Arc::new(repository))));
                Ok(Arc::clone(entry))
            }
            Self::Profile { .. } => {
                // Fast path: already constructed.
                if let Some(entry) = reactor.profile_repo_state().clone() {
                    return Ok(entry);
                }

                // First touch — wrap the profile's signer credential
                // as a `Credential::Signer` and feed it through
                // `Repository::from(Credential)` (which yields
                // `Repository<Credential>`, the default the cache
                // stores). The direct `From<&Profile>` impl returns
                // `Repository<SignerCredential>` which doesn't fit.
                let credential = Credential::Signer(reactor.profile().signer().clone());
                let repository: Repository = Repository::from(credential);
                let state = Arc::new(RepositoryState::new(Arc::new(repository)));

                Ok(reactor.set_profile_repo_state(state))
            }
        }
    }

    /// Narrow to a specific branch.
    pub fn branch(self, name: &'a str) -> BranchReference<'a> {
        BranchReference {
            repository: self,
            name,
        }
    }
}
