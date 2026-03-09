//! `carry identity` -- manage the local user identity.
//!
//! Identity is a `Profile` opened by name from dialog's storage. By default
//! the profile lives in the platform data directory (`Directory::Profile`);
//! tests pass an explicit `Directory` for isolation.
//!
//! Passkey-derived identity (cross-device account recovery) layers on top
//! of this and lives in `passkey.rs`.

use anyhow::{Context, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_varsig::Did;

/// Profile name used for `carry`'s identity within dialog storage.
const PROFILE_NAME: &str = "carry";

/// Where the profile lives on disk. `Directory::Profile` (the default for
/// production) resolves to the platform data directory under dialog's
/// storage namespace. Tests pass `Directory::Temp` or `Directory::At(...)`.
pub type ProfileLocation = Directory;

/// The trio that every command needs: profile identity and an operator
/// environment scoped to a `.carry/` directory. The backing `Storage` is
/// owned by the operator after `build`; commands access it through the
/// operator's effect dispatch.
pub struct Identity {
    pub profile: Profile,
    pub operator: Operator<NativeSpace>,
    pub account_did: Option<Did>,
}

/// Ensure a local identity exists. Opens (or creates) the carry profile in
/// dialog storage and derives an operator scoped to a `.carry/` directory.
///
/// - `profile_location`: where the profile lives. `None` -> `Directory::Profile`.
/// - `repo_base`: base directory for repository data. `None` -> `Directory::Current`.
pub async fn ensure_identity(
    profile_location: Option<ProfileLocation>,
    repo_base: Option<Directory>,
) -> Result<Identity> {
    let storage = Storage::<NativeSpace>::default();
    let directory = profile_location.unwrap_or(Directory::Profile);

    let profile = Profile::open(PROFILE_NAME)
        .at(directory)
        .perform(&storage)
        .await
        .context("Failed to open carry profile")?;

    let operator = profile
        .derive(b"carry-cli")
        .allow(Subject::any())
        .base(repo_base.unwrap_or(Directory::Current))
        .build(storage)
        .await
        .context("Failed to build operator from profile")?;

    Ok(Identity {
        profile,
        operator,
        account_did: None,
    })
}

/// Execute `carry identity [--reset]`.
pub async fn execute(reset: bool) -> Result<()> {
    if reset && let Some(profile_dir) = profile_data_dir() {
        match std::fs::remove_dir_all(&profile_dir) {
            Ok(()) => eprintln!("Profile reset."),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to remove profile data"),
        }
    }

    let id = ensure_identity(None, None).await?;

    if let Some(account_did) = &id.account_did {
        println!("account:  {}", account_did);
    }
    println!("profile:  {}", id.profile.did());

    Ok(())
}

/// Platform data directory for the carry profile.
fn profile_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("carry"))
}
