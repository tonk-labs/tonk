//! Test harness for carry CLI integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated `.carry/` site
//! in a temporary directory, with a bootstrapped store.
//!
//! All tests use unique temp locations for both profile and repo storage,
//! so tests can run in parallel without interference.

use anyhow::{Context, Result};
use carry::identity_cmd::ProfileLocation;
use carry::site::{RepoLocation, Site};
use dialog_effects::storage::Directory;
use dialog_repository::helpers::unique_name;
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
    pub profile_location: ProfileLocation,
    pub repo_location: RepoLocation,
    site: Site,
}

#[allow(dead_code)]
impl TestEnv {
    /// Create a new test environment with a bootstrapped site.
    pub async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp directory")?;
        let site_path = temp_dir.path().to_path_buf();
        let profile_location =
            ProfileLocation::new(Directory::Temp, unique_name("carry-test-profile"));
        let repo_location = RepoLocation::new(Directory::Temp, unique_name("carry-test-repo"));

        // Initialize a site (creates .carry/ and bootstraps identity)
        let site = Site::init(
            &site_path,
            Some(profile_location.clone()),
            Some(repo_location.clone()),
        )
        .await?;
        let profile_did = site.did();

        // Bootstrap pre-registered concepts
        carry::schema::bootstrap_builtins(&site.branch, &site.operator).await?;

        Ok(Self {
            _temp_dir: temp_dir,
            site_path,
            profile_did,
            profile_location,
            repo_location,
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
