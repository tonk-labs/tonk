//! Test harness for carry CLI integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated `.carry/` site
//! in a temporary directory, with a bootstrapped space.

use anyhow::{Context, Result};
use carry::site::{Site, SpaceRef};
use std::path::PathBuf;
use tempfile::TempDir;
use tonk_space::Operator;

/// An isolated test environment backed by a temporary directory.
///
/// Each `TestEnv` contains a `.carry/` site with a single space.
/// On drop the tempdir and its contents are deleted automatically.
#[allow(dead_code)]
pub struct TestEnv {
    _temp_dir: TempDir,
    _identity_dir: TempDir,
    pub site_path: PathBuf,
    pub space_did: String,
    pub admin: Operator,
}

#[allow(dead_code)]
impl TestEnv {
    /// Create a new test environment with a bootstrapped space.
    ///
    /// Sets up a test identity via `CARRY_IDENTITY` env var pointing to a
    /// temp file, avoiding writes to the real `~/.carry/identity`.
    pub async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp directory")?;
        let site_path = temp_dir.path().to_path_buf();
        let site = Site::init(&site_path)?;

        // Create a test admin identity in an isolated temp directory
        let identity_dir = TempDir::new().context("Failed to create identity temp dir")?;
        let admin = Operator::generate();
        let identity_path = identity_dir.path().join("identity");
        std::fs::write(&identity_path, admin.to_secret())?;
        // Point CARRY_IDENTITY to the temp file so load_identity() finds it
        unsafe { std::env::set_var("CARRY_IDENTITY", &identity_path) };

        let (space, _proofs) = site.create_delegated_space(&[admin.did()]).await?;
        site.set_active_space(&space.did)?;

        // Bootstrap pre-registered concepts (attribute, concept, bookmark)
        let mut session = space.open_session().await?;
        carry::schema::bootstrap_builtins(&mut session).await?;

        let space_did = space.did.clone();

        Ok(Self {
            _temp_dir: temp_dir,
            _identity_dir: identity_dir,
            site_path,
            space_did,
            admin,
        })
    }

    /// Get a `Site` handle for this environment.
    pub fn site(&self) -> Site {
        Site::open(&self.site_path).unwrap()
    }

    /// Get the active `SpaceRef`.
    pub fn space(&self) -> SpaceRef {
        self.site().active_space().unwrap()
    }

    /// Get the `--site` argument value.
    pub fn site_arg(&self) -> String {
        self.site_path.to_string_lossy().into_owned()
    }

    /// Resolve a `SiteContext` for use in commands.
    pub async fn ctx(&self) -> carry::site::SiteContext {
        carry::site::SiteContext::resolve(Some(self.site_path.as_path()), None)
            .await
            .unwrap()
    }

    /// Set up a test identity via `CARRY_IDENTITY` env var.
    ///
    /// Returns the operator and a `TempDir` that must be kept alive for the
    /// identity file to persist.
    pub fn setup_test_identity(op: &Operator) -> TempDir {
        let dir = TempDir::new().expect("Failed to create identity temp dir");
        let path = dir.path().join("identity");
        std::fs::write(&path, op.to_secret()).unwrap();
        unsafe { std::env::set_var("CARRY_IDENTITY", &path) };
        dir
    }

    /// Get path to a specific example YAML file.
    pub fn example_file(name: &str) -> String {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        manifest_dir
            .join("examples")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
}
