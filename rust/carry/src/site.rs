//! Site discovery and repository access for the `.carry/` per-project model.
//!
//! A **site** is a directory containing a `.carry/` subdirectory. Commands walk
//! up the filesystem tree from `$PWD` toward `$HOME` looking for the first
//! `.carry/` directory, unless `--repo <PATH>` is supplied.
//!
//! After discovery, `Site::resolve()` opens the identity (Profile + Operator)
//! and provides access to the data store for queries and mutations.

use crate::identity_cmd;
use anyhow::{Context, Result};
use dialog_artifacts::profile::Profile;
use dialog_artifacts::storage::Storage;
use dialog_artifacts::{Artifacts, Operator, Repository};
use dialog_query::Session;
use dialog_storage::FileSystemStorageBackend;
use std::path::{Path, PathBuf};

/// Type alias for the filesystem-backed storage backend.
/// Key is Blake3Hash ([u8; 32]) as required by Artifacts.
pub type FsBackend = FileSystemStorageBackend<[u8; 32], Vec<u8>>;

/// Type alias for the filesystem-backed artifacts store.
pub type FsArtifacts = Artifacts<FsBackend>;

/// Subdirectory inside `.carry/` for artifact storage.
const CLAIMS_DIR: &str = "claims";

/// Identifier for the artifacts store within `.carry/claims/`.
const STORE_ID: &str = "main";

// ---------------------------------------------------------------------------
// Site — the `.carry/` directory plus identity context
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
    pub repo: Repository,
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

    /// Resolve a site from an optional `--repo` flag. Opens identity + repo.
    pub async fn resolve(site_flag: Option<&Path>) -> Result<Self> {
        let root = Self::locate(site_flag)?;
        let id = identity_cmd::ensure_identity().await?;
        let repo = Repository::open(Storage::current(".carry"))
            .perform(&id.operator)
            .await
            .context("Failed to open repository")?;
        Ok(Self {
            root,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
        })
    }

    /// Create a new `.carry/` directory at `parent` and open identity + repo.
    ///
    /// Creates the repository credential and delegates ownership to the
    /// profile so the operator can act on behalf of the repo.
    pub async fn init(parent: &Path) -> Result<Self> {
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)
            .with_context(|| format!("Failed to create {}", carry_dir.display()))?;

        // Ensure claims subdirectory exists
        let claims_dir = carry_dir.join(CLAIMS_DIR);
        std::fs::create_dir_all(&claims_dir)
            .with_context(|| format!("Failed to create {}", claims_dir.display()))?;

        let id = identity_cmd::ensure_identity().await?;

        // Create the repository (generates a repo credential)
        let repo = Repository::open(Storage::current(".carry"))
            .perform(&id.operator)
            .await
            .context("Failed to create repository")?;

        // Delegate repo ownership to the profile so the operator
        // (derived from the profile) can act on the repo's behalf.
        let chain = repo
            .ownership()
            .delegate(&id.profile)
            .perform(&id.operator)
            .await
            .context("Failed to delegate repo ownership to profile")?;
        id.profile
            .save(chain)
            .perform(&id.operator)
            .await
            .context("Failed to save ownership delegation")?;

        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
        })
    }

    /// Open a site at an explicit path (for use by init when .carry/ already exists).
    pub async fn open(path: &Path) -> Result<Self> {
        let carry_dir = if path.ends_with(".carry") {
            path.to_path_buf()
        } else {
            path.join(".carry")
        };
        if !carry_dir.is_dir() {
            anyhow::bail!("No .carry directory found at {}", carry_dir.display());
        }
        let id = identity_cmd::ensure_identity().await?;
        let repo = Repository::open(Storage::current(".carry"))
            .perform(&id.operator)
            .await
            .context("Failed to open repository")?;
        Ok(Self {
            root: carry_dir,
            profile: id.profile,
            operator: id.operator,
            storage: id.storage,
            repo,
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

    /// Path to the `claims/` storage directory.
    pub fn claims_dir(&self) -> PathBuf {
        self.root.join(CLAIMS_DIR)
    }

    /// The profile DID.
    pub fn did(&self) -> String {
        self.profile.did().to_string()
    }

    /// The repository DID.
    pub fn repo_did(&self) -> String {
        self.repo.did().to_string()
    }

    // -- Data access ---------------------------------------------------------

    /// Open a filesystem-backed `Artifacts` store for this site.
    async fn open_artifacts(&self) -> Result<Artifacts<FsBackend>> {
        let claims_dir = self.claims_dir();
        if !claims_dir.exists() {
            std::fs::create_dir_all(&claims_dir)
                .with_context(|| format!("Failed to create {}", claims_dir.display()))?;
        }
        let backend = FsBackend::new(claims_dir).await?;
        let artifacts = Artifacts::open(STORE_ID.to_string(), backend)
            .await
            .context("Failed to open artifacts store")?;
        Ok(artifacts)
    }

    /// Open a `Session` for reading and writing data.
    pub async fn open_session(&self) -> Result<Session<Artifacts<FsBackend>>> {
        let artifacts = self.open_artifacts().await?;
        Ok(Session::open(artifacts))
    }

    /// Open a raw `Artifacts` store (for low-level Instruction commits).
    pub async fn open_branch(&self) -> Result<Artifacts<FsBackend>> {
        self.open_artifacts().await
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
