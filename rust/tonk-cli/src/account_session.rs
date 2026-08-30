//! Durable native account-session state and cross-process transition locking.

use std::fs::{File, OpenOptions, TryLockError};
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
const VERSION: u8 = 2;
const LEGACY_VERSION: u8 = 1;
const LOCK_FILE: &str = "account-session.lock";
const HANDOFF_LOCK_FILE: &str = "account-handoff.lock";
const STATE_FILE_PREFIX: &str = ACCOUNT_SESSION_SITE;

/// One durable account-session transition record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSessionState {
    /// Record version.
    pub version: u8,
    /// The only attachment authorized for remote account access.
    pub active: Option<ActiveAccount>,
    /// Crash-recoverable activation or a legacy browser handoff.
    pub pending_login: Option<PendingLogin>,
    /// Signed provider cleanup requests that survive local logout/restart.
    #[serde(default)]
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

/// One durable, generation-bound provider cleanup request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDetach {
    /// Provider base URL that owns the attachment.
    pub provider: String,
    /// Device-signed exact-generation detach intent.
    pub signed_intent: SignedDetachIntent,
    /// Unix time when local logout queued this item.
    pub queued_at: u64,
}

/// Result of atomically signing out locally and queueing provider cleanup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogoutTransition {
    /// Whether active or pending local login state was cleared.
    pub signed_out: bool,
    /// Number of newly queued detach intents.
    pub queued: usize,
}

/// Bounded best-effort cleanup result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushSummary {
    /// Terminal provider acknowledgements removed from the outbox.
    pub retired: usize,
    /// Items retained because delivery was unavailable or timed out.
    pub retryable: usize,
    /// Items retained because their local/provider representation is invalid.
    pub permanently_malformed: usize,
}

impl FlushSummary {
    /// Combine two flushes performed at one command boundary.
    pub fn merge(&mut self, other: Self) {
        self.retired += other.retired;
        self.retryable += other.retryable;
        self.permanently_malformed += other.permanently_malformed;
    }

    /// Whether any cleanup still needs attention.
    pub fn has_pending(self) -> bool {
        self.retryable > 0 || self.permanently_malformed > 0
    }
}

fn validate_state(state: &AccountSessionState) -> Result<()> {
    if state.version != VERSION {
        anyhow::bail!(
            "unsupported account-session state version {}",
            state.version
        );
    }
    if state.active.is_some() && state.pending_login.is_some() {
        anyhow::bail!("account-session state cannot be active and pending simultaneously");
    }
    Ok(())
}

fn decode_state(bytes: &[u8]) -> Result<AccountSessionState> {
    let mut state: AccountSessionState =
        serde_json::from_slice(bytes).context("stored account-session state is malformed")?;
    match state.version {
        LEGACY_VERSION => {
            // Version 1 had no outbox. Ignore any injected field while
            // migrating in memory; the source file is not rewritten by a read.
            state.version = VERSION;
            state.pending_detaches.clear();
        }
        VERSION => {}
        version => anyhow::bail!("unsupported account-session state version {version}"),
    }
    validate_state(&state)?;
    Ok(state)
}

/// Provider sign-in phase visible without opening a Dialog profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPhase {
    /// No canonical session state exists or it is inactive.
    SignedOut,
    /// A browser handoff is durable but not active yet.
    Pending,
    /// One provider attachment is active.
    Active,
}

/// Inspect a profile store without creating locks, directories, or profiles.
pub fn inspect_local(store: &SpaceStore) -> Result<LocalPhase> {
    let account = store.account_dir();
    let entries = match std::fs::read_dir(&account) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LocalPhase::SignedOut);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect account state at {}", account.display())
            });
        }
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(STATE_FILE_PREFIX) || !name.ends_with(".json") {
            continue;
        }
        let bytes = std::fs::read(entry.path())?;
        let state = decode_state(&bytes)?;
        return Ok(if state.active.is_some() {
            LocalPhase::Active
        } else if state.pending_login.is_some() {
            LocalPhase::Pending
        } else {
            LocalPhase::SignedOut
        });
    }
    Ok(LocalPhase::SignedOut)
}

