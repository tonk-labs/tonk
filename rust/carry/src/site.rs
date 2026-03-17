//! Site discovery and space access for the `.carry/` per-project model.
//!
//! A **site** is a directory containing a `.carry/` subdirectory. Commands walk
//! up the filesystem tree from `$PWD` toward `$HOME` looking for the first
//! `.carry/` directory, unless `--site <PATH>` is supplied.
//!
//! A **space** is a subdirectory of `.carry/` named by its `did:key:z...` DID.
//! Each space directory contains:
//!
//! - `credentials` — 32-byte Ed25519 secret key
//! - `facts/`      — dialog-db prolly tree storage
//!
//! The active space is tracked in `.carry/@active` (a plain-text file
//! containing the space DID).
//!
//! # TODO: Expose multi-space support in CLI
//!
//! The infrastructure here already supports multiple spaces per site:
//! - [`Site::list_spaces`] - enumerate all spaces
//! - [`Site::create_space`] - create additional spaces
//! - [`Site::set_active_space`] / [`Site::active_space_did`] - switch between spaces
//! - [`Site::space_by_did`] - lookup by DID
//!
//! What's needed:
//! 1. Add `carry space` subcommand in `bin/carry.rs` (see TODO there)
//! 2. Add label storage for spaces (currently only xyz.tonk.carry/label on init)
//! 3. Consider: space_by_label() lookup for `carry space switch my-label`
//! 4. Consider: delete_space() with safety checks (no active, confirm prompt)

use anyhow::{Context, Result};
use dialog_artifacts::repository::{BranchId, Credentials, Repository};
use dialog_query::Session;
use ed25519_dalek::SigningKey;
use rand_0_8::rngs::OsRng;
use std::path::{Path, PathBuf};
use tonk_space::{FsBackend, Operator};

/// Marker file for the active space within a `.carry/` directory.
const ACTIVE_MARKER: &str = "@active";

/// Filename for the 32-byte Ed25519 secret key inside a space directory.
const CREDENTIALS_FILE: &str = "credentials";

/// Subdirectory inside each space for dialog-db storage.
const FACTS_DIR: &str = "facts";

// ---------------------------------------------------------------------------
// Site — the `.carry/` directory and its contents
// ---------------------------------------------------------------------------

/// Handle to a discovered `.carry/` site directory.
#[derive(Debug, Clone)]
pub struct Site {
    /// Absolute path to the `.carry/` directory itself.
    root: PathBuf,
}

impl Site {
    // -- Discovery -----------------------------------------------------------

