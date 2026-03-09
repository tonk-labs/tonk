//! Site discovery and repository access for the `.carry/` per-project model.
//!
//! A **site** is a directory containing a `.carry/` subdirectory. Commands walk
//! up the filesystem tree from `$PWD` toward `$HOME` looking for the first
//! `.carry/` directory, unless `--repo <PATH>` is supplied.
//!
//! The user identity (`Profile`) lives in dialog's platform data directory.
//! Repository data lives under the discovered `.carry/` directory: dialog's
//! `OperatorBuilder::base(Directory::At(carry_dir))` roots the operator's
//! spaces there.

use crate::identity_cmd::{self, ProfileLocation};
use anyhow::{Context, Result};
use dialog_credentials::Credential;
use dialog_effects::storage::Directory;
use dialog_operator::{Operator, Profile};
use dialog_repository::{Branch, Repository, RepositoryExt as _};
use dialog_storage::provider::storage::NativeSpace;
use std::path::{Path, PathBuf};

/// Repository name within the operator's base directory.
///
/// Carry stores its data in a single named repository per `.carry/`,
/// matching the original "one .carry/ per project" UX.
const REPO_NAME: &str = "main";

/// For tests that need to override where the repository data lives.
/// Production passes `None` and the data goes under the discovered `.carry/`.
pub type RepoLocation = Directory;

// ---------------------------------------------------------------------------
// Site -- the `.carry/` directory plus identity context
// ---------------------------------------------------------------------------

/// Handle to a discovered `.carry/` site directory with identity context.
pub struct Site {
    /// Absolute path to the `.carry/` directory itself.
    root: PathBuf,
    /// The user's profile identity.
    pub profile: Profile,
    /// The operator environment (derived from profile, scoped to `.carry/`).
    pub operator: Operator<NativeSpace>,
    /// The capability-based repository (owns the delegation chain).
    pub repo: Repository<Credential>,
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

    /// `Directory` for repo data: explicit override for tests, or the
    /// discovered `.carry/` directory for production.
    fn repo_directory(carry_dir: &Path, repo_location: Option<RepoLocation>) -> Directory {
        repo_location.unwrap_or_else(|| Directory::At(carry_dir.to_string_lossy().into_owned()))
    }

    /// Open or create the carry repository and its main branch.
    ///
    /// On first call, creates the repository and immediately delegates access
    /// to the profile -- without that delegation, follow-up calls like
    /// `profile.access().claim(&repo).delegate(...)` (used by `carry invite`)
    /// would fail with "no delegation chain found". On subsequent calls the
    /// repository is loaded as-is.
    async fn open_repo_and_branch(
        operator: &Operator<NativeSpace>,
        profile: &Profile,
    ) -> Result<(Repository<Credential>, Branch)> {
        let repo: Repository<Credential> =
            match profile.repository(REPO_NAME).load().perform(operator).await {
                Ok(repo) => repo,
                Err(_) => {
                    let signer_repo = profile
                        .repository(REPO_NAME)
                        .create()
                        .perform(operator)
                        .await
                        .context("Failed to create repository")?;

                    let delegation = signer_repo
                        .access()
                        .claim(&signer_repo)
                        .delegate(profile.did())
                        .perform(operator)
                        .await
                        .context("Failed to delegate repo access to profile")?;

                    profile
                        .access()
                        .save(delegation)
                        .perform(operator)
                        .await
                        .context("Failed to save repo access delegation")?;

                    Repository::from(Credential::Signer(signer_repo.credential().clone()))
                }
            };

        let branch = repo
            .branch("main")
            .open()
            .perform(operator)
            .await
            .context("Failed to open main branch")?;
        Ok((repo, branch))
    }

    /// Resolve a site from an optional `--repo` flag. Opens identity + repo.
    pub async fn resolve(
        site_flag: Option<&Path>,
        profile_location: Option<ProfileLocation>,
    ) -> Result<Self> {
        let root = Self::locate(site_flag)?;
        let repo_dir = Self::repo_directory(&root, None);
        let id = identity_cmd::ensure_identity(profile_location.clone(), Some(repo_dir)).await?;
        let (repo, branch) = Self::open_repo_and_branch(&id.operator, &id.profile).await?;
        Ok(Self {
            root,
            profile: id.profile,
            operator: id.operator,
            repo,
            branch,
            profile_location,
        })
    }

    /// Create a new `.carry/` directory at `parent` and open identity + repo.
    pub async fn init(
        parent: &Path,
        profile_location: Option<ProfileLocation>,
        repo_location: Option<RepoLocation>,
    ) -> Result<Self> {
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)
            .with_context(|| format!("Failed to create {}", carry_dir.display()))?;

        let repo_dir = Self::repo_directory(&carry_dir, repo_location);
        let id = identity_cmd::ensure_identity(profile_location.clone(), Some(repo_dir)).await?;
        let (repo, branch) = Self::open_repo_and_branch(&id.operator, &id.profile).await?;

        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
            repo,
            branch,
            profile_location,
        })
    }

    /// Open a site at an explicit path (for use by init when .carry/ already exists).
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
        let repo_dir = Self::repo_directory(&carry_dir, repo_location);
        let id = identity_cmd::ensure_identity(profile_location.clone(), Some(repo_dir)).await?;
        let (repo, branch) = Self::open_repo_and_branch(&id.operator, &id.profile).await?;
        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
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
