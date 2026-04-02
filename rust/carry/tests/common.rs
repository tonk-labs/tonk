//! Test harness for carry CLI integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated `.carry/` site
//! in a temporary directory, with a bootstrapped store.

use anyhow::{Context, Result};
use carry::site::Site;
use std::path::PathBuf;
use tempfile::TempDir;

/// An isolated test environment backed by a temporary directory.
///
/// Each `TestEnv` contains a `.carry/` site with initialized data.
/// On drop the tempdir and its contents are deleted automatically.
#[allow(dead_code)]
pub struct TestEnv {
    _temp_dir: TempDir,
    pub site_path: PathBuf,
    pub profile_did: String,
    site: Site,
}

#[allow(dead_code)]
impl TestEnv {
    /// Create a new test environment with a bootstrapped site.
    pub async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp directory")?;
        let site_path = temp_dir.path().to_path_buf();

        // Initialize a site (creates .carry/ and bootstraps identity)
        let site = Site::init(&site_path).await?;
        let profile_did = site.did();

        // Open a session for bootstrapping
        let mut session = site.open_session().await?;
        carry::schema::bootstrap_builtins(&mut session).await?;

        Ok(Self {
            _temp_dir: temp_dir,
            site_path,
            profile_did,
            site,
        })
    }

    /// Get a reference to the Site.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Get a resolved Site (alias for `site()`, replaces old `ctx()`).
    pub async fn ctx(&self) -> &Site {
        &self.site
    }

    /// Get the `--site` argument value.
    pub fn site_arg(&self) -> String {
        self.site_path.to_string_lossy().into_owned()
    }

    /// Get path to a specific example YAML file.
    pub fn example_file(name: &str) -> String {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        format!("{}/tests/examples/{}", manifest_dir, name)
    }
}
