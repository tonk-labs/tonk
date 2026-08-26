//! Durable native account-session state and cross-process transition locking.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use tonk_account::detach::SignedDetachIntent;

use crate::space::SpaceStore;

/// Credential site containing the sole native remote-account authority state.
pub const ACCOUNT_SESSION_SITE: &str = "tonk-account-session-v1";
const VERSION: u8 = 1;
const LOCK_FILE: &str = "account-session.lock";
const STATE_FILE_PREFIX: &str = ACCOUNT_SESSION_SITE;

/// One durable account-session transition record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSessionState {
    /// Record version.
    pub version: u8,
    /// The only attachment authorized for remote account access.
    pub active: Option<ActiveAccount>,
    /// Crash-recoverable browser handoff phase.
    pub pending_login: Option<PendingLogin>,
}

impl Default for AccountSessionState {
    fn default() -> Self {
        Self {
            version: VERSION,
            active: None,
            pending_login: None,
        }
    }
}

/// Durable browser handoff phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingLogin {
    /// The browser has not completed the handoff yet.
    Waiting {
        /// Provider service URL.
        provider: String,
        /// Raw one-time secret needed to resume consumption.
        secret: String,
        /// Hash sent to the provider.
        token_hash: String,
    },
    /// Grant material is durable locally and activation can be replayed.
    Activating {
        /// Provider service URL.
        provider: String,
        /// Raw secret retained for recovery diagnostics/re-consumption.
        secret: String,
        /// Exact completed account generation.
        account: ActiveAccount,
    },
}

/// Exact account grant authorized for current remote use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveAccount {
    /// Provider service URL.
    pub provider: String,
    /// Passkey credential identifier.
    pub credential_id: String,
    /// Account root DID.
    pub root_did: String,
    /// CID of the exact root-to-device grant.
    pub delegation_cid: String,
    /// Exact root-to-device grant bytes, hex encoded.
    pub delegation_hex: String,
    /// Exact account repository descriptor bytes, hex encoded.
    pub descriptor_hex: Option<String>,
    /// Service-generated attachment generation.
    pub attachment_id: String,
    /// Provider attachment time.
    pub attached_at: u64,
}

/// Held shared lock covering active-state read through remote dispatch.
pub struct AccountSessionReadGuard {
    _file: File,
    store: SpaceStore,
}

/// Held exclusive lock covering one account lifecycle transition.
pub struct AccountSessionWriteGuard {
    _file: File,
    store: SpaceStore,
}

fn lock_file(store: &SpaceStore) -> Result<File> {
    let dir = store.account_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create account state at {}", dir.display()))?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(LOCK_FILE))
        .context("failed to open the account-session lock")
}

/// Acquire the cross-process shared remote-dispatch guard.
pub fn shared_remote_guard(store: &SpaceStore) -> Result<AccountSessionReadGuard> {
    let file = lock_file(store)?;
    file.lock_shared()
        .context("failed to acquire the account-session read lock")?;
    Ok(AccountSessionReadGuard {
        _file: file,
        store: store.clone(),
    })
}

/// Acquire the cross-process exclusive account-transition guard.
pub fn exclusive_transition_guard(store: &SpaceStore) -> Result<AccountSessionWriteGuard> {
    let file = lock_file(store)?;
    file.lock()
        .context("failed to acquire the account-session write lock")?;
    Ok(AccountSessionWriteGuard {
        _file: file,
        store: store.clone(),
    })
}

pub(crate) fn state_path(profile: &Profile, store: &SpaceStore) -> Result<PathBuf> {
    let profile_key = blake3::hash(profile.did().as_ref().as_bytes()).to_hex();
    Ok(store
        .account_dir()
        .join(format!("{STATE_FILE_PREFIX}-{profile_key}.json")))
}

async fn load_raw(
    profile: &Profile,
    _operator: &Operator<NativeSpace>,
    store: &SpaceStore,
) -> Result<Option<AccountSessionState>> {
    let path = state_path(profile, store)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to load account session at {}", path.display()));
        }
    };
    let state: AccountSessionState =
        serde_json::from_slice(&bytes).context("stored account-session state is malformed")?;
    if state.version != VERSION {
        anyhow::bail!(
            "unsupported account-session state version {}",
            state.version
        );
    }
    Ok(Some(state))
}

async fn save_raw(
    profile: &Profile,
    _operator: &Operator<NativeSpace>,
    store: &SpaceStore,
    state: &AccountSessionState,
) -> Result<()> {
    let path = state_path(profile, store)?;
    let parent = path
        .parent()
        .context("account-session path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create account state at {}", parent.display()))?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(state).context("failed to serialize account-session state")?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .with_context(|| {
            format!(
                "failed to create account-session temp file {}",
                tmp.display()
            )
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .context("failed to protect account-session temp file")?;
    }
    file.write_all(&bytes)
        .context("failed to write account-session temp file")?;
    file.sync_all()
        .context("failed to sync account-session temp file")?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .context("failed to sync the account-session directory")?;
    Ok(())
}

