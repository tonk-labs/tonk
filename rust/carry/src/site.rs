//! Site discovery and repository access for the `.carry/` per-project model.
//!
//! A **site** is a directory containing a `.carry/` subdirectory. Commands walk
//! up the filesystem tree from `$PWD` toward `$HOME` looking for the first
//! `.carry/` directory, unless `--repo <PATH>` is supplied.
//!
//! After discovery, `Site::resolve()` opens the identity (Profile + Operator)
//! and provides access to a `Branch` for queries and mutations.

use crate::identity_cmd::{self, ProfileLocation};
use anyhow::{Context, Result};
use dialog_capability::Capability;
use dialog_capability::storage::Location;
use dialog_credentials::credential::SignerCredential;
use dialog_repository::profile::Profile;
use dialog_repository::storage::Storage;
use dialog_repository::{Branch, Operator, Repository};
use dialog_storage::provider::Address;
use std::path::{Path, PathBuf};

/// A capability pointing to a repository's storage location.
pub type RepoLocation = Capability<Location<Address>>;

// ---------------------------------------------------------------------------
// Site -- the `.carry/` directory plus identity context
// ---------------------------------------------------------------------------

/// Handle to a discovered `.carry/` site directory with identity context.
pub struct Site {
    /// Absolute path to the `.carry/` directory itself.
    root: PathBuf,
    /// The user's profile identity.
    pub profile: Profile,
    /// The operator environment (derived from profile).
    pub operator: Operator,
    /// The backing storage.
    pub storage: Storage,
    /// The capability-based repository (owns the delegation chain).
    pub repo: Repository<SignerCredential>,
    /// The main branch for data operations.
    pub branch: Branch,
    /// Profile storage location (kept for re-opening with the same identity).
    profile_location: Option<ProfileLocation>,
}

impl Site {
    // -- Discovery -----------------------------------------------------------

    /// Discover a `.carry/` directory by walking up from `start` toward `$HOME`.
    fn discover_dir(start: &Path) -> Option<PathBuf> {
        let stop_at = dirs::home_dir();
        let mut current = start.to_path_buf();
        loop {
            let candidate = current.join(".carry");
            if candidate.is_dir() {
                return Some(candidate);
            }
            if let Some(ref home) = stop_at
                && current == *home
            {
                return None;
            }
            if !current.pop() {
                return None;
            }
        }
    }

    /// Locate the `.carry/` directory from an optional `--repo` flag,
    /// `CARRY_REPO` env var, or CWD discovery.
    fn locate(site_flag: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = site_flag {
            let carry_dir = if path.ends_with(".carry") {
                path.to_path_buf()
            } else {
                path.join(".carry")
            };
            if !carry_dir.is_dir() {
                anyhow::bail!("No .carry directory found at {}", carry_dir.display());
            }
            return Ok(carry_dir);
        }
        if let Ok(env_repo) = std::env::var("CARRY_REPO") {
            let p = Path::new(&env_repo);
            let carry_dir = if p.ends_with(".carry") {
                p.to_path_buf()
            } else {
                p.join(".carry")
            };
            if !carry_dir.is_dir() {
                anyhow::bail!("No .carry directory at CARRY_REPO={}", env_repo);
            }
            return Ok(carry_dir);
        }
        let cwd = std::env::current_dir().context("Failed to determine current directory")?;
        Self::discover_dir(&cwd).context("No .carry repo found (run `carry init` to create one)")
    }

    /// Default repo storage location (CWD-relative).
    fn default_repo_location() -> RepoLocation {
        Storage::current(".carry")
    }

    /// Open a repository and its main branch.
    async fn open_repo_and_branch(
        operator: &Operator,
        repo_location: Option<RepoLocation>,
    ) -> Result<(Repository<SignerCredential>, Branch)> {
        let location = repo_location.unwrap_or_else(Self::default_repo_location);
        let repo = Repository::open(location)
            .perform(operator)
            .await
            .context("Failed to open repository")?;
        let branch = repo
            .branch("main")
            .open()
            .perform(operator)
            .await
            .context("Failed to open main branch")?;
        Ok((repo, branch))
    }

    /// Resolve a site from an optional `--repo` flag. Opens identity + repo.
    ///
    /// `profile_location`: `None` for production (platform data dir),
    /// `Some(loc)` for test isolation.
    pub async fn resolve(
        site_flag: Option<&Path>,
        profile_location: Option<ProfileLocation>,
    ) -> Result<Self> {
        let root = Self::locate(site_flag)?;

        // Ensure CWD is the project directory (parent of .carry/) so that
        // Storage::current(".carry") resolves to the discovered repo,
        // not whatever directory the process happened to start in.
        let project_dir = root.parent().context(".carry/ has no parent directory")?;
        std::env::set_current_dir(project_dir)
            .with_context(|| format!("Failed to chdir to {}", project_dir.display()))?;

        let id = identity_cmd::ensure_identity(profile_location.clone()).await?;
        let (repo, branch) = Self::open_repo_and_branch(&id.operator, None).await?;
        Ok(Self {
            root,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
            branch,
            profile_location,
        })
    }

