//! Test harness for carry CLI integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated `.carry/` site
//! in a temporary directory. Profile and repo storage use unique
//! `Directory::Temp` namespaces so tests can run in parallel without
//! stepping on each other.

use anyhow::{Context, Result};
use carry::identity_cmd::ProfileLocation;
use carry::site::{RepoLocation, Site};
use dialog_effects::storage::Directory;
use dialog_repository::helpers::unique_name;
use std::path::PathBuf;
use tempfile::TempDir;

/// Build a unique `Directory::At(...)` rooted under the platform temp dir.
///
/// Tests use this for both profile and repo locations so that parallel
/// test runs don't share storage. Each call produces a fresh path.
#[allow(dead_code)]
pub fn unique_dir(label: &str) -> Directory {
    let path = std::env::temp_dir().join(unique_name(label));
    Directory::At(path.to_string_lossy().into_owned())
}

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
        // Each test gets its own temp namespaces so profile/repo storage
        // don't collide across parallel runs.
        let profile_location = unique_dir("carry-test-profile");
        let repo_location = Directory::At(site_path.to_string_lossy().into_owned());

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
