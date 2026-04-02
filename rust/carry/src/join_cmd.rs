//! `carry join` — redeem an invite token to join a repository.
//!
//! **Stub**: This command will be rewritten in step 6 to use dialog-artifacts
//! credential import and delegation chains. For now it prints an error.

use anyhow::Result;
use std::path::Path;

/// Execute `carry join <token> [--repo <REPO>]`.
pub async fn execute(_token: &str, _site_flag: Option<&Path>) -> Result<()> {
    anyhow::bail!(
        "Join is not yet implemented with the new identity system.\n\
         This will be available after migration step 6."
    )
}
