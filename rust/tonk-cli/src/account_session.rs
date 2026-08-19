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
        .truncate(false)
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
    store: &SpotStore,
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

/// Read the sole active attachment under an existing shared guard.
pub async fn active_guarded(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    guard: &AccountSessionReadGuard,
) -> Result<Option<ActiveAccount>> {
    Ok(load_guarded(profile, operator, guard).await?.active)
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
    pub delivered: usize,
    /// Number dropped because the provider can never accept them.
    pub abandoned: usize,
    /// Providers whose detach is still queued after this attempt.
    pub retained: Vec<String>,
    /// Bounded warning for the first entry that was not delivered.
    pub warning: Option<String>,
}

impl FlushOutcome {
    /// Whether a detach for `provider` is still undelivered.
    pub fn retains(&self, provider: &str) -> bool {
        let provider = provider.trim_end_matches('/');
        self.retained
            .iter()
            .any(|queued| queued.trim_end_matches('/') == provider)
    }
}

/// What one delivery attempt proved about a queued detach.
enum Disposition {
    /// The provider confirmed a terminal lifecycle outcome.
    Delivered,
    /// The provider can never accept this intent, so keeping it only
    /// blocks the device. The account row may stay visible until the
    /// account page revokes it.
    Abandoned(String),
    /// The attempt was inconclusive: the generation may still be active.
    Retained(String),
}

/// Classify one provider response.
///
/// A detach intent is immutable and signed once, so a `4xx` is a permanent
/// verdict on this exact payload rather than a transient condition: the
/// service reports an unknown attachment as `404`, a payload disagreeing
/// with the stored row as `409`, and a malformed or forged intent as
/// `400`/`403`. Retrying any of those forever cannot change the answer, and
/// a queue that never drains blocks `tonk account link` permanently. The
/// timeout and rate-limit statuses are the exceptions: they describe the
/// attempt, not the payload.
fn classify(status: reqwest::StatusCode) -> Option<Disposition> {
    if status.is_client_error()
        && !matches!(
            status,
            reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::TOO_MANY_REQUESTS
        )
    {
        return Some(Disposition::Abandoned(format!(
            "gave up delivering a device detach after provider status {status}; \
             the device may still be listed on the account page"
        )));
    }
    if !status.is_success() {
        return Some(Disposition::Retained(format!(
            "detach retry retained after provider status {status}"
        )));
    }
    None
}

/// Whether a success response names a terminal lifecycle outcome.
async fn terminal_outcome(response: reqwest::Response) -> bool {
    response
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
        })
}

/// Drop every undelivered detach intent under the exclusive transition
/// lock, returning how many were dropped.
///
/// The escape hatch for a provider whose failures never resolve. Delivery
/// is best effort already, and an outbox that cannot drain otherwise blocks
/// every later login on this device with no way out.
pub async fn abandon_pending(profile: &Profile, operator: &Operator<NativeSpace>) -> Result<usize> {
    let store = SpotStore::open().context("failed to locate account state")?;
    abandon_pending_for_store(profile, operator, &store).await
}

