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
    /// Crash-recoverable activation or a legacy browser handoff.
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
        let state: AccountSessionState =
            serde_json::from_slice(&bytes).context("stored account-session state is malformed")?;
        validate_state(&state)?;
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
    pub remote: Option<String>,
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
    let state: AccountSessionState =
        serde_json::from_slice(&bytes).context("stored account-session state is malformed")?;
    validate_state(&state)?;
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
    let _root_did: Did = root
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
    let Some(provider) = tonk_account::AccountProviderRecord::decode(&bytes)
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
        remote: provider.remote().map(ToOwned::to_owned),
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
) -> Result<Vec<ActiveAccount>> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let existed = state.active.is_some() || state.pending_login.is_some();
    let mut detached = Vec::new();
    if let Some(active) = state.active.take() {
        detached.push(active);
    }
    if let Some(PendingLogin::Activating { account, .. }) = state.pending_login.take() {
        detached.push(account);
    }
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
            remote: Some("https://accounts.example/ucan/".to_string()),
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

        let detached = logout_transition_for_store(&profile, &operator, &store)
            .await
            .unwrap();

        assert!(detached.is_empty());
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
            vec![pending_account]
        );
        assert_eq!(inspect_local(&store).unwrap(), LocalPhase::SignedOut);
        assert!(
            logout_transition_for_store(&profile, &operator, &store)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
