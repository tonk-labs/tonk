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

use crate::spot::SpotStore;

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
    /// Signed detach intents awaiting provider delivery.
    pub pending_detaches: Vec<PendingDetach>,
}

impl Default for AccountSessionState {
    fn default() -> Self {
        Self {
            version: VERSION,
            active: None,
            pending_login: None,
            pending_detaches: Vec::new(),
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

/// One provider-routed detach outbox entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDetach {
    /// Provider service URL.
    pub provider: String,
    /// Narrow generation-bound signed intent.
    pub intent: SignedDetachIntent,
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
    store: SpotStore,
}

/// Held exclusive lock covering one account lifecycle transition.
pub struct AccountSessionWriteGuard {
    _file: File,
    store: SpotStore,
}

fn lock_file(store: &SpotStore) -> Result<File> {
    let dir = store.account_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create account state at {}", dir.display()))?;
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(dir.join(LOCK_FILE))
        .context("failed to open the account-session lock")
}

/// Acquire the cross-process shared remote-dispatch guard.
pub fn shared_remote_guard(store: &SpotStore) -> Result<AccountSessionReadGuard> {
    let file = lock_file(store)?;
    file.lock_shared()
        .context("failed to acquire the account-session read lock")?;
    Ok(AccountSessionReadGuard {
        _file: file,
        store: store.clone(),
    })
}

/// Acquire the cross-process exclusive account-transition guard.
pub fn exclusive_transition_guard(store: &SpotStore) -> Result<AccountSessionWriteGuard> {
    let file = lock_file(store)?;
    file.lock()
        .context("failed to acquire the account-session write lock")?;
    Ok(AccountSessionWriteGuard {
        _file: file,
        store: store.clone(),
    })
}

fn state_path(profile: &Profile, store: &SpotStore) -> Result<PathBuf> {
    let profile_key = blake3::hash(profile.did().as_ref().as_bytes()).to_hex();
    Ok(store
        .account_dir()
        .join(format!("{STATE_FILE_PREFIX}-{profile_key}.json")))
}

async fn load_raw(
    profile: &Profile,
    _operator: &Operator<NativeSpace>,
    store: &SpotStore,
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
    store: &SpotStore,
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
    let mut state = AccountSessionState::default();
    if let Some(root) = crate::identity::local_root_with_operator(profile, operator).await? {
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
        if let Ok(bytes) = bytes
            && !bytes.is_empty()
            && let Some(provider) = tonk_account::AccountProviderRecord::decode(&bytes, &root_did)
                .await
                .context("stored legacy provider is unusable")?
        {
            state.active = Some(ActiveAccount {
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
            });
        }
    }
    save_raw(profile, operator, &guard.store, &state).await
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

/// Read the sole active attachment under an existing shared guard.
pub async fn active_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionReadGuard,
) -> Result<Option<ActiveAccount>> {
    Ok(load_guarded(profile, operator, guard).await?.active)
}

/// Save a waiting or activating handoff under an exclusive transition.
pub async fn begin_login(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    pending: PendingLogin,
) -> Result<()> {
    let store = SpotStore::open().context("failed to locate account state")?;
    let guard = exclusive_transition_guard(&store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, &store)
        .await?
        .unwrap_or_default();
    if state.active.is_some() {
        anyhow::bail!("an account is already active; log out before linking another account");
    }
    let allowed = match (&state.pending_login, &pending) {
        (None, PendingLogin::Waiting { .. }) => true,
        (Some(existing), candidate) if existing == candidate => true,
        (
            Some(PendingLogin::Waiting {
                provider: old_provider,
                secret: old_secret,
                ..
            }),
            PendingLogin::Activating {
                provider, secret, ..
            },
        ) => old_provider == provider && old_secret == secret,
        _ => false,
    };
    if !allowed {
        anyhow::bail!("account handoff state changed; refusing a stale transition");
    }
    state.pending_login = Some(pending);
    save_raw(profile, operator, &store, &state).await
}

/// Require that the caller still owns the exact pending activation while it
/// holds the supplied exclusive transition guard.
pub async fn require_pending_activation(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    expected: &ActiveAccount,
    guard: &AccountSessionWriteGuard,
) -> Result<()> {
    let state = load_raw(profile, operator, &guard.store)
        .await?
        .unwrap_or_default();
    match state.pending_login {
        Some(PendingLogin::Activating { account, .. }) if &account == expected => Ok(()),
        _ => anyhow::bail!("account handoff was cancelled or replaced"),
    }
}

/// Atomically promote a confirmed activating handoff to active.
pub async fn finish_activation(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    expected: &ActiveAccount,
) -> Result<()> {
    let store = SpotStore::open().context("failed to locate account state")?;
    let guard = exclusive_transition_guard(&store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, &store)
        .await?
        .unwrap_or_default();
    let account = match state.pending_login.as_ref() {
        Some(PendingLogin::Activating { account, .. }) if account == expected => account.clone(),
        Some(PendingLogin::Activating { .. }) => {
            anyhow::bail!("another account handoff replaced this activation")
        }
        _ => anyhow::bail!("no completed account handoff is awaiting activation"),
    };
    state.pending_login = None;
    state.active = Some(account);
    save_raw(profile, operator, &store, &state).await
}

async fn queue_detach(
    profile: &Profile,
    state: &mut AccountSessionState,
    account: ActiveAccount,
    now: u64,
) -> Result<()> {
    for pending in &state.pending_detaches {
        if pending
            .intent
            .validate()
            .await
            .is_ok_and(|payload| payload.attachment_id == account.attachment_id)
        {
            return Ok(());
        }
    }
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
    state.pending_detaches.push(PendingDetach {
        provider: account.provider,
        intent,
    });
    Ok(())
}

/// Commit local logout in one canonical state write. Returns false when no
/// active or pending handoff existed.
pub async fn logout_transition(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    now: u64,
) -> Result<bool> {
    let store = SpotStore::open().context("failed to locate account state")?;
    let guard = exclusive_transition_guard(&store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, &store)
        .await?
        .unwrap_or_default();
    let existed = state.active.is_some() || state.pending_login.is_some();
    if let Some(active) = state.active.take() {
        queue_detach(profile, &mut state, active, now).await?;
    }
    if let Some(PendingLogin::Activating { account, .. }) = state.pending_login.take() {
        queue_detach(profile, &mut state, account, now).await?;
    }
    state.pending_login = None;
    if existed {
        save_raw(profile, operator, &store, &state).await?;
        // Compatibility only: canonical state is already authoritative.
        let _ = profile
            .credential()
            .site(crate::account::ACCOUNT_LINK_SITE)
            .save(Vec::<u8>::new())
            .perform(operator)
            .await;
    }
    Ok(existed)
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

/// Pending-detach retry summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlushOutcome {
    /// Number of entries removed after terminal provider outcomes.
    pub flushed: usize,
    /// Number retained for a later retry.
    pub pending: usize,
    /// Bounded warning for the first retained failure.
    pub warning: Option<String>,
}

/// Retry detach outbox entries under the exclusive transition lock.
pub async fn flush_pending(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
) -> Result<FlushOutcome> {
    let store = SpotStore::open().context("failed to locate account state")?;
    flush_pending_for_store(profile, operator, &store).await
}

pub(crate) async fn flush_pending_for_store(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpotStore,
) -> Result<FlushOutcome> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, &store)
        .await?
        .unwrap_or_default();
    if state.pending_detaches.is_empty() {
        return Ok(FlushOutcome::default());
    }
    let mut retained = Vec::new();
    let mut outcome = FlushOutcome::default();
    for pending in std::mem::take(&mut state.pending_detaches) {
        let result = reqwest::Client::new()
            .post(format!(
                "{}/devices/detach",
                pending.provider.trim_end_matches('/')
            ))
            .json(&pending.intent)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;
        let terminal = match result {
            Ok(response) if response.status().is_success() => response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|value| {
                    value
                        .get("outcome")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                })
                .is_some_and(|value| {
                    matches!(
                        value.as_str(),
                        "detached"
                            | "alreadyDetached"
                            | "cancelledPendingActivation"
                            | "superseded"
                            | "revoked"
                    )
                }),
            Ok(response) => {
                let status = response.status();
                outcome.warning.get_or_insert_with(|| {
                    format!("detach retry retained after provider status {status}")
                });
                false
            }
            Err(error) => {
                outcome
                    .warning
                    .get_or_insert_with(|| format!("detach retry retained: {error}"));
                false
            }
        };
        if terminal {
            outcome.flushed += 1;
        } else {
            retained.push(pending);
        }
    }
    outcome.pending = retained.len();
    state.pending_detaches = retained;
    save_raw(profile, operator, &store, &state).await?;
    Ok(outcome)
}