    /// Create a new `.carry/` directory at `parent` and open identity + repo.
    ///
    /// Creates the repository credential and delegates ownership to the
    /// profile so the operator can act on behalf of the repo.
    ///
    /// `profile_location`: `None` for production, `Some(loc)` for tests.
    /// `repo_location`: `None` for production (CWD-relative), `Some(loc)` for tests.
    pub async fn init(
        parent: &Path,
        profile_location: Option<ProfileLocation>,
        repo_location: Option<RepoLocation>,
    ) -> Result<Self> {
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)
            .with_context(|| format!("Failed to create {}", carry_dir.display()))?;

        // When no explicit repo_location is given, ensure CWD is the
        // project directory so Storage::current(".carry") resolves
        // to the newly created directory.
        if repo_location.is_none() {
            std::env::set_current_dir(parent)
                .with_context(|| format!("Failed to chdir to {}", parent.display()))?;
        }

        let id = identity_cmd::ensure_identity(profile_location.clone()).await?;

        let location = repo_location
            .clone()
            .unwrap_or_else(Self::default_repo_location);

        // Create the repository (generates a repo credential)
        let repo = Repository::open(location)
            .perform(&id.operator)
            .await
            .context("Failed to create repository")?;

        // Delegate repo ownership to the profile so the operator
        // (derived from the profile) can act on the repo's behalf. The
        // repo claims authority over itself and re-delegates to the
        // profile's DID; the profile then saves the resulting chain.
        let chain = repo
            .access()
            .claim(&repo)
            .delegate(id.profile.did())
            .perform(&id.operator)
            .await
            .context("Failed to delegate repo ownership to profile")?;
        id.profile
            .save(chain)
            .perform(&id.operator)
            .await
            .context("Failed to save ownership delegation")?;

        // Open the main branch
        let branch = repo
            .branch("main")
            .open()
            .perform(&id.operator)
            .await
            .context("Failed to open main branch")?;

        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
            branch,
            profile_location,
        })
    }

    /// Open a site at an explicit path (for use by init when .carry/ already exists).
    ///
    /// `profile_location`: `None` for production, `Some(loc)` for tests.
    /// `repo_location`: `None` for production, `Some(loc)` for tests.
    pub async fn open(
        path: &Path,
        profile_location: Option<ProfileLocation>,
        repo_location: Option<RepoLocation>,
    ) -> Result<Self> {
        let carry_dir = if path.ends_with(".carry") {
            path.to_path_buf()
        } else {
            path.join(".carry")
        };
        if !carry_dir.is_dir() {
            anyhow::bail!("No .carry directory found at {}", carry_dir.display());
        }
        let id = identity_cmd::ensure_identity(profile_location.clone()).await?;
        let (repo, branch) = Self::open_repo_and_branch(&id.operator, repo_location).await?;
        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
            branch,
            profile_location,
        })
    }

    // -- Accessors -----------------------------------------------------------

    /// Path to the `.carry/` directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the directory *containing* `.carry/`.
    pub fn parent(&self) -> &Path {
        self.root
            .parent()
            .expect(".carry/ always has a parent directory")
    }

    /// The profile DID.
    pub fn did(&self) -> String {
        self.profile.did().to_string()
    }

    /// The repository DID.
    pub fn repo_did(&self) -> String {
        self.repo.did().to_string()
    }

    /// The profile storage location (for passing to sub-sites, e.g. join).
    pub fn profile_location(&self) -> Option<ProfileLocation> {
        self.profile_location.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_discover_walks_up() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".carry")).unwrap();

        let nested = tmp.path().join("foo").join("bar").join("baz");
        std::fs::create_dir_all(&nested).unwrap();

        let found = Site::discover_dir(&nested).unwrap();
        assert_eq!(found, tmp.path().join(".carry"));
    }

    #[test]
    fn test_discover_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(Site::discover_dir(tmp.path()).is_none());
    }

    #[test]
    fn test_locate_explicit_path() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".carry")).unwrap();

        let located = Site::locate(Some(tmp.path())).unwrap();
        assert_eq!(located, tmp.path().join(".carry"));

        let located2 = Site::locate(Some(&tmp.path().join(".carry"))).unwrap();
        assert_eq!(located2, tmp.path().join(".carry"));
    }
}
