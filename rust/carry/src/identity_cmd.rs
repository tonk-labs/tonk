//! `carry identity` — manage the local user identity.
//!
//! Identity is backed by a dialog-artifacts `Profile` which auto-generates
//! and persists an Ed25519 keypair. The profile lives under the platform
//! data directory via `Storage::profile("carry")`.

use anyhow::{Context, Result};
use dialog_artifacts::profile::Profile;
use dialog_artifacts::storage::Storage;
use dialog_artifacts::{Operator, Remote};
use dialog_capability::Subject;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// The trio that every command needs: profile identity, an operator
/// environment, and the backing storage.
pub struct Identity {
    pub profile: Profile,
    pub operator: Operator,
    pub storage: Storage,
}

/// Ensure a local identity exists (creates on first use).
/// Returns the Profile, Operator, and Storage.
pub async fn ensure_identity() -> Result<Identity> {
    let storage = Storage::new();
    let profile = Profile::open(Storage::profile("carry"))
        .perform(&storage)
        .await
        .context("Failed to open carry profile")?;
    let operator = profile
        .derive(b"carry-cli")
        .allow(Subject::any())
        .network(Remote)
        .build(storage.clone())
        .await
        .context("Failed to build operator from profile")?;
    Ok(Identity {
        profile,
        operator,
        storage,
    })
}

// ---------------------------------------------------------------------------
// CLI execute
// ---------------------------------------------------------------------------

/// Execute `carry identity [--reset]`.
///
/// If `reset` is false and a profile exists, just print the DID.
/// If `reset` is true, delete the stored profile and create a new one.
pub async fn execute(reset: bool) -> Result<()> {
    if reset {
        // Delete the profile storage directory
        let storage = Storage::new();
        let profile_result = Profile::load(Storage::profile("carry"))
            .perform(&storage)
            .await;
        if profile_result.is_ok() {
            // Remove the profile data directory
            if let Some(profile_dir) = profile_data_dir() {
                match std::fs::remove_dir_all(&profile_dir) {
                    Ok(()) => eprintln!("Profile reset."),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e).context("Failed to remove profile data"),
                }
            }
        }
    }

    let id = ensure_identity().await?;
    println!("{}", id.profile.did());

    Ok(())
}

/// Platform data directory for the carry profile.
fn profile_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("carry"))
}
