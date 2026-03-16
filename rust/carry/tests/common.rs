//! Test harness for carry CLI integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated `.carry/` site
//! in a temporary directory, with a bootstrapped space.

use anyhow::{Context, Result};
use carry::site::{Site, SpaceRef};
use std::path::PathBuf;
use tempfile::TempDir;

/// An isolated test environment backed by a temporary directory.
///
/// Each `TestEnv` contains a `.carry/` site with a single space.
/// On drop the tempdir and its contents are deleted automatically.
#[allow(dead_code)]
pub struct TestEnv {
    _temp_dir: TempDir,
    pub site_path: PathBuf,
    pub space_did: String,
}

#[allow(dead_code)]
impl TestEnv {
    /// Create a new test environment with a bootstrapped space.
    ///
    /// The space is initialized with pre-registered concepts (attribute,
    /// concept, bookmark) so that meta-schema operations work immediately.
    pub async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp directory")?;
        let site_path = temp_dir.path().to_path_buf();
        let site = Site::init(&site_path)?;
        let space = site.create_space()?;
        site.set_active_space(&space.did)?;

        // Bootstrap pre-registered concepts (attribute, concept, bookmark)
        let mut session = space.open_session().await?;
        carry::schema::bootstrap_builtins(&mut session).await?;

        let space_did = space.did.clone();

        Ok(Self {
            _temp_dir: temp_dir,
            site_path,
            space_did,
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