    /// Discover a site by walking up from `start` toward `$HOME`.
    ///
    /// Returns `None` if no `.carry/` directory is found before reaching
    /// `$HOME` (or the filesystem root if `$HOME` is not set).
    pub fn discover(start: &Path) -> Option<Self> {
        let stop_at = dirs::home_dir();
        let mut current = start.to_path_buf();
        loop {
            let candidate = current.join(".carry");
            if candidate.is_dir() {
                return Some(Self { root: candidate });
            }
            // Don't walk above $HOME
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

    /// Open a site at an explicit path (`--site <PATH>`).
    ///
    /// `path` should point to the directory *containing* `.carry/`, or to the
    /// `.carry/` directory itself. Returns an error if the directory does not
    /// exist.
    pub fn open(path: &Path) -> Result<Self> {
        let carry_dir = if path.ends_with(".carry") {
            path.to_path_buf()
        } else {
            path.join(".carry")
        };
        if !carry_dir.is_dir() {
            anyhow::bail!("No .carry directory found at {}", carry_dir.display());
        }
        Ok(Self { root: carry_dir })
    }

    /// Resolve a site from an optional `--site` flag, falling back to the
    /// `CARRY_SITE` env var, then CWD discovery.
    pub fn resolve(site_flag: Option<&Path>) -> Result<Self> {
        if let Some(path) = site_flag {
            return Self::open(path);
        }
        if let Ok(env_site) = std::env::var("CARRY_SITE") {
            return Self::open(Path::new(&env_site));
        }
        let cwd = std::env::current_dir().context("Failed to determine current directory")?;
        Self::discover(&cwd).context("No .carry site found (run `carry init` to create one)")
    }

    /// Create a new `.carry/` directory at `parent`.
    ///
    /// Returns a `Site` handle. Does **not** create any spaces — call
    /// [`Site::create_space`] after this.
    pub fn init(parent: &Path) -> Result<Self> {
        let carry_dir = parent.join(".carry");
        std::fs::create_dir_all(&carry_dir)
            .with_context(|| format!("Failed to create {}", carry_dir.display()))?;
        Ok(Self { root: carry_dir })
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

    // -- Space management ----------------------------------------------------

    /// List all spaces (directories whose name starts with `did:key:`).
    pub fn list_spaces(&self) -> Result<Vec<SpaceRef>> {
        let mut spaces = Vec::new();
        for entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("Failed to read {}", self.root.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_str().unwrap_or_default().to_string();
            if name.starts_with("did:key:") && entry.path().is_dir() {
                spaces.push(SpaceRef {
                    did: name,
                    dir: entry.path(),
                });
            }
        }
        spaces.sort_by(|a, b| a.did.cmp(&b.did));
        Ok(spaces)
    }

    /// Get the active space DID (from `.carry/@active`).
    pub fn active_space_did(&self) -> Result<Option<String>> {
        let active_file = self.root.join(ACTIVE_MARKER);
        if !active_file.exists() {
            return Ok(None);
        }
        let content =
            std::fs::read_to_string(&active_file).context("Failed to read @active marker")?;
        let did = content.trim().to_string();
        if did.is_empty() {
            Ok(None)
        } else {
            Ok(Some(did))
        }
    }

    /// Set the active space DID.
    pub fn set_active_space(&self, space_did: &str) -> Result<()> {
        let active_file = self.root.join(ACTIVE_MARKER);
        std::fs::write(&active_file, space_did).context("Failed to write @active marker")?;
        Ok(())
    }

    /// Clear the active space marker.
    pub fn clear_active_space(&self) -> Result<()> {
        let active_file = self.root.join(ACTIVE_MARKER);
        if active_file.exists() {
            std::fs::remove_file(&active_file).context("Failed to remove @active marker")?;
        }
        Ok(())
    }

    /// Get a `SpaceRef` for a space by DID. Returns `None` if the space
    /// directory does not exist.
    pub fn space_by_did(&self, did: &str) -> Option<SpaceRef> {
        let dir = self.root.join(did);
        if dir.is_dir() {
            Some(SpaceRef {
                did: did.to_string(),
                dir,
            })
        } else {
            None
        }
    }

    /// Get the currently active space, or error with a helpful message.
    pub fn active_space(&self) -> Result<SpaceRef> {
        let did = self
            .active_space_did()?
            .context("No active space. Run `carry init` to create one")?;
        self.space_by_did(&did)
            .with_context(|| format!("Active space {} not found on disk", did))
    }

    /// Create a new space: generate an Ed25519 keypair, create the directory
    /// structure, and write the credentials file.
    ///
    /// Returns a `SpaceRef` for the newly created space.
    pub fn create_space(&self) -> Result<SpaceRef> {
        let signing_key = SigningKey::generate(&mut OsRng);
        self.create_space_from_key(&signing_key)
    }

    /// Create a space from an existing signing key.
    pub fn create_space_from_key(&self, signing_key: &SigningKey) -> Result<SpaceRef> {
        let operator = Operator::from_secret(signing_key.to_bytes());
        let did = operator.did().to_string();
        let space_dir = self.root.join(&did);

        if space_dir.exists() {
            anyhow::bail!("Space {} already exists at {}", did, space_dir.display());
        }

        // Create directories
        let facts_dir = space_dir.join(FACTS_DIR);
        std::fs::create_dir_all(&facts_dir)
            .with_context(|| format!("Failed to create {}", facts_dir.display()))?;

        // Write credentials (raw 32-byte secret key)
        let creds_path = space_dir.join(CREDENTIALS_FILE);
        std::fs::write(&creds_path, signing_key.to_bytes())
            .with_context(|| format!("Failed to write {}", creds_path.display()))?;

        // Restrict permissions on credentials file (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&creds_path, perms)?;
        }

        Ok(SpaceRef {
            did,
            dir: space_dir,
        })
    }
}

// ---------------------------------------------------------------------------
// SpaceRef — a reference to a single space directory
// ---------------------------------------------------------------------------

/// Lightweight reference to a space directory under `.carry/`.
#[derive(Debug, Clone)]
pub struct SpaceRef {
    /// The `did:key:z...` identifier for this space.
    pub did: String,
    /// Absolute path to the space directory (`.carry/did:key:z.../`).
    dir: PathBuf,
}

impl SpaceRef {
    /// Path to the space directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path to the `facts/` storage directory.
    pub fn facts_dir(&self) -> PathBuf {
        self.dir.join(FACTS_DIR)
    }

    /// Path to the `credentials` file.
    fn credentials_path(&self) -> PathBuf {
        self.dir.join(CREDENTIALS_FILE)
    }

    /// Load the Ed25519 signing key from the `credentials` file.
    pub fn load_signing_key(&self) -> Result<SigningKey> {
        let path = self.credentials_path();
        let bytes = std::fs::read(&path)
            .with_context(|| format!("Failed to read credentials at {}", path.display()))?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "Invalid credentials file at {} (expected 32 bytes, got {})",
                path.display(),
                bytes.len()
            );
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        Ok(SigningKey::from_bytes(&key_bytes))
    }

    /// Load an `Operator` from the credentials file.
    pub fn load_operator(&self) -> Result<Operator> {
        let key = self.load_signing_key()?;
        Ok(Operator::from_secret(key.to_bytes()))
    }

    /// Open a dialog-db `Session` for this space.
    pub async fn open_session(
        &self,
    ) -> Result<Session<dialog_artifacts::repository::Branch<FsBackend>>> {
        let branch = self.open_branch().await?;
        Ok(Session::open(branch))
    }

    /// Open a raw dialog-db `Branch` for this space.
    ///
    /// Use [`open_session`](SpaceRef::open_session) for read paths and
    /// single-valued writes. Use `open_branch` when you need raw
    /// `Instruction`-level access (e.g. multi-valued attributes).
    pub async fn open_branch(&self) -> Result<dialog_artifacts::repository::Branch<FsBackend>> {
        let operator = self.load_operator()?;
        let credentials = Credentials::from(&operator);
        let space_did: dialog_varsig::Did = self
            .did
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse space DID '{}': {:?}", self.did, e))?;
        let backend = FsBackend::new(self.facts_dir()).await?;
        let replica = Repository::open(credentials, space_did, backend)?;
        let branch_id = BranchId::new("main".to_string());
        let branch = replica.branches.open(&branch_id).await?;
        Ok(branch)
    }
}

// ---------------------------------------------------------------------------
// Convenience: resolve site + active space in one call
// ---------------------------------------------------------------------------

/// Resolved context for CLI commands that operate on a space.
///
/// Replaces the old `schema::SpaceContext`.
pub struct SiteContext {
    pub site: Site,
    pub space: SpaceRef,
}

impl SiteContext {
    /// Resolve from an optional `--site` flag. Discovers the site, then
    /// loads the active space.
    pub fn resolve(site_flag: Option<&Path>) -> Result<Self> {
        let site = Site::resolve(site_flag)?;
        let space = site.active_space()?;
        Ok(Self { site, space })
    }

