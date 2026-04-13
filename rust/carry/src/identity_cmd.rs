//! `carry identity` -- manage the local user identity.
//!
//! Identity is derived from a passkey via WebAuthn PRF. On first use, the
//! user authenticates with a passkey and the PRF output is used to
//! deterministically derive an account key and a profile key. The profile
//! credential is persisted so subsequent runs don't require the browser.
//!
//! The account key signs a UCAN delegation to the profile, enabling
//! cross-device recovery: the same passkey on a new device produces the
//! same account key, which can delegate to that device's profile.

use crate::passkey;
use anyhow::{Context, Result};
use dialog_capability::storage::Location;
use dialog_capability::{Capability, Policy, Subject};
use dialog_credentials::SignerCredential;
use dialog_credentials::credential::Credential;
use dialog_repository::profile::Profile;
use dialog_repository::storage::{LocationExt, Storage};
use dialog_repository::{Operator, Remote};
use dialog_storage::provider::Address;
use dialog_ucan::DelegationChain;
use dialog_ucan::delegation::builder::DelegationBuilder;
use dialog_ucan::subject::Subject as UcanSubject;
use dialog_varsig::{Did, Principal};

/// A capability pointing to a profile's storage location.
pub type ProfileLocation = Capability<Location<Address>>;

/// The trio that every command needs: profile identity, an operator
/// environment, and the backing storage.
pub struct Identity {
    pub profile: Profile,
    pub operator: Operator,
    pub storage: Storage,
    pub account_did: Option<Did>,
}

/// Ensure a local identity exists. On first use, runs the passkey WebAuthn
/// flow to derive keys. Subsequent calls load the persisted profile.
///
/// `location` controls where the profile is stored:
/// - `None` -> platform data directory, uses passkey-derived keys
/// - `Some(loc)` -> custom location (e.g. for tests), uses auto-generated keys
pub async fn ensure_identity(location: Option<ProfileLocation>) -> Result<Identity> {
    let storage = Storage::new();
    let use_passkey = location.is_none();
    let profile_location = location.unwrap_or_else(|| Storage::profile("carry"));

    let (profile, account_did) = if use_passkey {
        ensure_passkey_identity(&storage, &profile_location).await?
    } else {
        // Test/custom path: auto-generate a random profile (no passkey).
        let profile = Profile::open(profile_location.clone())
            .perform(&storage)
            .await
            .context("Failed to open carry profile")?;
        (profile, None)
    };

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
        account_did,
    })
}

/// Passkey-based identity: load existing or derive from passkey ceremony.
async fn ensure_passkey_identity(
    storage: &Storage,
    profile_location: &ProfileLocation,
) -> Result<(Profile, Option<Did>)> {
    // Try to load an existing profile.
    if let Ok(profile) = Profile::load(profile_location.clone())
        .perform(storage)
        .await
    {
        let account_did = profile_data_dir()
            .and_then(|dir| passkey::load_account_did(&dir))
            .and_then(|s| s.parse::<Did>().ok());
        return Ok((profile, account_did));
    }

    // No profile -- run passkey ceremony.
    let profile_dir = profile_data_dir().context("Cannot determine profile data directory")?;

    let credential_id = passkey::load_credential_id(&profile_dir);
    let result = passkey::authenticate(credential_id.as_deref()).await?;

    let derived = passkey::derive_identity(&result.prf_output).await?;
    let account_did = derived.account_signer.did();

    // Save the derived profile credential where Profile::load expects it.
    save_derived_profile(storage, profile_location, &derived.profile_signer).await?;
    let profile = Profile::load(profile_location.clone())
        .perform(storage)
        .await
        .context("Failed to load newly created profile")?;

    // Persist passkey metadata.
    passkey::save_credential_id(&profile_dir, &result.credential_id)?;
    passkey::save_account_did(&profile_dir, account_did.as_ref())?;

    // Build a temporary operator to store the account->profile delegation.
    let tmp_operator = profile
        .derive(b"carry-cli")
        .allow(Subject::any())
        .network(Remote)
        .build(storage.clone())
        .await
        .context("Failed to build operator for delegation setup")?;

    create_account_delegation(&derived.account_signer, &profile, &tmp_operator).await?;

    eprintln!("Identity created from passkey.");
    eprintln!("  account:  {}", account_did);
    eprintln!("  profile:  {}", profile.did());

    Ok((profile, Some(account_did)))
}

/// Save a derived profile credential to the location dialog-db's Profile expects.
async fn save_derived_profile(
    storage: &Storage,
    location: &ProfileLocation,
    credential: &SignerCredential,
) -> Result<()> {
    let cred_location = location
        .resolve("credential/profile")
        .context("Failed to resolve credential path")?;

    cred_location
        .save(Credential::Signer(credential.clone()))
        .perform(storage)
        .await
        .context("Failed to save derived profile credential")?;

    let address = Location::of(location).address().clone();
    dialog_capability::storage::Storage::mount(credential.did(), address)
        .perform(storage)
        .await
        .context("Failed to mount profile DID")?;

    Ok(())
}

/// Create a powerline UCAN delegation from account to profile.
async fn create_account_delegation(
    account_credential: &SignerCredential,
    profile: &Profile,
    operator: &Operator,
) -> Result<()> {
    let delegation = DelegationBuilder::new()
        .issuer(account_credential.clone())
        .audience(profile)
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .context("Failed to build account->profile delegation")?;

    let chain = DelegationChain::new(delegation);
    profile
        .save(chain)
        .perform(operator)
        .await
        .context("Failed to save account->profile delegation")?;

    Ok(())
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

    let id = ensure_identity(None).await?;

    if let Some(account_did) = &id.account_did {
        println!("account:  {}", account_did);
    }
    println!("profile:  {}", id.profile.did());

    Ok(())
}

/// Platform data directory for the carry profile.
fn profile_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("dialog").join("carry"))
}
