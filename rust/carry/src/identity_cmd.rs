//! `carry identity` -- manage the local user identity.
//!
//! Identity is a `Profile` opened by name from dialog's storage. By default
//! the profile lives in the platform data directory (`Directory::Profile`);
//! tests pass an explicit `Directory` for isolation.
//!
//! On first use in production, the profile is bootstrapped from a passkey
//! WebAuthn ceremony (see [`crate::passkey`]). The PRF output deterministically
//! derives the profile key, so the same passkey on a new device produces the
//! same profile DID. The derived account DID is recorded next to the profile
//! for future cross-device recovery (account → profile delegation).

use crate::passkey;
use anyhow::{Context, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_varsig::{Did, Principal};

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
/// First-run in production triggers the passkey browser ceremony and records
/// the account DID alongside the profile. Tests pass an explicit
/// `profile_location` and skip the passkey path entirely.
pub async fn ensure_identity(
    profile_location: Option<ProfileLocation>,
    repo_base: Option<Directory>,
) -> Result<Identity> {
    let storage = Storage::<NativeSpace>::default();
    let use_passkey = profile_location.is_none();
    let directory = profile_location.unwrap_or(Directory::Profile);

    let profile = Profile::open(PROFILE_NAME)
        .at(directory.clone())
        .perform(&storage)
        .await
        .context("Failed to open carry profile")?;

    let account_did = if use_passkey {
        load_or_provision_account_did().await?
    } else {
        None
    };

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
        account_did,
    })
}

/// Run the passkey ceremony if we don't have an account DID on disk yet.
///
/// The PRF output drives [`passkey::derive_identity`] to produce account +
/// profile signers. The account DID is what's used for cross-device
/// recovery via account → profile delegation; that delegation flow is a
/// follow-up — for now we just persist the account DID alongside the
/// passkey credential id.
async fn load_or_provision_account_did() -> Result<Option<Did>> {
    let Some(profile_dir) = profile_data_dir() else {
        return Ok(None);
    };

    if let Some(s) = passkey::load_account_did(&profile_dir)
        && let Ok(did) = s.parse::<Did>()
    {
        return Ok(Some(did));
    }

    let credential_id = passkey::load_credential_id(&profile_dir);
    let result = passkey::authenticate(credential_id.as_deref()).await?;
    let derived = passkey::derive_identity(&result.prf_output).await?;

    std::fs::create_dir_all(&profile_dir)
        .with_context(|| format!("Failed to create {}", profile_dir.display()))?;
    passkey::save_credential_id(&profile_dir, &result.credential_id)?;
    let account_did = derived.account_signer.did();
    passkey::save_account_did(&profile_dir, account_did.as_ref())?;

    eprintln!("Identity created from passkey.");
    eprintln!("  account: {}", account_did);

    Ok(Some(account_did))
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
        println!("account: {}", account_did);
    }
    println!("profile: {}", id.profile.did());

    Ok(())
}

/// Platform data directory for the carry profile.
fn profile_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("carry"))
}