/// Durable login recovery phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingLogin {
    /// A legacy browser handoff whose process-local callback cannot be
    /// resumed. A new client reports this state and asks the user to log out.
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

/// Exclusive token retained from durable activation staging through final
/// promotion. Its contents are intentionally private: only this module may
/// decide which staged generation the token authorizes.
pub struct AccountActivationGuard {
    _guard: AccountSessionWriteGuard,
    pending: PendingLogin,
}

/// Cross-process exclusion held while a browser may adopt a provider
/// generation that has not reached the local callback yet.
#[derive(Debug)]
pub struct AccountHandoffGuard {
    _file: File,
}

/// Shared exclusion for a local transition that must not race browser
/// registration.
#[derive(Debug)]
pub struct AccountOperationGuard {
    _file: File,
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

fn handoff_lock_file(store: &SpaceStore) -> Result<File> {
    let dir = store.account_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create account state at {}", dir.display()))?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(HANDOFF_LOCK_FILE))
        .context("failed to open the account-handoff lock")
}

/// Wait for bounded cleanup already in flight, then exclude cleanup and other
/// login processes while browser registration can precede its callback.
pub async fn begin_handoff(store: &SpaceStore) -> Result<AccountHandoffGuard> {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(11);
    let file = handoff_lock_file(store)?;
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(AccountHandoffGuard { _file: file }),
            Err(TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(TryLockError::WouldBlock) => {
                anyhow::bail!(
                    "another account login is waiting for browser approval; finish or cancel it, then try again"
                );
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).context("failed to lock the account handoff");
            }
        }
    }
}