/// Project the persisted root and provider record into an active
/// session, when both are present. This is what the legacy migration in
/// [`ensure_initialized`] reads, and what a completed callback link
/// activates from.
async fn projected_active(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<Option<ActiveAccount>> {
    let Some(root) = crate::identity::local_root_with_operator(profile, operator).await? else {
        return Ok(None);
    };
    let root_did: Did = root
        .root_did
        .parse()
        .context("stored root DID is invalid")?;
    let bytes = profile
        .credential()
        .site(crate::account::ACCOUNT_LINK_SITE)
        .load::<Vec<u8>>()
        .perform(operator)
        .await;
    let Ok(bytes) = bytes else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let Some(provider) = tonk_account::AccountProviderRecord::decode(&bytes, &root_did)
        .await
        .context("stored provider record is unusable")?
    else {
        return Ok(None);
    };
    Ok(Some(ActiveAccount {
        provider: provider.provider().to_string(),
        credential_id: root.credential_id,
        root_did: root.root_did,
        delegation_cid: root.delegation_cid.clone(),
        delegation_hex: root.delegation_hex,
        descriptor_hex: provider
            .descriptor()
            .map(|value| hex::encode(value.bytes())),
        attachment_id: root.delegation_cid,
        attached_at: provider.attached_at().unwrap_or_default(),
    }))
}

/// Initialize canonical state once, migrating the legacy root/provider
/// projection while the caller holds the exclusive lock.
pub async fn ensure_initialized(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionWriteGuard,
) -> Result<()> {
    if load_raw(profile, operator, &guard.store).await?.is_some() {
        return Ok(());
    }
    let state = AccountSessionState {
        active: projected_active(profile, operator).await?,
        ..Default::default()
    };
    save_raw(profile, operator, &guard.store, &state).await
}

/// Activate the session for a link that just persisted its root and
/// provider record: the callback flow's counterpart to the legacy
/// migration above, which only runs when no canonical state exists yet.
pub async fn activate_link(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
    attachment_id: Option<String>,
) -> Result<()> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut active = projected_active(profile, operator)
        .await?
        .context("link completed but its root and provider record did not persist")?;
    if let Some(attachment_id) = attachment_id {
        active.attachment_id = attachment_id;
    }
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    state.active = Some(active);
    state.pending_login = None;
    save_raw(profile, operator, store, &state).await
}

/// Strictly read canonical state while the caller retains a shared lock.
pub async fn load_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionReadGuard,
) -> Result<AccountSessionState> {
    load_raw(profile, operator, &guard.store)
        .await?
        .context("account-session state has not been initialized")
}

/// Read canonical state under a shared lock without requiring that the
/// exact-profile sidecar has already been initialized.
pub(crate) async fn load_optional_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionReadGuard,
) -> Result<Option<AccountSessionState>> {
    load_raw(profile, operator, &guard.store).await
}

/// Read the sole active attachment under an existing shared guard.
pub async fn active_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionReadGuard,
) -> Result<Option<ActiveAccount>> {
    Ok(load_guarded(profile, operator, guard).await?.active)
}

/// Commit local logout in one explicit profile store.
///
/// Logout is local-first: the durable transition never depends on a provider
/// being reachable. The returned attachments are notified best-effort.
pub async fn logout_transition_for_store(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
) -> Result<Vec<ActiveAccount>> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let mut detached = Vec::new();
    if let Some(active) = state.active.take() {
        detached.push(active);
    }
    if let Some(PendingLogin::Activating { account, .. }) = state.pending_login.take() {
        detached.push(account);
    }
    let existed = !detached.is_empty() || state.pending_login.is_some();
    state.pending_login = None;
    if existed {
        save_raw(profile, operator, store, &state).await?;
        // Compatibility only: canonical state is already authoritative.
        let _ = profile
            .credential()
            .site(crate::account::ACCOUNT_LINK_SITE)
            .save(Vec::<u8>::new())
            .perform(operator)
            .await;
    }
    Ok(detached)
}

/// Tell `account`'s provider this attachment ended: one signed POST,
/// no retry. See [`logout_transition_for_store`] for why failure is tolerable.
pub async fn deliver_detach(profile: &Profile, account: &ActiveAccount, now: u64) -> Result<()> {
    let root: Did = account
        .root_did
        .parse()
        .context("active root DID is invalid")?;
    let intent = SignedDetachIntent::sign(
        profile.signer(),
        &root,
        &account.attachment_id,
        &account.delegation_cid,
        now,
    )
    .await
    .context("failed to sign account detach intent")?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/devices/detach",
            account.provider.trim_end_matches('/')
        ))
        .json(&intent)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to reach the account provider")?;
    if !response.status().is_success() {
        anyhow::bail!("provider answered {}", response.status());
    }
    Ok(())
}

#[cfg(feature = "integration-tests")]
pub(crate) async fn install_for_integration_test(
    profile: &Profile,
    operator: &crate::account_authority::AccountBoundOperator,
    state: &AccountSessionState,
) -> Result<()> {
    let store = operator.store();
    let _guard = exclusive_transition_guard(store)?;
    save_raw(profile, operator.local(), store, state).await
}
