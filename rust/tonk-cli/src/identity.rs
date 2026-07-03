//! Local profile management.
//!
//! The profile is the user's persistent identity. It lives in
//! the platform-specific data directory (`~/Library/Application
//! Support/dialog/` on macOS, `~/.local/share/dialog/` on
//! Linux), under the subdirectory named [`PROFILE_NAME`].

use std::path::PathBuf;

use anyhow::{Context, Result};
use dialog_operator::Profile;
use dialog_storage::provider::storage::{NativeSpace, Storage};

use crate::site::PROFILE_NAME;

/// Storage namespace dialog uses under the platform data dir.
/// Mirrors the constant in
/// `dialog-storage::storage::provider::fs::STORAGE_NAMESPACE`.
/// Vendored here because it isn't exposed publicly, and `--reset`
/// needs the on-disk path to wipe the profile directory.
const STORAGE_NAMESPACE: &str = "dialog";

/// Open the user's profile, creating it on first run.
pub async fn open() -> Result<Profile> {
    let storage = Storage::<NativeSpace>::default();
    Profile::open(PROFILE_NAME)
        .perform(&storage)
        .await
        .with_context(|| format!("failed to open profile '{PROFILE_NAME}'"))
}

/// Wipe the on-disk profile directory and create a fresh
/// profile. The new profile has a brand-new DID — every site
/// (`.tonk/`) the previous identity owned will be unreachable
/// without re-delegation.
pub async fn reset() -> Result<Profile> {
    let dir = profile_dir()?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove profile directory {}", dir.display()))?;
    }
    open().await
}

/// Whether a profile already exists on disk. Telemetry uses this to
/// avoid creating a profile as a side effect of computing a hashed
/// distinct id for a command that never touches the profile.
pub fn exists() -> bool {
    profile_dir().map(|dir| dir.is_dir()).unwrap_or(false)
}

/// Filesystem path to the profile directory. `tonk identity
/// --reset` calls `remove_dir_all` on this path; nothing else
/// inside the crate depends on the on-disk layout.
fn profile_dir() -> Result<PathBuf> {
    let data_dir = dirs::data_dir().context("could not determine platform data directory")?;
    Ok(data_dir.join(STORAGE_NAMESPACE).join(PROFILE_NAME))
}