pub(crate) async fn abandon_pending_for_store(
    profile: &Profile,
    operator: &Operator<NativeSpace>,
    store: &SpotStore,
) -> Result<usize> {
    let guard = exclusive_transition_guard(store)?;
    ensure_initialized(profile, operator, &guard).await?;
    let mut state = load_raw(profile, operator, store)
        .await?
        .unwrap_or_default();
    let abandoned = state.pending_detaches.len();
    if abandoned > 0 {
        state.pending_detaches.clear();
        save_raw(profile, operator, store, &state).await?;
    }
    Ok(abandoned)
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
    let mut state = load_raw(profile, operator, store)
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
        let disposition = match result {
            Ok(response) => {
                if let Some(disposition) = classify(response.status()) {
                    disposition
                } else if terminal_outcome(response).await {
                    // A success status still has to name a terminal lifecycle
                    // outcome; anything else leaves this generation's fate
                    // unknown, which is a retry rather than a delivery.
                    Disposition::Delivered
                } else {
                    Disposition::Retained(
                        "detach retry retained after an unrecognized provider outcome".to_string(),
                    )
                }
            }
            Err(error) => Disposition::Retained(format!("detach retry retained: {error}")),
        };
        match disposition {
            Disposition::Delivered => outcome.delivered += 1,
            Disposition::Abandoned(warning) => {
                outcome.abandoned += 1;
                outcome.warning.get_or_insert(warning);
            }
            Disposition::Retained(warning) => {
                outcome.warning.get_or_insert(warning);
                outcome.retained.push(pending.provider.clone());
                retained.push(pending);
            }
        }
    }
    state.pending_detaches = retained;
    save_raw(profile, operator, store, &state).await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use dialog_operator::DeriveOperator as _;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    use dialog_capability::Subject;
    use dialog_effects::storage::Directory;
    use dialog_storage::provider::storage::Storage;

    use super::*;

    /// Answer every detach request with one fixed status and body.
    fn detach_server(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind detach service");
        let endpoint = format!("http://{}", listener.local_addr().expect("service address"));
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut stream) = incoming else { return };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
            }
        });
        endpoint
    }

    /// A port nothing is listening on, so delivery cannot even connect.
    fn unreachable_provider() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a port");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("reserved address")
        );
        drop(listener);
        endpoint
    }

    struct Fixture {
        profile: Profile,
        operator: Operator<NativeSpace>,
        store: SpotStore,
        _directory: tempfile::TempDir,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = SpotStore::at(directory.path().join("state"));
        let storage = Storage::<NativeSpace>::default();
        let profile = Profile::open(format!("cli-detach-outbox-{}", rand::random::<u64>()))
            .at(Directory::At(
                directory.path().join("profiles").to_string_lossy().into(),
            ))
            .perform(&storage)
            .await
            .expect("open profile");
        std::fs::create_dir_all(store.account_dir()).expect("create account directory");
        let account_dir = store.account_dir().canonicalize().expect("account path");
        let operator = profile
            .derive(b"tonk/account-state/v1")
            .allow(Subject::any())
            .base(Directory::At(account_dir.to_string_lossy().into()))
            .build(storage)
            .await
            .expect("derive operator");
        Fixture {
            profile,
            operator,
            store,
            _directory: directory,
        }
    }

    /// Queue one detach intent addressed to `provider`.
    async fn queue(fixture: &Fixture, provider: &str) {
        let intent = SignedDetachIntent::sign(
            fixture.profile.signer(),
            &fixture.profile.did(),
            "attachment",
            "delegation",
            1,
        )
        .await
        .expect("sign detach intent");
        let state = AccountSessionState {
            pending_detaches: vec![PendingDetach {
                provider: provider.to_string(),
                intent,
            }],
            ..AccountSessionState::default()
        };
        save_raw(&fixture.profile, &fixture.operator, &fixture.store, &state)
            .await
            .expect("seed the outbox");
    }

    async fn outbox(fixture: &Fixture) -> Vec<PendingDetach> {
        load_raw(&fixture.profile, &fixture.operator, &fixture.store)
            .await
            .expect("load state")
            .unwrap_or_default()
            .pending_detaches
    }

    async fn flush(fixture: &Fixture) -> FlushOutcome {
        flush_pending_for_store(&fixture.profile, &fixture.operator, &fixture.store)
            .await
            .expect("flush the outbox")
    }

    #[dialog_common::test]
    async fn it_removes_a_detach_the_provider_terminally_confirmed() {
        let fixture = fixture().await;
        let provider = detach_server("200 OK", r#"{"outcome":"alreadyDetached"}"#);
        queue(&fixture, &provider).await;

        let outcome = flush(&fixture).await;

        assert_eq!(outcome.delivered, 1);
        assert_eq!(outcome.abandoned, 0);
        assert!(outcome.retained.is_empty());
        assert_eq!(outcome.warning, None);
        assert!(outbox(&fixture).await.is_empty());
    }

    /// A `2xx` whose body names no lifecycle outcome leaves the generation's
    /// fate unknown, so the intent has to survive for a later attempt.
    #[dialog_common::test]
    async fn it_retains_a_detach_after_an_unrecognized_success_body() {
        let fixture = fixture().await;
        let provider = detach_server("200 OK", r#"{"outcome":"somethingElse"}"#);
        queue(&fixture, &provider).await;

        let outcome = flush(&fixture).await;

        assert_eq!(outcome.delivered, 0);
        assert!(outcome.retains(&provider));
        assert_eq!(outbox(&fixture).await.len(), 1);
    }

    /// The lockout this prevents: an account service with no detach route
    /// (or no such attachment) answers `404` forever, and a retained entry
    /// blocks every later `tonk account link` on the device.
    #[dialog_common::test]
    async fn it_abandons_a_detach_the_provider_can_never_accept() {
        let fixture = fixture().await;
        let provider = detach_server("404 Not Found", "{}");
        queue(&fixture, &provider).await;

        let outcome = flush(&fixture).await;

        assert_eq!(outcome.abandoned, 1);
        assert_eq!(outcome.delivered, 0);
        assert!(!outcome.retains(&provider));
        assert!(
            outcome
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("may still be listed")),
            "an abandoned detach must say the device can remain listed: {:?}",
            outcome.warning
        );
        assert!(outbox(&fixture).await.is_empty());
    }

    #[dialog_common::test]
    async fn it_abandons_a_detach_whose_payload_the_provider_rejects() {
        let fixture = fixture().await;
        let provider = detach_server("409 Conflict", "{}");
        queue(&fixture, &provider).await;

        assert_eq!(flush(&fixture).await.abandoned, 1);
        assert!(outbox(&fixture).await.is_empty());
    }

    /// Retryable failures describe the attempt rather than the signed
    /// payload, so these must outlive the attempt even though two of them
    /// are client-error statuses.
    #[dialog_common::test]
    async fn it_retains_a_detach_after_a_retryable_failure() {
        for status in [
            "503 Service Unavailable",
            "429 Too Many Requests",
            "408 Request Timeout",
        ] {
            let fixture = fixture().await;
            let provider = detach_server(status, "{}");
            queue(&fixture, &provider).await;

            let outcome = flush(&fixture).await;

            assert_eq!(outcome.abandoned, 0, "{status} must not be abandoned");
            assert!(outcome.retains(&provider), "{status} must be retried");
            assert_eq!(outbox(&fixture).await.len(), 1, "{status} must persist");
        }
    }

    #[dialog_common::test]
    async fn it_retains_a_detach_the_provider_never_answered() {
        let fixture = fixture().await;
        let provider = unreachable_provider();
        queue(&fixture, &provider).await;

        let outcome = flush(&fixture).await;

        assert!(outcome.retains(&provider));
        assert_eq!(outbox(&fixture).await.len(), 1);
    }

    #[dialog_common::test]
    async fn it_abandons_every_queued_detach_on_demand() {
        let fixture = fixture().await;
        queue(&fixture, &unreachable_provider()).await;

        let abandoned =
            abandon_pending_for_store(&fixture.profile, &fixture.operator, &fixture.store)
                .await
                .expect("abandon the outbox");

        assert_eq!(abandoned, 1);
        assert!(outbox(&fixture).await.is_empty());
    }

    /// A detach queued for one provider says nothing about linking to
    /// another, so it must not report itself as blocking that handoff.
    #[dialog_common::test]
    async fn it_reports_a_retained_detach_only_for_its_own_provider() {
        let fixture = fixture().await;
        let provider = detach_server("503 Service Unavailable", "{}");
        queue(&fixture, &provider).await;

        let outcome = flush(&fixture).await;

        assert!(outcome.retains(&format!("{provider}/")));
        assert!(!outcome.retains("https://accounts.example"));
    }
}
