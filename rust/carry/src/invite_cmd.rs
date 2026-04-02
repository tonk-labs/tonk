//! `carry invite` — create an invite token for a collaborator.
//!
//! **Stub**: This command will be rewritten in step 5 to use dialog-artifacts
//! credential-as-capability delegation. For now it prints an error.

use crate::site::Site;
use anyhow::Result;

/// Execute `carry invite <invited_did>`.
pub async fn execute(_site: &Site, _invited_did: &str) -> Result<()> {
    anyhow::bail!(
        "Invite is not yet implemented with the new identity system.\n\
         This will be available after migration step 5."
    )
}