/// Refuse a local transition while another process is between browser
/// registration and callback activation.
pub fn begin_account_operation(store: &SpaceStore) -> Result<AccountOperationGuard> {
    let file = handoff_lock_file(store)?;
    match file.try_lock_shared() {
        Ok(()) => Ok(AccountOperationGuard { _file: file }),
        Err(TryLockError::WouldBlock) => {
            anyhow::bail!("another account login is waiting for browser approval")
        }
        Err(TryLockError::Error(error)) => Err(error).context("failed to lock the account handoff"),
    }
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

fn state_path(profile: &Profile, store: &SpaceStore) -> Result<PathBuf> {
    let profile_key = blake3::hash(profile.did().as_ref().as_bytes()).to_hex();
    Ok(store
        .account_dir()
        .join(format!("{STATE_FILE_PREFIX}-{profile_key}.json")))
}

fn legacy_provider_bytes(
    result: std::result::Result<Vec<u8>, dialog_effects::credential::CredentialError>,
) -> Result<Option<Vec<u8>>> {
    match result {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if crate::account_state::credential_is_missing(&error) => Ok(None),
        Err(error) => Err(error).context("failed to load the legacy account provider"),
    }
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
    let state = decode_state(&bytes)?;
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

/// Project the persisted root and provider record into an active session,
/// when both are present. This is used only to migrate legacy installs that
/// predate canonical account-session state.
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
    let Some(bytes) = legacy_provider_bytes(
        profile
            .credential()
            .site(crate::account::ACCOUNT_LINK_SITE)
            .load::<Vec<u8>>()
            .perform(operator)
            .await,
    )?
    else {
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

/// Persist the exact post-callback account generation before compatibility
/// projection writes begin, retaining the exclusive transition lock.
pub async fn stage_activation(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
    account: ActiveAccount,
) -> Result<AccountActivationGuard> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let pending = PendingLogin::Activating { account };
    match (&state.active, &state.pending_login) {
        (None, None) => {
            state.pending_login = Some(pending.clone());
            save_raw(profile, operator, store, &state).await?;
        }
        (None, Some(existing)) if existing == &pending => {}
        _ => {
            anyhow::bail!(
                "cannot stage account activation while another account transition exists"
            );
        }
    }
    Ok(AccountActivationGuard {
        _guard: guard,
        pending,
    })
}

/// Atomically promote the exact staged callback generation to active.
pub async fn finalize_activation(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: AccountActivationGuard,
    account: &ActiveAccount,
) -> Result<()> {
    let expected = PendingLogin::Activating {
        account: account.clone(),
    };
    if guard.pending != expected {
        anyhow::bail!("activation guard belongs to a different account generation");
    }
    let store = &guard._guard.store;
    let mut state = load_raw(profile, operator, store)
        .await?
        .context("account-session state has not been initialized")?;
    if state.active.is_some() || state.pending_login.as_ref() != Some(&expected) {
        anyhow::bail!("staged account activation changed before finalization");
    }
    state.active = Some(account.clone());
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
) -> Result<LogoutTransition> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let existed = state.active.is_some() || state.pending_login.is_some();
    let mut accounts = Vec::new();
    if let Some(active) = state.active.as_ref() {
        accounts.push(active.clone());
    }
    if let Some(PendingLogin::Activating { account, .. }) = state.pending_login.as_ref() {
        accounts.push(account.clone());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut queued = 0;
    for account in accounts {
        let mut already_queued = false;
        for pending in &state.pending_detaches {
            if pending.provider.trim_end_matches('/') == account.provider.trim_end_matches('/')
                && pending
                    .signed_intent
                    .validate()
                    .await
                    .map(|payload| payload.attachment_id == account.attachment_id)
                    .unwrap_or(false)
            {
                already_queued = true;
                break;
            }
        }
        if already_queued {
            continue;
        }
        let root: Did = account
            .root_did
            .parse()
            .context("active root DID is invalid")?;
        let signed_intent = SignedDetachIntent::sign(
            profile.signer(),
            &root,
            &account.attachment_id,
            &account.delegation_cid,
            now,
        )
        .await
        .context("failed to sign account detach intent")?;
        state.pending_detaches.push(PendingDetach {
            provider: account.provider.trim_end_matches('/').to_string(),
            signed_intent,
            queued_at: now,
        });
        queued += 1;
    }
    // Signing every required intent succeeded. Persist queue + local sign-out
    // together so neither can become durable without the other.
    state.active = None;
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
    Ok(LogoutTransition {
        signed_out: existed,
        queued,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    Terminal,
    Retryable,
    Malformed,
}

async fn deliver_detach(pending: &PendingDetach, active: Option<&ActiveAccount>) -> Delivery {
    let payload = match pending.signed_intent.validate().await {
        Ok(payload) => payload,
        Err(_) => return Delivery::Malformed,
    };
    // A same-account retry may intentionally recover the still-active
    // generation whose earlier logout could not deliver. Never let that stale
    // queued intent detach the newly re-adopted local session; it remains
    // queued until a later logout clears active state again.
    if active.is_some_and(|active| {
        active.provider.trim_end_matches('/') == pending.provider.trim_end_matches('/')
            && active.attachment_id == payload.attachment_id
            && active.delegation_cid == payload.delegation_cid
    }) {
        return Delivery::Retryable;
    }
    let endpoint = match reqwest::Url::parse(&format!(
        "{}/devices/detach",
        pending.provider.trim_end_matches('/')
    )) {
        Ok(endpoint)
            if matches!(endpoint.scheme(), "https" | "http")
                && endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none() =>
        {
            endpoint
        }
        _ => return Delivery::Malformed,
    };
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&pending.signed_intent)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    let Ok(response) = response else {
        return Delivery::Retryable;
    };
    if response.status().is_server_error()
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Delivery::Retryable;
    }
    if !response.status().is_success() {
        return Delivery::Malformed;
    }
    let Ok(receipt) = response.json::<serde_json::Value>().await else {
        return Delivery::Malformed;
    };
    match receipt.get("outcome").and_then(serde_json::Value::as_str) {
        Some("detached" | "alreadyDetached" | "superseded" | "revoked") => Delivery::Terminal,
        _ => Delivery::Malformed,
    }
}

/// Deliver a bounded batch of durable detach intents. Only exact terminal
/// provider acknowledgements remove an item; every other outcome remains for
/// a later command/restart.
pub async fn flush_pending_detaches(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
) -> Result<FlushSummary> {
    flush_pending_detaches_inner(profile, operator, store, false).await
}

/// Flush while the caller already holds either the exclusive browser-handoff
/// guard or a shared account-operation guard.
pub(crate) async fn flush_pending_detaches_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
) -> Result<FlushSummary> {
    flush_pending_detaches_inner(profile, operator, store, true).await
}

async fn flush_pending_detaches_inner(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpaceStore,
    handoff_guarded: bool,
) -> Result<FlushSummary> {
    const MAX_BATCH: usize = 8;
    const TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    // Browser registration happens outside this process and may commit before
    // the callback makes its adopted generation locally visible. Holding this
    // shared lock through dispatch closes that window; a login holds the
    // exclusive side from preflight through callback activation. Callers that
    // already hold one side must not lock a second file descriptor.
    let _handoff_guard = if handoff_guarded {
        None
    } else {
        let file = handoff_lock_file(store)?;
        match file.try_lock_shared() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock) => {
                let guard = exclusive_transition_guard(store)?;
                ensure_initialized(profile, operator, &guard).await?;
                let pending = load_raw(profile, operator, store)
                    .await?
                    .unwrap_or_default()
                    .pending_detaches
                    .len();
                return Ok(FlushSummary {
                    retryable: pending,
                    ..Default::default()
                });
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).context("failed to lock provider cleanup against login");
            }
        }
    };

    let (pending, active) = {
        let guard = exclusive_transition_guard(store)?;
        ensure_initialized(profile, operator, &guard).await?;
        let state = load_raw(profile, operator, store)
            .await?
            .unwrap_or_default();
        (state.pending_detaches, state.active)
    };
    if pending.is_empty() {
        return Ok(FlushSummary::default());
    }
    let attempted: Vec<PendingDetach> = pending.iter().take(MAX_BATCH).cloned().collect();
    let deliveries = tokio::time::timeout(TOTAL_TIMEOUT, async {
        let mut results = Vec::with_capacity(attempted.len());
        for item in &attempted {
            results.push((item.clone(), deliver_detach(item, active.as_ref()).await));
        }
        results
    })
    .await;
    let Ok(deliveries) = deliveries else {
        return Ok(FlushSummary {
            retryable: pending.len(),
            ..Default::default()
        });
    };

    let terminal: Vec<PendingDetach> = deliveries
        .iter()
        .filter(|(_, delivery)| *delivery == Delivery::Terminal)
        .map(|(pending, _)| pending.clone())
        .collect();
    let mut summary = FlushSummary {
        retired: terminal.len(),
        retryable: pending.len().saturating_sub(attempted.len())
            + deliveries
                .iter()
                .filter(|(_, delivery)| *delivery == Delivery::Retryable)
                .count(),
        permanently_malformed: deliveries
            .iter()
            .filter(|(_, delivery)| *delivery == Delivery::Malformed)
            .count(),
    };
    if terminal.is_empty() {
        return Ok(summary);
    }
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let before = state.pending_detaches.len();
    state
        .pending_detaches
        .retain(|pending| !terminal.contains(pending));
    let removed = before - state.pending_detaches.len();
    if removed > 0 {
        save_raw(profile, operator, store, &state).await?;
    }
    // Another process may already have removed an exact item while this one
    // delivered it. Report only what this transition retired locally.
    summary.retired = removed;
    Ok(summary)
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