    /// Open a dialog-db `Session` for the active space.
    pub async fn open_session(
        &self,
    ) -> Result<Session<dialog_artifacts::repository::Branch<FsBackend>>> {
        self.space.open_session().await
    }

    /// Open a raw `Branch` for the active space.
    pub async fn open_branch(&self) -> Result<dialog_artifacts::repository::Branch<FsBackend>> {
        self.space.open_branch().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_creates_carry_dir() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();
        assert!(site.root().exists());
        assert!(site.root().is_dir());
        assert_eq!(site.root(), tmp.path().join(".carry"));
    }

    #[test]
    fn test_create_space_and_list() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();
        let space = site.create_space().unwrap();
        assert!(space.did.starts_with("did:key:"));
        assert!(space.dir().exists());
        assert!(space.facts_dir().exists());
        assert!(space.credentials_path().exists());

        let spaces = site.list_spaces().unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].did, space.did);
    }

    #[test]
    fn test_active_space_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();
        let space = site.create_space().unwrap();

        assert!(site.active_space_did().unwrap().is_none());

        site.set_active_space(&space.did).unwrap();
        assert_eq!(site.active_space_did().unwrap().unwrap(), space.did);

        let active = site.active_space().unwrap();
        assert_eq!(active.did, space.did);

        site.clear_active_space().unwrap();
        assert!(site.active_space_did().unwrap().is_none());
    }

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();
        let space = site.create_space().unwrap();

        let key = space.load_signing_key().unwrap();
        let operator = Operator::from_secret(key.to_bytes());
        assert_eq!(operator.did().to_string(), space.did);
    }

    #[test]
    fn test_discover_walks_up() {
        let tmp = TempDir::new().unwrap();
        let _site = Site::init(tmp.path()).unwrap();

        // Create a nested directory
        let nested = tmp.path().join("foo").join("bar").join("baz");
        std::fs::create_dir_all(&nested).unwrap();

        // Discovery from nested should find the .carry/ at root
        let found = Site::discover(&nested).unwrap();
        assert_eq!(found.root(), tmp.path().join(".carry"));
    }

    #[test]
    fn test_discover_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(Site::discover(tmp.path()).is_none());
    }

    #[test]
    fn test_open_explicit_site() {
        let tmp = TempDir::new().unwrap();
        let _site = Site::init(tmp.path()).unwrap();

        // Open via parent directory
        let opened = Site::open(tmp.path()).unwrap();
        assert_eq!(opened.root(), tmp.path().join(".carry"));

        // Open via .carry/ directly
        let opened2 = Site::open(&tmp.path().join(".carry")).unwrap();
        assert_eq!(opened2.root(), tmp.path().join(".carry"));
    }

    #[test]
    fn test_duplicate_space_fails() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();

        let key = SigningKey::generate(&mut OsRng);
        site.create_space_from_key(&key).unwrap();
        let result = site.create_space_from_key(&key);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_open_session() {
        let tmp = TempDir::new().unwrap();
        let site = Site::init(tmp.path()).unwrap();
        let space = site.create_space().unwrap();

        // Should successfully open a session (creates the prolly tree storage)
        let _session = space.open_session().await.unwrap();
    }
}
