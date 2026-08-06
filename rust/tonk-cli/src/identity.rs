//! Local profile management.
//!
//! The profile is the user's persistent identity. It lives in
//! the platform-specific data directory (`~/Library/Application
//! Support/dialog/` on macOS, `~/.local/share/dialog/` on
//! Linux), under the subdirectory named [`PROFILE_NAME`].

use std::path::PathBuf;

use anyhow::{Context, Result};
use dialog_effects::credential::CredentialError;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use serde::{Deserialize, Serialize};

use crate::site::PROFILE_NAME;

/// Storage namespace dialog uses under the platform data dir.
/// Mirrors the constant in
/// `dialog-storage::storage::provider::fs::STORAGE_NAMESPACE`.
/// Vendored here because it isn't exposed publicly, and `--reset`
/// needs the on-disk path to wipe the profile directory.
const STORAGE_NAMESPACE: &str = "dialog";

/// Provider-neutral credential-store key for the durable local root.
pub const LOCAL_ROOT_SITE: &str = "tonk-local-root-v1";

/// Stable local-root record shared in shape with the browser worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRoot {
    /// Opaque passkey credential identifier.
    pub credential_id: String,
    /// Root DID derived by the passkey ceremony.
    pub root_did: String,
    /// CID of the exact root-to-device grant.
    pub delegation_cid: String,
    /// Exact grant bytes, hex encoded.
    pub delegation_hex: String,
}

fn missing_credential(error: &CredentialError) -> bool {
    matches!(error, CredentialError::NotFound(_))
        || matches!(error, CredentialError::Storage(message) if message.contains("No such file or directory"))
}

/// Load the provider-neutral local root, if one has been provisioned.
///
/// Mounts the profile's account operator first: a bare
/// `Storage::default()` has no mounts, so performing a credential load
/// against one fails with "no mount for {did}" before it ever reaches
/// the store — on every machine, provisioned or not.
pub async fn local_root(profile: &Profile) -> Result<Option<LocalRoot>> {
    let operator = crate::account_state::credential_operator(profile).await?;
    local_root_with_operator(profile, &operator).await
}

/// Load the local root through an already-mounted site operator.
pub(crate) async fn local_root_with_operator(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<LocalRoot>> {
    let bytes = match profile
        .credential()
        .site(LOCAL_ROOT_SITE)
        .load::<Vec<u8>>()
        .perform(operator)
        .await
    {
        Ok(bytes) if bytes.is_empty() => return Ok(None),
        Ok(bytes) => bytes,
        Err(error) if missing_credential(&error) => return Ok(None),
        Err(error) => return Err(error).context("failed to load the local root"),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .context("stored local root is malformed")
}

/// Validate and persist exact root-to-device material from a browser handoff.
pub async fn save_local_root(
    profile: &Profile,
    credential_id: String,
    delegation_hex: String,
) -> Result<LocalRoot> {
    let bytes = hex::decode(&delegation_hex).context("invalid local-root delegation hex")?;
    let chain = DelegationChain::try_from(bytes.as_slice())
        .context("invalid local-root delegation container")?;
    let grant = tonk_identity::delegation::validate_account_grant(&chain, &profile.did())
        .await
        .context("local-root delegation is not usable account authority")?;
    let record = LocalRoot {
        credential_id,
        root_did: grant.root_did.to_string(),
        delegation_cid: grant.delegation_cid.to_string(),
        delegation_hex,
    };
    // The latest handoff replaces this compatibility projection. Historical
    // UCAN certificates remain installed for local repository writes.
    let operator = crate::account_state::credential_operator(profile).await?;
    profile
        .save(UcanDelegation(chain))
        .perform(&operator)
        .await
        .context("failed to install the local-root delegation")?;
    profile
        .credential()
        .site(LOCAL_ROOT_SITE)
        .save(serde_json::to_vec(&record).context("failed to serialize the local root")?)
        .perform(&operator)
        .await
        .context("failed to persist the local root")?;
    Ok(record)
}

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