#[cfg(test)]
mod tests {
    use dialog_capability::Subject;
    use dialog_effects::storage::Directory;
    use dialog_operator::DeriveOperator as _;
    use dialog_storage::provider::storage::Storage;

    use super::*;

    async fn isolated_session() -> (
        tempfile::TempDir,
        SpaceStore,
        Profile,
        Operator<NativeSpace>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let store = SpaceStore::at(temp.path().join("state"));
        let profile_dir = Directory::At(temp.path().join("profiles").to_string_lossy().into());
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(format!("account-session-test-{}", rand::random::<u64>()))
            .at(profile_dir)
            .perform(&storage)
            .await
            .unwrap();
        std::fs::create_dir_all(store.account_dir()).unwrap();
        let account_dir = store.account_dir().canonicalize().unwrap();
        let operator = profile
            .derive(b"tonk/account-session-test/v1")
            .allow(Subject::any())
            .base(Directory::At(account_dir.to_string_lossy().into()))
            .build(storage)
            .await
            .unwrap();
        (temp, store, profile, operator)
    }

    fn active_account(attachment_id: &str) -> ActiveAccount {
        ActiveAccount {
            provider: "https://accounts.example".to_string(),
            credential_id: "credential".to_string(),
            root_did: "did:key:z6MkhFDyBYNT1Y1jNj8RJKVc7CWurCVPmrnGEGmbYxvwHJkX".to_string(),
            delegation_cid: "bafk-delegation".to_string(),
            delegation_hex: "00".to_string(),
            descriptor_hex: Some("01".to_string()),
            attachment_id: attachment_id.to_string(),
            attached_at: 42,
        }
    }

