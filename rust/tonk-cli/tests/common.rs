//! Test harness for tonk-cli integration tests.
//!
//! Provides a `TestEnv` struct that creates an isolated filesystem environment
//! for each test, with a bootstrapped operator, session (authority), and
//! optionally a space — bypassing the browser-based login flow entirely.

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::Delegation as UcanDelegation;
use dialog_ucan::subject::Subject;
use dialog_varsig::Did;
use dialog_varsig::eddsa::Ed25519Signature;
use std::path::PathBuf;
use tempfile::TempDir;
use tonk_cli::crypto::Operator;
use tonk_cli::delegation::Delegation;

/// An isolated test environment backed by a temporary directory.
///
/// Sets `TONK_HOME` and `TONK_OPERATOR_KEY` env vars so that all tonk-cli
/// functions use a fresh tempdir as the tonk data directory (equivalent to
/// `~/.tonk/` in normal operation).
///
/// On drop the tempdir and its contents are deleted automatically.
#[allow(dead_code)]
pub struct TestEnv {
    _temp_dir: TempDir,
    pub home_path: PathBuf,
    pub operator: Operator,
    pub operator_did: String,
    pub authority_operator: Operator,
    pub authority_did: String,
}

impl TestEnv {
    /// Create a new test environment with a bootstrapped session.
    ///
    /// This simulates the result of `tonk login` by:
    /// 1. Generating a deterministic operator keypair
    /// 2. Generating an "authority" keypair (simulating the auth service)
    /// 3. Creating a powerline UCAN delegation: authority → operator
    /// 4. Saving the delegation to the correct filesystem path
    /// 5. Setting the active session
    pub async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temp directory")?;
        let home_path = temp_dir.path().to_path_buf();

        // Generate operator keypair
        let operator = Operator::generate();
        let operator_did = operator.did().to_string();

        // Encode operator secret as base58btc for TONK_OPERATOR_KEY
        let operator_secret = operator.to_secret();
        let operator_key_b58 = bs58::encode(&operator_secret).into_string();

        // Set environment variables for isolation.
        // SAFETY: Tests run serially (via #[serial]) so no concurrent env var access.
        unsafe {
            std::env::set_var("TONK_HOME", &home_path);
            std::env::set_var("TONK_OPERATOR_KEY", &operator_key_b58);
        }

        // Generate authority keypair (simulates the auth service identity)
        let authority_operator = Operator::generate();
        let authority_did = authority_operator.did().to_string();

        // Create powerline delegation: authority → operator (subject = *)
        let authority_signer = Ed25519Signer::from(&authority_operator);
        let operator_did_parsed: Did = operator_did
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse operator DID: {:?}", e))?;

        let ucan_delegation: UcanDelegation<Ed25519Signature> = UcanDelegation::builder()
            .issuer(authority_signer)
            .audience(&operator_did_parsed)
            .subject(Subject::Any) // Powerline = "*"
            .command(vec![]) // Empty = root access "/"
            .try_build()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {}", e))?;

        let delegation = Delegation::from_ucan(ucan_delegation);

        // Save delegation to filesystem (this uses TONK_HOME via util::tonk_dir())
        delegation.save()?;

        // Set active session
        tonk_cli::state::set_active_session(&authority_did)?;

        // Save session metadata
        let session_meta = tonk_cli::metadata::SessionMetadata::new(
            "test-session".to_string(),
            "test://local".to_string(),
        );
        session_meta.save(&authority_did)?;

        Ok(Self {
            _temp_dir: temp_dir,
            home_path,
            operator,
            operator_did,
            authority_operator,
            authority_did,
        })
    }

    /// Create a named space in this test environment.
    ///
    /// Calls `tonk_cli::space::create()` which creates the space, saves
    /// delegations, metadata, initializes the dialog-db, and sets the
    /// space as active.
    pub async fn create_space(&self, name: &str) -> Result<String> {
        // Use JSON mode to capture the space DID from output
        // But space::create prints to stdout, so we call it and then
        // read the active space from state.
        tonk_cli::space::create(
            name.to_string(),
            Some(vec![]), // No additional owners (authority is always included)
            None,         // No description
            true,         // JSON mode (suppresses interactive prompts)
        )
        .await?;

        // Read back the active space DID
        let space_did = tonk_cli::state::get_active_space(&self.authority_did)?
            .context("Space was not set as active after creation")?;

        Ok(space_did)
    }

    /// Get the path to the examples directory.
    ///
    /// Uses the runtime `CARGO_MANIFEST_DIR` env var, which nextest
    /// automatically sets to the remapped path when running from an
    /// archive with `--workspace-remap`. Falls back to the compile-time
    /// value for normal `cargo test` runs.
    pub fn examples_dir() -> PathBuf {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        manifest_dir.join("examples")
    }

    /// Get path to a specific example YAML file.
    pub fn example_file(name: &str) -> String {
        Self::examples_dir()
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // Clean up env vars (best-effort, tests run serially anyway).
        // SAFETY: Tests run serially (via #[serial]) so no concurrent env var access.
        unsafe {
            std::env::remove_var("TONK_HOME");
            std::env::remove_var("TONK_OPERATOR_KEY");
        }
    }
}
