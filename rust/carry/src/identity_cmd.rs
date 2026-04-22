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
use dialog_capability::Subject;
use dialog_credentials::SignerCredential;
use dialog_credentials::credential::Credential;
use dialog_effects::storage::{Directory, LocationExt, Storage as StorageFx};
use dialog_operator::profile::Profile;
use dialog_repository::Operator;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::delegation::builder::DelegationBuilder;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::{Did, Principal};

/// Selector for where a profile lives on disk.
///
/// A `ProfileLocation` is a `(directory, name)` pair that tells the storage
/// layer where to load or create the profile's credential.
#[derive(Clone, Debug)]
pub struct ProfileLocation {
    /// The directory category (profile dir, temp dir, or explicit path).
    pub directory: Directory,
    /// The name within the directory.
    pub name: String,
}

impl ProfileLocation {
    /// Create a profile location.
    pub fn new(directory: Directory, name: impl Into<String>) -> Self {
        Self {
            directory,
            name: name.into(),
        }
    }

    /// Default platform profile location for carry.
    pub fn production() -> Self {
        Self::new(Directory::Profile, "carry")
    }
}

/// The pair that every command needs: profile identity and an operator
/// environment.
pub struct Identity {
    pub profile: Profile,
    pub operator: Operator<NativeSpace>,
    pub account_did: Option<Did>,
}

/// Ensure a local identity exists. On first use, runs the passkey WebAuthn
/// flow to derive keys. Subsequent calls load the persisted profile.
///
/// `location` controls where the profile is stored:
/// - `None` -> platform data directory, uses passkey-derived keys
/// - `Some(loc)` -> custom location (e.g. for tests), uses auto-generated keys
///
/// `repo_base` is the operator's base directory, which scopes the repository
/// storage. Defaults to `Directory::Current` when `None`.
pub async fn ensure_identity(
    location: Option<ProfileLocation>,
    repo_base: Option<Directory>,
) -> Result<Identity> {
    let storage = Storage::<NativeSpace>::default();
    let use_passkey = location.is_none();
    let profile_location = location.unwrap_or_else(ProfileLocation::production);

    // Load existing profile if present; otherwise either run the passkey
    // ceremony (production) or auto-generate (test). The passkey path also
    // returns the account signer so the caller can persist the
    // account->profile delegation once the operator is built.
    let (profile, account_did, pending_account_signer) = if use_passkey {
        load_or_derive_passkey_identity(&storage, &profile_location).await?
    } else {
        let profile = Profile::open(&profile_location.name)
            .at(profile_location.directory.clone())
            .perform(&storage)
            .await
            .context("Failed to open carry profile")?;
        (profile, None, None)
    };

    let operator = profile
        .derive(b"carry-cli")
        .base(repo_base.unwrap_or(Directory::Current))
        .allow(Subject::any())
        .build(storage)
        .await
        .context("Failed to build operator from profile")?;

    if let Some(account_signer) = pending_account_signer {
        create_account_delegation(&account_signer, &profile, &operator).await?;
        eprintln!("Identity created from passkey.");
        if let Some(ref did) = account_did {
            eprintln!("  account:  {}", did);
        }
        eprintln!("  profile:  {}", profile.did());
    }

    Ok(Identity {
        profile,
        operator,
        account_did,
    })
}

/// Load an existing passkey-backed profile, or run the passkey ceremony and
/// persist the derived credential. Returns the profile, the account DID (if
/// known), and — on first derivation — the account signer that still needs
/// to be used to mint the account->profile delegation.
async fn load_or_derive_passkey_identity(
    storage: &Storage<NativeSpace>,
    profile_location: &ProfileLocation,
) -> Result<(Profile, Option<Did>, Option<SignerCredential>)> {
    if let Ok(profile) = Profile::load(&profile_location.name)
        .at(profile_location.directory.clone())
        .perform(storage)
        .await
    {
        let account_did = profile_data_dir()
            .and_then(|dir| passkey::load_account_did(&dir))
            .and_then(|s| s.parse::<Did>().ok());
        return Ok((profile, account_did, None));
    }

    let profile_dir = profile_data_dir().context("Cannot determine profile data directory")?;
    let credential_id = passkey::load_credential_id(&profile_dir);
    let result = passkey::authenticate(credential_id.as_deref()).await?;

    let derived = passkey::derive_identity(&result.prf_output).await?;
    let account_did = derived.account_signer.did();

    save_derived_profile(storage, profile_location, &derived.profile_signer).await?;
    let profile = Profile::load(&profile_location.name)
        .at(profile_location.directory.clone())
        .perform(storage)
        .await
        .context("Failed to load newly created profile")?;

    passkey::save_credential_id(&profile_dir, &result.credential_id)?;
    passkey::save_account_did(&profile_dir, account_did.as_ref())?;

    Ok((profile, Some(account_did), Some(derived.account_signer)))
}

/// Save a derived profile credential to the location dialog-db's Profile expects.
async fn save_derived_profile(
    storage: &Storage<NativeSpace>,
    location: &ProfileLocation,
    credential: &SignerCredential,
) -> Result<()> {
    let storage_location = match &location.directory {
        Directory::Profile => StorageFx::profile(&location.name),
        Directory::Current => StorageFx::current(&location.name),
        Directory::Temp => StorageFx::temp(&location.name),
        Directory::At(path) => StorageFx::at(path),
    };

    storage_location
        .create(Credential::Signer(credential.clone()))
        .perform(storage)
        .await
        .context("Failed to save derived profile credential")?;

    Ok(())
}

/// Create a powerline UCAN delegation from account to profile.
async fn create_account_delegation(
    account_credential: &SignerCredential,
    profile: &Profile,
    operator: &Operator<NativeSpace>,
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
        .save(UcanDelegation::new(chain))
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

    let id = ensure_identity(None, None).await?;

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