    #[dialog_common::test]
    fn activating_decodes_the_legacy_duplicate_fields_but_does_not_reemit_them() {
        let account = active_account("legacy-generation");
        let legacy = serde_json::json!({
            "activating": {
                "provider": "https://accounts.example",
                "secret": "legacy-unused-secret",
                "account": account,
            }
        });

        let decoded: PendingLogin = serde_json::from_value(legacy).unwrap();

        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::json!({
                "activating": {
                    "account": active_account("legacy-generation"),
                }
            })
        );
    }

    #[dialog_common::test]
    fn legacy_migration_distinguishes_absence_from_storage_failure() {
        use dialog_effects::credential::CredentialError;

        assert_eq!(
            legacy_provider_bytes(Err(CredentialError::NotFound("missing".to_owned()))).unwrap(),
            None
        );
        let error = legacy_provider_bytes(Err(CredentialError::Storage(
            "permission denied".to_owned(),
        )))
        .expect_err("a transient provider read failure must not become signed out");
        assert!(error.to_string().contains("legacy account provider"));
    }

    #[dialog_common::test]
    async fn inspect_local_distinguishes_signed_out_pending_and_active_states() {
        let (_temp, store, profile, operator) = isolated_session().await;

        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::SignedOut);

        save_raw(&profile, &operator, &store, &AccountSessionState::default())
            .await
            .unwrap();
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::SignedOut);

        let mut state = AccountSessionState {
            pending_login: Some(PendingLogin::Waiting {
                provider: "https://accounts.example".to_string(),
                secret: "one-time-secret".to_string(),
                token_hash: "token-hash".to_string(),
            }),
            ..Default::default()
        };
        save_raw(&profile, &operator, &store, &state).await.unwrap();
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::Pending);

        state.pending_login = Some(PendingLogin::Activating {
            account: active_account("pending-generation"),
        });
        save_raw(&profile, &operator, &store, &state).await.unwrap();
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::Pending);

        state.pending_login = None;
        state.active = Some(active_account("active-generation"));
        save_raw(&profile, &operator, &store, &state).await.unwrap();
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::Active);
    }

    #[dialog_common::test]
    async fn version_one_state_loads_as_version_two_without_rewriting_the_file() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let path = state_path(&profile, &store).unwrap();
        let legacy = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "active": null,
            "pending_login": null,
        }))
        .unwrap();
        std::fs::write(&path, &legacy).unwrap();

        let loaded = load_raw(&profile, &operator, &store)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(
            serde_json::to_value(&loaded).unwrap()["pending_detaches"],
            serde_json::json!([])
        );
        assert_eq!(std::fs::read(path).unwrap(), legacy);
    }

    #[dialog_common::test]
    async fn active_and_pending_authority_fails_closed() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let state = AccountSessionState {
            active: Some(active_account("active-generation")),
            pending_login: Some(PendingLogin::Activating {
                account: active_account("pending-generation"),
            }),
            ..Default::default()
        };
        save_raw(&profile, &operator, &store, &state).await.unwrap();

        let error =
            inspect_local(&store).expect_err("contradictory state must not be reported as active");
        assert!(error.to_string().contains("active and pending"));

        let guard = shared_remote_guard(&store).unwrap();
        let error = load_guarded(&profile, &operator, &guard)
            .await
            .expect_err("contradictory state must not authorize a remote request");
        assert!(error.to_string().contains("active and pending"));
    }

    #[dialog_common::test]
    async fn stage_activation_is_the_first_durable_post_callback_write() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("callback-generation");

        let guard = stage_activation(&profile, &operator, &store, account.clone())
            .await
            .unwrap();

        assert_eq!(
            load_raw(&profile, &operator, &store).await.unwrap(),
            Some(AccountSessionState {
                version: VERSION,
                active: None,
                pending_login: Some(PendingLogin::Activating { account }),
                pending_detaches: Vec::new(),
            })
        );
        drop(guard);
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::Pending);
    }

    #[dialog_common::test]
    async fn activation_resume_is_idempotent_and_rejects_a_different_generation() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("callback-generation");
        let mut other = account.clone();
        other.delegation_cid = "fresh-delegation".to_owned();
        other.delegation_hex = "02".to_owned();

        drop(
            stage_activation(&profile, &operator, &store, account.clone())
                .await
                .unwrap(),
        );
        drop(
            stage_activation(&profile, &operator, &store, account.clone())
                .await
                .expect("reopening the exact generation is idempotent"),
        );

        let error = match stage_activation(&profile, &operator, &store, other).await {
            Ok(_) => panic!("a different callback generation must not replace pending state"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("another account transition"));
        assert_eq!(
            load_raw(&profile, &operator, &store).await.unwrap(),
            Some(AccountSessionState {
                version: VERSION,
                active: None,
                pending_login: Some(PendingLogin::Activating { account }),
                pending_detaches: Vec::new(),
            })
        );
    }

    #[dialog_common::test]
    async fn finalize_activation_promotes_only_the_exact_staged_generation() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("callback-generation");
        let other = active_account("other-generation");

        let guard = stage_activation(&profile, &operator, &store, account.clone())
            .await
            .unwrap();
        let error = finalize_activation(&profile, &operator, guard, &other)
            .await
            .expect_err("a stale finalizer must not promote another generation");
        assert!(error.to_string().contains("different account generation"));

        assert_eq!(
            load_raw(&profile, &operator, &store).await.unwrap(),
            Some(AccountSessionState {
                version: VERSION,
                active: None,
                pending_login: Some(PendingLogin::Activating {
                    account: account.clone(),
                }),
                pending_detaches: Vec::new(),
            })
        );

        let guard = stage_activation(&profile, &operator, &store, account.clone())
            .await
            .expect("the exact generation remains resumable");
        finalize_activation(&profile, &operator, guard, &account)
            .await
            .expect("the exact generation is promoted");
        assert_eq!(
            load_raw(&profile, &operator, &store).await.unwrap(),
            Some(AccountSessionState {
                version: VERSION,
                active: Some(account),
                pending_login: None,
                pending_detaches: Vec::new(),
            })
        );
    }

    #[dialog_common::test]
    async fn pre_commit_final_save_failure_preserves_the_pending_generation() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("callback-generation");
        let guard = stage_activation(&profile, &operator, &store, account.clone())
            .await
            .unwrap();
        let tmp = state_path(&profile, &store)
            .unwrap()
            .with_extension("json.tmp");
        std::fs::create_dir(&tmp).unwrap();

        let error = finalize_activation(&profile, &operator, guard, &account)
            .await
            .expect_err("an obstructed atomic replacement must fail");
        assert!(
            error
                .to_string()
                .contains("failed to create account-session temp file"),
            "unexpected save failure: {error:#}"
        );
        assert_eq!(
            load_raw(&profile, &operator, &store).await.unwrap(),
            Some(AccountSessionState {
                version: VERSION,
                active: None,
                pending_login: Some(PendingLogin::Activating { account }),
                pending_detaches: Vec::new(),
            })
        );
    }

    #[dialog_common::test]
    async fn stage_activation_resumes_the_identical_generation_after_restart() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("callback-generation");
        let first = stage_activation(&profile, &operator, &store, account.clone())
            .await
            .unwrap();
        drop(first);
        let before = load_raw(&profile, &operator, &store).await.unwrap();

        let resumed = stage_activation(&profile, &operator, &store, account)
            .await
            .unwrap();

        assert_eq!(load_raw(&profile, &operator, &store).await.unwrap(), before);
        drop(resumed);
    }

    #[dialog_common::test]
    async fn logout_clears_a_durable_waiting_handoff() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let state = AccountSessionState {
            pending_login: Some(PendingLogin::Waiting {
                provider: "https://accounts.example".to_string(),
                secret: "one-time-secret".to_string(),
                token_hash: "token-hash".to_string(),
            }),
            ..Default::default()
        };
        save_raw(&profile, &operator, &store, &state).await.unwrap();
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::Pending);

        let transition = logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        assert_eq!(
            transition,
            LogoutTransition {
                signed_out: true,
                queued: 0,
            }
        );
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::SignedOut);
        assert_eq!(
            load_raw(&profile, &operator, &store)
                .await
                .unwrap()
                .unwrap()
                .pending_login,
            None
        );
    }

    #[dialog_common::test]
    async fn logout_detaches_a_durable_activating_generation_exactly_once() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let pending_account = active_account("pending-generation");
        let state = AccountSessionState {
            pending_login: Some(PendingLogin::Activating {
                account: pending_account.clone(),
            }),
            ..Default::default()
        };
        save_raw(&profile, &operator, &store, &state).await.unwrap();

        assert_eq!(
            logout_transition_for_store(&profile, &operator, &store)
                .await
                .unwrap(),
            LogoutTransition {
                signed_out: true,
                queued: 1,
            }
        );
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::SignedOut);
        assert_eq!(
            logout_transition_for_store(&profile, &operator, &store)
                .await
                .unwrap(),
            LogoutTransition::default()
        );
    }

    #[dialog_common::test]
    async fn offline_logout_persists_a_signed_detach_before_clearing_active_state() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("offline-generation");
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(account.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        let bytes = std::fs::read(state_path(&profile, &store).unwrap()).unwrap();
        let persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(persisted["version"], 2);
        assert!(persisted["active"].is_null());
        let queued = persisted["pending_detaches"].as_array().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0]["provider"], account.provider);
        let intent: SignedDetachIntent =
            serde_json::from_value(queued[0]["signed_intent"].clone()).unwrap();
        let payload = intent.validate().await.unwrap();
        assert_eq!(payload.attachment_id, account.attachment_id);
        assert_eq!(payload.delegation_cid, account.delegation_cid);

        // Reloading from disk represents a fresh process; the queued cleanup
        // remains while the local session is already signed out.
        let restarted = load_raw(&profile, &operator, &store)
            .await
            .unwrap()
            .unwrap();
        assert!(restarted.active.is_none());
        assert_eq!(
            serde_json::to_value(restarted).unwrap()["pending_detaches"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    async fn detach_receipt_server(outcome: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Json, Router, routing::post};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/devices/detach",
            post(move || async move { Json(serde_json::json!({ "outcome": outcome })) }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    #[dialog_common::test]
    async fn terminal_provider_receipt_drains_the_restarted_outbox() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let (provider, server) = detach_receipt_server("alreadyDetached").await;
        let mut account = active_account("restart-generation");
        account.provider = provider;
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(account),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        let summary = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(summary.retired, 1);
        assert_eq!(summary.retryable, 0);
        assert!(
            load_raw(&profile, &operator, &store)
                .await
                .unwrap()
                .unwrap()
                .pending_detaches
                .is_empty()
        );
        server.abort();
    }

    #[dialog_common::test]
    async fn browser_handoff_defers_cleanup_until_callback_activation_can_settle() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let (provider, server) = detach_receipt_server("detached").await;
        let mut account = active_account("handoff-generation");
        account.provider = provider;
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(account),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        // The browser can now re-adopt this provider generation before its
        // callback makes that fact visible in local session state. The handoff
        // guard prevents an unrelated status process from detaching it in that
        // gap.
        let handoff = begin_handoff(&store).await.unwrap();
        let deferred = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(deferred.retryable, 1);
        assert_eq!(deferred.retired, 0);
        assert_eq!(
            load_raw(&profile, &operator, &store)
                .await
                .unwrap()
                .unwrap()
                .pending_detaches
                .len(),
            1
        );

        drop(handoff);
        let settled = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(settled.retired, 1);
        server.abort();
    }

    #[dialog_common::test]
    async fn unavailable_provider_keeps_cleanup_for_a_later_process() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let mut account = active_account("retry-generation");
        account.provider = "http://127.0.0.1:9".into();
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(account),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        let summary = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(summary.retryable, 1);
        assert_eq!(
            load_raw(&profile, &operator, &store)
                .await
                .unwrap()
                .unwrap()
                .pending_detaches
                .len(),
            1
        );
    }

    #[dialog_common::test]
    async fn malformed_provider_keeps_cleanup_with_a_permanent_diagnostic() {
        let (_temp, store, profile, operator) = isolated_session().await;
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(active_account("malformed-provider-generation")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();
        let mut state = load_raw(&profile, &operator, &store)
            .await
            .unwrap()
            .unwrap();
        state.pending_detaches[0].provider = "not a provider URL".into();
        save_raw(&profile, &operator, &store, &state).await.unwrap();

        let summary = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(summary.retryable, 0);
        assert_eq!(summary.permanently_malformed, 1);
        assert_eq!(
            load_raw(&profile, &operator, &store)
                .await
                .unwrap()
                .unwrap()
                .pending_detaches
                .len(),
            1
        );
    }

    #[dialog_common::test]
    async fn stale_cleanup_cannot_detach_a_generation_recovered_by_new_login() {
        let (_temp, store, profile, operator) = isolated_session().await;
        let account = active_account("recovered-generation");
        save_raw(
            &profile,
            &operator,
            &store,
            &AccountSessionState {
                active: Some(account.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();
        let mut state = load_raw(&profile, &operator, &store)
            .await
            .unwrap()
            .unwrap();
        state.active = Some(account);
        save_raw(&profile, &operator, &store, &state).await.unwrap();

        let summary = flush_pending_detaches(&profile, &operator, &store)
            .await
            .unwrap();
        assert_eq!(summary.retryable, 1);
        let state = load_raw(&profile, &operator, &store)
            .await
            .unwrap()
            .unwrap();
        assert!(state.active.is_some());
        assert_eq!(state.pending_detaches.len(), 1);
    }
}
