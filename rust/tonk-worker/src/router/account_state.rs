//! Account state on profile main: upstream configuration and trusted
//! hydration.
//!
//! The account is the upstream remote of the profile repository's main
//! branch — no separate repository exists. Configuration is
//! deliberately separate from readiness: the upstream may be set while
//! the remote is unavailable, and no account-state mutation API runs
//! until the trusted-base marker matches the signed descriptor.

use std::collections::HashSet;
use std::sync::Mutex;

use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{RemoteAddress, RemoteRepository, Repository, SiteAddress, Upstream};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Principal;
use tonk_account::{
    AccountRepositoryDescriptorV1, AccountStateStatus, CreateGenesis, RemotePresence,
    probe_remote_main, publish_genesis_if_absent,
};
use tonk_common::log;
use tonk_schema::{AccountPasskeyCreated, Replica, prelude::DidExt as _};

use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Remote name for the account's access branch in the profile repository.
const ACCOUNT_ACCESS_REMOTE: &str = "account-access";

/// Identity returned only after the trusted-base gate has passed.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct ReadyAccountBranch {
    pub(crate) key: String,
    pub(crate) subject: dialog_varsig::Did,
}

/// Read the trusted marker without mounting or contacting the remote.
async fn trusted_marker(tonk: &TonkState) -> Result<Option<Vec<u8>>, TonkWorkerError> {
    match tonk
        .profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .load::<Vec<u8>>()
        .perform(&tonk.operator)
        .await
    {
        Ok(marker) => Ok(Some(marker)),
        Err(error) if crate::credential::is_missing(&error) => Ok(None),
        Err(error) => Err(TonkWorkerError::Internal(format!(
            "failed to load account trusted-base marker: {error}"
        ))),
    }
}

async fn mark_trusted(
    tonk: &TonkState,
    descriptor: &AccountRepositoryDescriptorV1,
) -> Result<(), TonkWorkerError> {
    tonk.profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .save(descriptor.content_hash().to_vec())
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to save account trusted-base marker: {error}"
            ))
        })
}

fn marker_matches(marker: Option<&[u8]>, descriptor: &AccountRepositoryDescriptorV1) -> bool {
    marker == Some(descriptor.content_hash().as_slice())
}

/// The descriptor this profile is configured with, absent when the local link
/// is missing, unreadable, or still a legacy raw delegation.
async fn configured_descriptor(tonk: &TonkState) -> Option<AccountRepositoryDescriptorV1> {
    super::account::descriptor(tonk).await
}

/// Current durable account-state status, without a network request.
pub(crate) async fn status(tonk: &TonkState) -> AccountStateStatus {
    let Some(descriptor) = configured_descriptor(tonk).await else {
        return AccountStateStatus::Unconfigured;
    };
    match trusted_marker(tonk).await {
        Ok(marker) if marker_matches(marker.as_deref(), &descriptor) => AccountStateStatus::Ready,
        Ok(_) => AccountStateStatus::Unhydrated,
        Err(error) => {
            log!("account trusted-base marker unreadable: {error}");
            AccountStateStatus::Unhydrated
        }
    }
}

/// The account routing keys, resolved once. Nothing is hidden behind
/// them any more — no repository exists — but the sync layer still
/// routes the account sweep by its key, and resolving the descriptor
/// costs a credential load plus a signature verification, so the
/// answer is cached between the few writes that change it.
///
/// The inner `None` means "not resolved yet". An empty set is a real
/// answer: this profile has no linked account.
#[derive(Default)]
pub struct AccountKeys(Mutex<Option<HashSet<String>>>);

impl AccountKeys {
    /// Forget the resolved answer, so the next lookup rebuilds it.
    ///
    /// Call this from anything that writes the account-link credential or
    /// indexes an account replica.
    pub(crate) fn invalidate(&self) {
        if let Ok(mut cached) = self.0.lock() {
            *cached = None;
        }
    }

    fn get(&self) -> Option<HashSet<String>> {
        self.0.lock().ok().and_then(|cached| cached.clone())
    }

    fn set(&self, keys: HashSet<String>) {
        if let Ok(mut cached) = self.0.lock() {
            *cached = Some(keys);
        }
    }
}

/// Whether `key` names this profile's account: the sync layer routes
/// the account sweep by its key, and the join path refuses to mount
/// the account subject as a user space.
pub(crate) async fn is_account_key(tonk: &TonkState, key: &str) -> bool {
    if let Some(keys) = tonk.account_keys.get() {
        return keys.contains(key);
    }
    let keys = resolve_account_keys(tonk).await;
    let matched = keys.contains(key);
    tonk.account_keys.set(keys);
    matched
}

/// Every routing key that names this profile's account: the configured
/// descriptor's subject, plus the local root itself — the account
/// subject is the root, so its key is derivable without an attachment.
async fn resolve_account_keys(tonk: &TonkState) -> HashSet<String> {
    let mut keys = HashSet::new();
    if let Some(descriptor) = configured_descriptor(tonk).await {
        keys.insert(descriptor.account_subject().repo_key().to_owned());
    }
    if let Ok(root) = super::identity::local_root(tonk).await {
        keys.insert(root.root_did.repo_key().to_owned());
    }
    keys
}

/// The account replica rows the profile repository indexes.
async fn account_replicas(tonk: &TonkState) -> Result<Vec<Replica>, TonkWorkerError> {
    let meta = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open the profile index: {error}"))
        })?;
    meta.handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::from(Replica::account_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to query account replicas: {error}"))
        })
}

/// The linked account's subject, read from the account replica the
/// profile repository indexes — the same remote registration sync draws
/// its pull population from (plan/Account model.md §5). Mounting the
/// account repository records it; unlink retracts it. There is no
/// separate linked flag: an attachment record without this replica is a
/// link that never completed, not a linked account.
///
/// `Err` is a transient index read failure, distinct from a readable
/// "no replica"; callers with a stored attachment may fall back to it
/// rather than signing the profile out on a flaky read.
pub(crate) async fn linked_account(
    tonk: &TonkState,
) -> Result<Option<dialog_varsig::Did>, TonkWorkerError> {
    Ok(account_replicas(tonk)
        .await?
        .into_iter()
        .find_map(|row| row.subject.0.to_string().parse::<dialog_varsig::Did>().ok()))
}

/// Retract every account replica row from the profile index: the unlink
/// half of the linked-state signal. The mounted repository and its
/// remote configuration stay on disk — dialog remotes are create-only —
/// but nothing tracks them any more, so neither sync nor the linked
/// signal sees them, and re-linking re-records the same replica.
pub(crate) async fn retract_account_replicas(tonk: &TonkState) -> Result<(), TonkWorkerError> {
    let rows = account_replicas(tonk).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction();
    for row in rows {
        transaction = transaction.retract(row);
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to retract account replicas: {error}"))
        })?;
    tonk.account_keys.invalidate();
    Ok(())
}

/// Republish a stored remote's address cell so it matches the current
/// descriptor, returning the repointed remote.
///
/// A remote's address is a memory cell, not an immutable record: when
/// the link's provider address changes or the profile links to a
/// different account, the stored cell goes stale and every
/// strict-equality check after it would refuse to mount forever.
async fn repoint_remote<C: Principal>(
    repository: &Repository<C>,
    name: &str,
    address: &SiteAddress,
    subject: &dialog_varsig::Did,
    tonk: &TonkState,
) -> Result<RemoteRepository, TonkWorkerError> {
    let reference = repository.remote(name);
    let target = RemoteAddress::new(address.clone(), subject.clone());
    let cell = reference.address();
    // Resolve first: publish is a compare-and-swap against the cell's
    // current version, and a fresh handle has not seen one yet.
    cell.resolve()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to resolve the '{name}' remote: {error}"))
        })?;
    cell.publish(target.clone())
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to repoint the '{name}' remote: {error}"))
        })?;
    Ok(RemoteRepository::new(cell.retain(target), reference))
}

/// Point profile main at the account: the account is the upstream
/// remote of the profile repository's main branch (`Account model.md`
/// §5 in its literal form), not a separate repository. The subject-
/// different remote resolves against the account's DID — the same
/// shape [`adopt_account_access`] gives the access branch.
///
/// The returned key is the account's *routing* key: it still names the
/// account in the sync drain and dirty-marking, but no repository —
/// and no database — exists behind it any more.
async fn configure_account_upstream(
    tonk: &TonkState,
    descriptor: &AccountRepositoryDescriptorV1,
) -> Result<String, TonkWorkerError> {
    let subject = descriptor.account_subject().clone();
    let key = subject.repo_key().to_owned();
    let repository = Repository::from(&tonk.profile);

    let address = SiteAddress::from(UcanAddress::new(descriptor.remote().as_str()));
    let remote = match repository
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(remote) if remote.address().site() == &address && remote.did() == subject => remote,
        // A stored remote that disagrees with the descriptor follows an
        // older link — a previous provider address or an account this
        // profile has since left. The descriptor is the current link,
        // so repoint the address cell to it rather than refusing to
        // mount forever.
        Ok(_) => {
            repoint_remote(
                &repository,
                tonk_account::ORIGIN_REMOTE,
                &address,
                &subject,
                tonk,
            )
            .await?
        }
        Err(_) => repository
            .remote(tonk_account::ORIGIN_REMOTE)
            .create(address.clone())
            .subject(subject.clone())
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "failed to configure the account remote: {error}"
                ))
            })?,
    };

    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open profile main branch: {error}"))
        })?;
    let remote_branch = remote
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open account remote branch: {error}"))
        })?;

    match branch.upstream() {
        Some(Upstream::Remote { remote, branch, .. })
            if remote == tonk_account::ORIGIN_REMOTE && branch == tonk_account::MAIN_BRANCH => {}
        // A pointer left by an earlier account scheme (or an older link)
        // is repointed, like the remote cell above: with a linked
        // account, the account IS profile main's upstream by
        // definition, and set_upstream promotes over an existing
        // tracking target.
        _ => branch
            .set_upstream(&remote_branch)
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to set profile main upstream: {error}"))
            })?,
    }

    record_account_replica(tonk, &subject, &address).await?;
    Ok(key)
}

/// Index the account replica on profile main. The row keeps the sync
/// drain's routing contract — the account key still schedules the
/// account sweep — and the linked-state signal reads it.
async fn record_account_replica(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
    address: &SiteAddress,
) -> Result<(), TonkWorkerError> {
    let replica = Replica::account(tonk.profile.did(), subject.clone());
    let remote = replica.remote(tonk_account::ORIGIN_REMOTE, subject.clone(), address);
    let tracked = remote.branch(tonk_account::MAIN_BRANCH);

    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(replica.clone())
        .assert(replica.branch(tonk_account::MAIN_BRANCH))
        .assert(remote)
        .assert(tracked.clone())
        .assert(
            replica
                .branch(tonk_account::MAIN_BRANCH)
                .set_upstream(&tracked),
        )
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to index the account replica: {error}"))
        })?;
    // The index the routing-key fallback reads just gained a row; a resolve
    // that ran during an earlier transient read failure cached an empty set,
    // and only this clears it.
    tonk.account_keys.invalidate();
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

async fn hydrate_untrusted(tonk: &TonkState) -> Result<(), TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let remote = Repository::from(&tonk.profile)
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;

    match probe_remote_main(&remote, &tonk.operator).await {
        Ok(RemotePresence::Present(_)) => {
            session
                .handle()
                .pull()
                .download()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to hydrate the account into profile main: {error}"
                    ))
                })?;
        }
        Ok(RemotePresence::Absent) => {
            tonk.reactor
                .profile_repository()
                .branch(tonk_account::MAIN_BRANCH)
                .transaction()
                .commit()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to commit the account genesis base: {error}"
                    ))
                })?;
            match publish_genesis_if_absent(session.handle(), &remote, &tonk.operator).await {
                Ok(CreateGenesis::Winner(_)) => {}
                // Adopt the winner by pulling: the pull integrates the
                // established head AND records it as this branch's sync
                // base, so the next push fast-forwards instead of CASing
                // against an empty upstream. Reads resolve missing blocks
                // through the configured remote. Covered by
                // `it_adopts_a_losing_candidate_onto_the_winners_content`
                // in tonk-access-service's `account_remote` tests.
                Ok(CreateGenesis::Loser(_)) => {
                    session
                        .handle()
                        .pull()
                        .download()
                        .perform(&tonk.operator)
                        .await
                        .map(|_| ())
                        .map_err(|error| {
                            TonkWorkerError::Internal(format!(
                                "failed to adopt winning account genesis: {error}"
                            ))
                        })?;
                }
                Err(error) => return Err(TonkWorkerError::Internal(error.to_string())),
            }
        }
        Err(error) => return Err(TonkWorkerError::Internal(error.to_string())),
    }
    Ok(())
}

/// Reconcile profile main against the account upstream: pull, project,
/// push.
///
/// This is the account's whole sweep. It is deliberately not the
/// generic per-branch [`sync`](crate::router::sync) route: the account
/// has no pause preference to honor and no status chip to stamp, and
/// routing it through both would pull and push it twice per heartbeat.
///
/// `Err` names the step that did not land, so the caller can report a sweep
/// worth retrying. Convergence failing is not one of those: it is per-space and
/// keeps its own retry list, so it is logged and the sweep still counts.
/// Push profile main to the account remote.
///
/// Split out because both sweep arms need it: the ready arm as the tail
/// of `sync_ready`, and the hydrate arm as the step that publishes what
/// hydration just established. Only these two paths push profile main —
/// the generic per-branch sync returns before it reaches the account
/// key — so a sweep that skips this leaves local facts unpublished.
pub(crate) async fn push_account_main(tonk: &TonkState) -> Result<(), String> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| format!("account branch unavailable: {error}"))?;
    session
        .handle()
        .push()
        .perform(&tonk.operator)
        .await
        .map_err(|error| format!("account push failed: {error}"))?;
    Ok(())
}

async fn sync_ready(tonk: &TonkState, _key: &str) -> Result<(), String> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| format!("account branch unavailable: {error}"))?;
    // Pull-and-materialize: profile main is also the access branch, and
    // the authorization walk over it must read entirely locally at the
    // next session open (see `adopt_account_upstream`). Downloading here,
    // while this session can still authorize remote reads, is what keeps
    // a bare adoption from bricking the next boot.
    session
        .handle()
        .pull()
        .download()
        .perform(&tonk.operator)
        .await
        .map_err(|error| format!("account pull failed: {error}"))?;
    // The account's own sync is not enough on its own: the operator proves
    // from the PROFILE's access branch, so authority that arrived in the
    // account above is present but unusable until the access branch adopts
    // it. Runs on every sweep, and is a no-op once the upstream is set.
    adopt_account_access(tonk).await;
    // After the pull, so the seed sees what other devices already recorded,
    // and before the push, so anything it writes leaves with this sweep.
    if seed_passkey_facts(tonk).await {
        log!("recorded this device's passkey creation facts in the account space");
    }
    describe_own_device(tonk).await;
    if let Err(error) = converge_account_state(tonk).await {
        log!("account-state convergence after sync failed: {error}");
    }
    // Pushed through the session this function already holds, rather
    // than `push_account_main`, which acquires one of its own for the
    // hydrate arm.
    session
        .handle()
        .push()
        .perform(&tonk.operator)
        .await
        .map_err(|error| format!("account push failed: {error}"))?;
    Ok(())
}

/// Mount and, when necessary, hydrate the configured account repository.
///
/// All remote failures leave the account unhydrated. Only exact remote
/// adoption or a create-if-absent winner writes the trusted marker.
///
/// An already-ready branch is reconciled on the way through, so this doubles as
/// the account repository's catch-up on boot and before an authoritative write.
/// That reconcile is best-effort here; [`ensure_account_state_swept`] is the
/// same work with its outcome reported.
pub(crate) async fn ensure_account_state(tonk: &TonkState) -> AccountStateStatus {
    let (status, swept) = ensure_account_state_swept(tonk).await;
    if let Err(error) = swept {
        log!("account repository is ready but did not reconcile: {error}");
    }
    status
}

/// [`ensure_account_state`], plus whether an already-ready branch reconciled.
///
/// The background sweep needs the second half: the account repository is swept
/// here and nowhere else, so a failed pull or push has to reach the caller to
/// become a retryable `sync` instead of being logged and forgotten. `Ok(())`
/// on any path that ran no reconcile — the status carries that story instead.
pub(crate) async fn ensure_account_state_swept(
    tonk: &TonkState,
) -> (AccountStateStatus, Result<(), String>) {
    // One ensure at a time. The drain heartbeat, the link path, and the
    // save path can all arrive here concurrently, and their futures
    // interleave at await points; dialog's commit takes no per-branch
    // lock, so two interleaved ensures committing profile main or the
    // access branch can tear an artifact — a branch head referencing a
    // blob that never landed, which wedges the worker at the next boot.
    static ENSURE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serialized = ENSURE.lock().await;

    let Some(descriptor) = configured_descriptor(tonk).await else {
        return (AccountStateStatus::Unconfigured, Ok(()));
    };

    let key = match configure_account_upstream(tonk, &descriptor).await {
        Ok(key) => key,
        Err(error) => {
            log!("account upstream configuration failed: {error}");
            return (AccountStateStatus::Unhydrated, Ok(()));
        }
    };

    // An install that mounted the account as a hidden repository keeps
    // that database around until this runs. Once per worker life; the
    // deletion shim never rejects, and everything the repository held
    // is recoverable from the same remote profile main now follows.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static LEGACY_DISCARDED: AtomicBool = AtomicBool::new(false);
        if !LEGACY_DISCARDED.swap(true, Ordering::Relaxed) {
            super::repository::delete_legacy_storage(&key).await;
        }
    }

    match trusted_marker(tonk).await {
        Ok(marker) if marker_matches(marker.as_deref(), &descriptor) => {
            let swept = sync_ready(tonk, &key).await;
            (AccountStateStatus::Ready, swept)
        }
        Ok(_) => match hydrate_untrusted(tonk).await {
            Ok(()) => match mark_trusted(tonk, &descriptor).await {
                Ok(()) => {
                    // The path a freshly created account takes, where the
                    // ready sweep above has not run yet.
                    if seed_passkey_facts(tonk).await {
                        log!("recorded this device's passkey creation facts in the account space");
                    }
                    describe_own_device(tonk).await;
                    if let Err(error) = converge_account_state(tonk).await {
                        log!("account-state convergence after hydration failed: {error}");
                    }
                    // Push what this sweep just made durable. Without
                    // it, hydrating leaves the account Ready but never
                    // uploaded, so nothing local reaches the account
                    // remote until some *later* sweep happens to take
                    // the `marker_matches` arm above — which is the only
                    // other place profile main is pushed. A device that
                    // is not poked again simply never publishes: its
                    // spaces stay invisible to the account's other
                    // devices.
                    let pushed = push_account_main(tonk).await;
                    (AccountStateStatus::Ready, pushed)
                }
                Err(error) => {
                    log!("account repository hydrated but marker save failed: {error}");
                    (AccountStateStatus::Unhydrated, Ok(()))
                }
            },
            Err(error) => {
                log!("account repository remains unhydrated: {error}");
                (AccountStateStatus::Unhydrated, Ok(()))
            }
        },
        Err(error) => {
            log!("account trusted-base marker unreadable: {error}");
            (AccountStateStatus::Unhydrated, Ok(()))
        }
    }
}

/// Return the account identity only after re-checking the trusted marker.
#[allow(dead_code)]
pub(crate) async fn require_ready_account_state(
    tonk: &TonkState,
) -> Result<ReadyAccountBranch, TonkWorkerError> {
    let descriptor = configured_descriptor(tonk)
        .await
        .ok_or_else(|| TonkWorkerError::Conflict("account state is unconfigured".to_string()))?;
    let marker = trusted_marker(tonk).await?;
    if !marker_matches(marker.as_deref(), &descriptor) {
        return Err(TonkWorkerError::Conflict(
            "account state has no trusted remote base".to_string(),
        ));
    }
    let subject = descriptor.account_subject().clone();
    let key = subject.repo_key().to_owned();
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    Ok(ReadyAccountBranch { key, subject })
}

/// Read the creation facts recorded on a ready account branch.
async fn read_passkey_facts(
    tonk: &TonkState,
    ready: &ReadyAccountBranch,
) -> Result<Option<tonk_worker_api::PasskeyMetadata>, TonkWorkerError> {
    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open ready account state: {error}")))?;
    let rows: Vec<AccountPasskeyCreated> = branch
        .handle()
        .query()
        .select(Query::<AccountPasskeyCreated> {
            this: Term::from(ready.subject.this()),
            created_at: Term::var("created_at"),
            created_on: Term::var("created_on"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read account passkey facts: {error:?}"))
        })?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| tonk_worker_api::PasskeyMetadata {
            created_at: row.seconds(),
            created_on: row.created_on.0,
        }))
}

/// This account's passkey facts as recorded in the account space, absent when
/// the account is not ready or carries none.
///
/// Best-effort by design — the dashboard has an explicit unavailable state and
/// must not fail because a hidden system repository is mid-hydration. Every
/// `None` that is not simply "no fact" is logged, so an unreadable branch is
/// visible rather than silent.
pub(crate) async fn passkey_facts(tonk: &TonkState) -> Option<tonk_worker_api::PasskeyMetadata> {
    let ready = require_ready_account_state(tonk).await.ok()?;
    match read_passkey_facts(tonk, &ready).await {
        Ok(facts) => facts,
        Err(error) => {
            log!("account passkey facts unreadable: {error}");
            None
        }
    }
}

/// Point this profile's access branch at the account and pull it.
///
/// The account repository syncing with its own remote is not enough: the
/// operator resolves proofs from the PROFILE's access branch, so authority
/// living in the account is present but unusable until the access branch
/// adopts it. This is what makes a recovered delegation authorize anything.
///
/// Best-effort and non-fatal, like the rest of the sweep: a device that
/// cannot reach the account keeps whatever authority it already holds.
/// Returns whether it adopted, and logs every reason it did not.
pub(crate) async fn adopt_account_access(tonk: &TonkState) -> bool {
    let Ok(descriptor) = configured_descriptor(tonk).await.ok_or(()) else {
        return false;
    };
    let subject = descriptor.account_subject().clone();
    let repository = Repository::from(&tonk.profile);
    let access = match repository
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("open profile access branch to adopt the account: {error}");
            return false;
        }
    };

    // A remote resolved against the ACCOUNT's DID. A local upstream would
    // resolve against this profile's own subject and could only name a
    // sibling branch, never the account's.
    let address = SiteAddress::from(UcanAddress::new(descriptor.remote().as_str()));
    let remote = match repository
        .remote(ACCOUNT_ACCESS_REMOTE)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(remote) if remote.address().site() == &address && remote.did() == subject => remote,
        // Stale cell from an earlier link; see `repoint_remote`.
        Ok(_) => {
            match repoint_remote(&repository, ACCOUNT_ACCESS_REMOTE, &address, &subject, tonk).await
            {
                Ok(remote) => remote,
                Err(error) => {
                    log!("repoint the account access remote: {error}");
                    return false;
                }
            }
        }
        Err(_) => match repository
            .remote(ACCOUNT_ACCESS_REMOTE)
            .create(address)
            .subject(subject)
            .perform(&tonk.operator)
            .await
        {
            Ok(remote) => remote,
            Err(error) => {
                log!("configure the account access remote: {error}");
                return false;
            }
        },
    };
    let upstream = match remote
        .branch(dialog_repository::ACCESS_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("open the account access branch: {error}");
            return false;
        }
    };

    match tonk_account::delegations::adopt_account_upstream(&access, upstream, &tonk.operator).await
    {
        Ok(_) => true,
        Err(error) => {
            log!("adopt the account as the access upstream: {error}");
            false
        }
    }
}

/// Retain a `space → account-root` delegation into this device's account
/// space, resolving the branch and swallowing every failure.
///
/// The retain itself is [`tonk_account::delegations::retain_space_delegation`],
/// shared with the CLI so both adapters retain the same thing. What is local
/// to the worker is how the branch is reached (through the reactor, so the
/// write joins the sync queue) and the decision to treat failure as
/// non-fatal: a space is fully usable the moment its delegation reaches the
/// profile's own access branch, so failing space creation because a hidden
/// system repository was mid-hydration would trade a working space for a
/// recoverable one. Returns whether it retained, and logs every reason it did
/// not.
pub(crate) async fn retain_space_delegation(tonk: &TonkState, chain: &DelegationChain) -> bool {
    let ready = match require_ready_account_state(tonk).await {
        Ok(ready) => ready,
        // Unconfigured and unhydrated are ordinary states for a signed-out or
        // still-hydrating profile, not failures worth a line in the log.
        Err(_) => return false,
    };
    let branch = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(branch) => branch,
        Err(error) => {
            log!("open account branch to retain space delegation: {error}");
            return false;
        }
    };
    match tonk_account::delegations::retain_space_delegation(branch.handle(), chain, &tonk.operator)
        .await
    {
        Ok(wrote) => {
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            if wrote {
                tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
            }
            // The routing key only feeds the wasm dirty-marking above.
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            let _ = &ready;
            wrote
        }
        Err(error) => {
            log!("retain space delegation into account space: {error}");
            false
        }
    }
}

/// Describe this device's own link in the account space, best-effort.
///
/// Runs on every sweep like the passkey-fact seed: retaining is
/// content-addressed, so an already-described device commits nothing.
/// This is what puts the signing browser's own row where every device's
/// list reads — sign-up, passkey sign-in, and accounts that predate the
/// facts all converge through it.
async fn describe_own_device(tonk: &TonkState) {
    let Ok(root) = super::identity::local_root(tonk).await else {
        return;
    };
    let Ok(chain) = DelegationChain::try_from(root.bytes.as_slice()) else {
        return;
    };
    if let Err(error) =
        crate::onboarding::describe_device_link(tonk, &chain, crate::onboarding::device_title())
            .await
    {
        log!("describe this device's link: {error}");
    }
}

/// Write this device's recorded passkey facts into the account space when it
/// has them and the space does not. Returns whether it wrote.
///
/// Idempotent: a device that only ever *evaluated* an existing root has
/// nothing to contribute and returns `false` without touching the branch. So
/// does a device whose facts are already there — an existing fact is never
/// overwritten, so a device that later derives a different label cannot
/// rewrite history.
pub(crate) async fn seed_passkey_facts(tonk: &TonkState) -> bool {
    // Unconfigured and unhydrated are ordinary states, reached on every sweep
    // of a signed-out profile. They are not worth a line in the log.
    let Ok(ready) = require_ready_account_state(tonk).await else {
        return false;
    };
    match read_passkey_facts(tonk, &ready).await {
        Ok(None) => {}
        Ok(Some(_)) => return false,
        Err(error) => {
            log!("account passkey facts unreadable before seeding: {error}");
            return false;
        }
    }
    let Ok(root) = super::identity::local_root(tonk).await else {
        return false;
    };
    let Some(metadata) = root.passkey else {
        return false;
    };
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountPasskeyCreated::new(
            ready.subject.this(),
            metadata.created_at,
            metadata.created_on,
        ))
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("commit account passkey creation facts: {error}");
        return false;
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
    true
}

/// Project the authoritative account display name into local caches and every
/// known real-space roster.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn converge_account_state(
    tonk: &TonkState,
) -> Result<tonk_worker_api::AccountConvergenceReport, TonkWorkerError> {
    use tonk_schema::{AccountDisplayName, MemberName, Membership, ProfileName};
    use tonk_worker_api::AccountConvergenceReport;

    let ready = require_ready_account_state(tonk).await?;
    let account = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open ready account state: {error}")))?;
    let names: Vec<AccountDisplayName> = account
        .handle()
        .query()
        .select(Query::<AccountDisplayName> {
            this: Term::from(ready.subject.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read account display name: {error:?}"))
        })?;
    let Some(name) = names.into_iter().next().map(|name| name.name.0) else {
        return Ok(AccountConvergenceReport::default());
    };

    let profile_entity = tonk.profile.did().this();
    let profile = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open profile name cache: {error}")))?;
    let cached: Vec<ProfileName> = profile
        .handle()
        .query()
        .select(Query::<ProfileName> {
            this: Term::from(profile_entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("read profile name: {error:?}")))?;
    let profile_changed = cached.first().is_none_or(|cached| cached.name.0 != name);
    if profile_changed {
        tonk.reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .transaction()
            .assert(ProfileName::new(profile_entity, name.clone()))
            .commit()
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("project account name to profile: {error}"))
            })?;
    }

    let member = ready.subject;
    let device = tonk.profile.did();
    let mut report = AccountConvergenceReport {
        profile_changed,
        ..AccountConvergenceReport::default()
    };
    for key in crate::router::profile_name::real_space_keys(tonk).await {
        let result = async {
            let session = tonk
                .reactor
                .repository(&key)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("open projection target '{key}': {error}"))
                })?;
            let subject = session.handle().of().clone();
            let membership = Membership::new(member.clone(), subject.clone());
            let current: Vec<MemberName> = session
                .handle()
                .query()
                .select(Query::<MemberName> {
                    this: Term::from(membership.this().clone()),
                    name: Term::var("name"),
                })
                .perform(&tonk.operator)
                .try_vec()
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "read account member name in '{key}': {error:?}"
                    ))
                })?;
            let root_stale = current.first().is_none_or(|row| row.name.0 != name);

            let mut obsolete = Vec::new();
            if device != member {
                let device_membership = Membership::new(device.clone(), subject);
                obsolete = session
                    .handle()
                    .query()
                    .select(Query::<MemberName> {
                        this: Term::from(device_membership.this().clone()),
                        name: Term::var("name"),
                    })
                    .perform(&tonk.operator)
                    .try_vec()
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!(
                            "read obsolete member name in '{key}': {error:?}"
                        ))
                    })?;
            }

            if !root_stale && obsolete.is_empty() {
                return Ok(false);
            }
            let mut transaction = tonk
                .reactor
                .repository(&key)
                .branch(tonk_account::MAIN_BRANCH)
                .transaction();
            if root_stale {
                transaction =
                    transaction.assert(MemberName::new(membership.this().clone(), name.clone()));
            }
            for stale in obsolete {
                transaction = transaction.retract(stale);
            }
            transaction
                .commit()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "project account member name to '{key}': {error}"
                    ))
                })?;
            Ok::<bool, TonkWorkerError>(true)
        }
        .await;

        let changed = match result {
            Ok(changed) => {
                if changed {
                    tonk.sync_queue.mark_dirty(&key, js_sys::Date::now());
                    report.changed_keys.push(key.clone());
                }
                changed
            }
            Err(error) => {
                log!("account-name projection for '{key}' failed: {error}");
                report.failed_keys.push(key.clone());
                false
            }
        };
        if changed || profile_changed {
            crate::router::sync::publish_self_identity(tonk, &key, tonk_account::MAIN_BRANCH).await;
        }
    }

    Ok(report)
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn converge_account_state(
    _tonk: &TonkState,
) -> Result<tonk_worker_api::AccountConvergenceReport, TonkWorkerError> {
    Ok(Default::default())
}

fn account_state_unavailable() -> TonkWorkerError {
    TonkWorkerError::AccountStateUnavailable(
        "Finish or retry account setup at /account before changing the linked account name"
            .to_string(),
    )
}

async fn adopt_account_display_name(
    tonk: &TonkState,
    name: &str,
) -> Result<tonk_worker_api::AccountDisplayNameResponse, TonkWorkerError> {
    use tonk_schema::{AccountDisplayName, prelude::DidExt as _};

    let ready = require_ready_account_state(tonk)
        .await
        .map_err(|_| account_state_unavailable())?;
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountDisplayName::new(
            ready.subject.this(),
            name.to_string(),
        ))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("commit account display name: {error}"))
        })?;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
    let convergence = converge_account_state(tonk).await?;
    Ok(tonk_worker_api::AccountDisplayNameResponse {
        name: name.to_string(),
        convergence,
    })
}

/// Seed the authoritative name from this device's current profile name when
/// the account repository is ready and still unnamed.
pub(crate) async fn initialize_display_name(
    tonk: &TonkState,
) -> Result<tonk_worker_api::AccountDisplayNameResponse, TonkWorkerError> {
    use tonk_schema::{AccountDisplayName, prelude::DidExt as _};

    if ensure_account_state(tonk).await != AccountStateStatus::Ready {
        return Err(account_state_unavailable());
    }
    let ready = require_ready_account_state(tonk)
        .await
        .map_err(|_| account_state_unavailable())?;
    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open ready account state: {error}")))?;
    let existing: Vec<AccountDisplayName> = branch
        .handle()
        .query()
        .select(Query::<AccountDisplayName> {
            this: Term::from(ready.subject.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read initial account display name: {error:?}"))
        })?;
    if let Some(existing) = existing.into_iter().next() {
        let convergence = converge_account_state(tonk).await?;
        return Ok(tonk_worker_api::AccountDisplayNameResponse {
            name: existing.name.0,
            convergence,
        });
    }

    let name = crate::router::profile_name::resolve_display_name(tonk).await;
    adopt_account_display_name(tonk, &name).await
}

/// Apply the display-name flow used by both the result-bearing HTTP endpoint
/// and the legacy transient command handler.
pub(crate) async fn rename_display_name(
    tonk: &TonkState,
    name: &str,
) -> Result<Option<tonk_worker_api::AccountDisplayNameResponse>, TonkWorkerError> {
    use tonk_schema::{ProfileName, prelude::DidExt as _};
    use tonk_worker_api::{AccountConvergenceReport, AccountDisplayNameResponse};

    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }

    if super::account::provider(tonk).await.is_some() {
        if configured_descriptor(tonk).await.is_none() {
            return Err(account_state_unavailable());
        }
        let response = adopt_account_display_name(tonk, name).await?;
        // Roster upkeep: the switcher row shows the new name.
        super::profiles::upsert_active_entry(tonk, None).await;
        return Ok(Some(response));
    }

    let profile_entity = tonk.profile.did().this();
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(ProfileName::new(profile_entity, name.to_string()))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to persist profile name override: {error}"))
        })?;

    let convergence = AccountConvergenceReport {
        profile_changed: true,
        ..AccountConvergenceReport::default()
    };
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let mut convergence = convergence;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    for key in crate::router::profile_name::real_space_keys(tonk).await {
        match crate::router::profile_name::restamp_member_name(tonk, &key, name).await {
            Ok(()) => {
                tonk.sync_queue.mark_dirty(&key, js_sys::Date::now());
                convergence.changed_keys.push(key.clone());
            }
            Err(error) => {
                log!("restamp MemberName for space '{key}' failed: {error}");
                convergence.failed_keys.push(key.clone());
            }
        }
        crate::router::sync::publish_self_identity(tonk, &key, tonk_account::MAIN_BRANCH).await;
    }

    // Roster upkeep: the switcher row shows the new name.
    super::profiles::upsert_active_entry(tonk, None).await;

    Ok(Some(AccountDisplayNameResponse {
        name: name.to_string(),
        convergence,
    }))
}

#[cfg(test)]
mod tests {
    use dialog_credentials::Ed25519Signer;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    #[cfg(not(target_arch = "wasm32"))]
    use dialog_common::helpers::Provisionable as _;

    use super::*;

    #[dialog_common::test]
    async fn it_requires_the_exact_descriptor_hash_for_readiness() {
        let root = Ed25519Signer::import(&[7; 32]).await.unwrap();
        let descriptor =
            AccountRepositoryDescriptorV1::sign(&root, "https://accounts.example/ucan/")
                .await
                .unwrap();
        let hash = descriptor.content_hash();
        let different = [9_u8; 32];

        assert!(marker_matches(Some(&hash), &descriptor));
        assert!(!marker_matches(None, &descriptor));
        assert!(!marker_matches(Some(&different), &descriptor));
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_projects_each_space_independently_and_retries_only_failures() {
        use dialog_capability::Subject;
        use dialog_credentials::{Credential, Ed25519Verifier};
        use dialog_effects::space::{Space, SpaceExt as _};
        use dialog_varsig::Principal as _;
        use tonk_schema::prelude::DidExt as _;

        let state = crate::router::tests::test_state().await;
        crate::router::profile_name::ensure_display_name(&state)
            .await
            .unwrap();
        let (app, state, _lsp) = crate::router::api_router_with_state(state);
        let key_a = crate::router::tests::put_repo(&app, "account-project-a").await;
        let key_c = crate::router::tests::put_repo(&app, "account-project-c").await;

        // The descriptor must be signed by the root `test_state` persisted:
        // linking now attaches a provider to that exact local root.
        let seed = {
            let tonk = state.read().await;
            crate::router::tests::test_root_seed(&tonk.profile_name)
        };
        let root = Ed25519Signer::import(&seed).await.unwrap();
        let descriptor = AccountRepositoryDescriptorV1::sign(&root, "http://127.0.0.1:9/")
            .await
            .unwrap();
        let missing = Ed25519Signer::import(&[77; 32]).await.unwrap().did();
        let missing_key = missing.repo_key().to_owned();
        {
            let tonk = state.read().await;
            let matching = crate::router::account::tests_matching_request(&tonk).await;
            crate::router::account::persist_link(
                &tonk,
                &tonk_worker_api::AccountLinkRequest {
                    descriptor_hex: hex::encode(descriptor.bytes()),
                    ..matching
                },
            )
            .await
            .unwrap();
            tonk.profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .save(descriptor.content_hash().to_vec())
                .perform(&tonk.operator)
                .await
                .unwrap();
            tonk.reactor
                .profile_repository()
                .branch(tonk_account::MAIN_BRANCH)
                .transaction()
                .assert(Replica::new(tonk.profile.did(), missing.clone()))
                .commit()
                .perform(&tonk.operator)
                .await
                .unwrap();
            tonk.reactor.run_scheduled_polls(&tonk.operator).await;
            assert_eq!(ensure_account_state(&tonk).await, AccountStateStatus::Ready);

            let response = rename_display_name(&tonk, "shared-name")
                .await
                .unwrap()
                .unwrap();
            assert!(response.convergence.profile_changed);
            assert!(response.convergence.changed_keys.contains(&key_a));
            assert!(response.convergence.changed_keys.contains(&key_c));
            assert_eq!(response.convergence.failed_keys, vec![missing_key.clone()]);
        }

        let (before_a, before_c) = {
            let tonk = state.read().await;
            let a = tonk
                .reactor
                .repository(&key_a)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            let c = tonk
                .reactor
                .repository(&key_c)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            (a, c)
        };

        {
            let tonk = state.read().await;
            let verifier: Ed25519Verifier = missing.to_string().parse().unwrap();
            let local = Subject::from(tonk.profile.did()).attenuate(Space::new(&missing_key));
            let credential = local
                .create(Credential::from(verifier))
                .perform(&tonk.operator)
                .await
                .unwrap();
            let repository = Repository::from(credential);
            repository
                .branch(tonk_account::MAIN_BRANCH)
                .open()
                .perform(&tonk.operator)
                .await
                .unwrap();

            let retry = converge_account_state(&tonk).await.unwrap();
            assert!(!retry.profile_changed);
            assert_eq!(retry.changed_keys, vec![missing_key.clone()]);
            assert!(retry.failed_keys.is_empty());

            let no_op = converge_account_state(&tonk).await.unwrap();
            assert!(!no_op.profile_changed);
            assert!(no_op.changed_keys.is_empty());
            assert!(no_op.failed_keys.is_empty());

            let after_a = tonk
                .reactor
                .repository(&key_a)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            let after_c = tonk
                .reactor
                .repository(&key_c)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            assert_eq!(
                before_a, after_a,
                "correct target A receives no retry write"
            );
            assert_eq!(
                before_c, after_c,
                "correct target C receives no retry write"
            );
        }
    }

    /// A native `TonkState` on a randomized profile, plus the local root and
    /// account link that make [`ensure_account_state`] reach `Ready`.
    ///
    /// Yields the state, the running access service, and the signed
    /// descriptor. Callers stop the service and clean up the on-disk verifier
    /// repository themselves, the way the offline test does.
    #[cfg(not(target_arch = "wasm32"))]
    async fn ready_account_state(
        passkey: Option<tonk_worker_api::PasskeyMetadata>,
    ) -> (
        TonkState,
        dialog_common::helpers::Service<
            tonk_access_service::helpers::AccessServiceAddress,
            tonk_access_service::helpers::AccessServer,
        >,
        AccountRepositoryDescriptorV1,
        Ed25519Signer,
        String,
    ) {
        use dialog_operator::Profile;
        use dialog_storage::provider::storage::Storage;
        use dialog_varsig::Principal as _;
        use tonk_access_service::helpers::AccessServiceAddress;

        let service = AccessServiceAddress::start(Default::default())
            .await
            .unwrap();
        let storage = Storage::<crate::worker::DefaultSpace>::default();
        let name = format!("account-state-worker-test-{}", rand::random::<u64>());
        let profile = Profile::open(&name).perform(&storage).await.unwrap();
        let session = crate::session::open(&profile, &storage).await.unwrap();
        let reactor = crate::Reactor::new(profile.clone());
        let state = TonkState {
            profile,
            operator: session.operator,
            storage,
            session_expires_at: session.expires_at,
            profile_name: name.clone(),
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            sync_queue: Default::default(),
            commands: crate::router::command_registry(),
            clients: Default::default(),
            account_keys: Default::default(),
            registry: crate::device::Registry {
                profile: name.clone(),
                directory: dialog_effects::storage::Directory::Profile,
            },
        };
        crate::router::repository::bootstrap_profile(&state)
            .await
            .unwrap();

        let root = Ed25519Signer::generate().await.unwrap();
        let root_signer = root.clone();
        // Hydration syncs the account space, which the access service
        // serves only once its customer has confirmed the emailed
        // activation link.
        service
            .address
            .activate_customer(&root, "worker-account-state@example.com")
            .await
            .unwrap();
        let remote = format!(
            "{}/",
            service.address.access_service_url.trim_end_matches('/')
        );
        let descriptor = AccountRepositoryDescriptorV1::sign(&root, &remote)
            .await
            .unwrap();
        let root_did = root.did().to_string();
        let credential_id = "account-state-test-credential".to_string();
        let delegation =
            tonk_identity::delegation::mint_device_delegation(root, &state.profile.did())
                .await
                .unwrap();
        let delegation_hex = hex::encode(delegation.to_bytes().unwrap());
        // The link attaches a provider to an already-persisted local root, so
        // the root has to exist before persist_link will accept it.
        crate::router::identity::persist_root(
            &state,
            tonk_worker_api::SaveRootRequest {
                credential_id: credential_id.clone(),
                delegation_hex: delegation_hex.clone(),
                passkey,
            },
        )
        .await
        .unwrap();
        crate::router::account::persist_link(
            &state,
            &tonk_worker_api::AccountLinkRequest {
                provider: "https://accounts.example".to_string(),
                root_did,
                credential_id,
                delegation_hex,
                descriptor_hex: hex::encode(descriptor.bytes()),
                initialize_name: false,
            },
        )
        .await
        .unwrap();
        state
            .profile
            .credential()
            .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
            .save(vec![0_u8; 32])
            .perform(&state.operator)
            .await
            .unwrap();

        (state, service, descriptor, root_signer, remote)
    }

    /// Every recorded creation fact on the ready account branch.
    #[cfg(not(target_arch = "wasm32"))]
    async fn recorded_passkey_facts(
        state: &TonkState,
        ready: &ReadyAccountBranch,
    ) -> Vec<tonk_schema::AccountPasskeyCreated> {
        state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap()
            .handle()
            .query()
            .select(Query::<tonk_schema::AccountPasskeyCreated> {
                this: Term::from(ready.subject.this()),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap()
    }

    /// Remove the on-disk verifier repository `NativeSpace` rooted in the
    /// package working directory for one randomized fixture.
    #[cfg(not(target_arch = "wasm32"))]
    fn discard(state: TonkState, key: &str) {
        let local = std::env::current_dir().unwrap().join(key);
        drop(state);
        if local.is_dir() {
            std::fs::remove_dir_all(local).unwrap();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_seeds_passkey_creation_facts_from_the_local_root() {
        let (state, service, _descriptor, _root, _remote) =
            ready_account_state(Some(tonk_worker_api::PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".to_string(),
            }))
            .await;

        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let seeded = recorded_passkey_facts(&state, &ready).await;
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].seconds(), 1_754_380_800);
        assert_eq!(seeded[0].created_on.0, "Chrome on macOS");

        // The sweep runs on every boot and every heartbeat. A second pass must
        // find its own fact and leave it exactly as written.
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let again = recorded_passkey_facts(&state, &ready).await;
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].seconds(), 1_754_380_800);
        assert_eq!(again[0].created_on.0, "Chrome on macOS");

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// A space delegation retained into the account space becomes
    /// `dialog.ucan/*` facts there — the whole point of the account being the
    /// durable home of delegations, since a device regains access by pulling
    /// them rather than by fetching an artifact.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_retains_a_space_delegation_into_the_account_space() {
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_varsig::Principal as _;

        let (state, service, _descriptor, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();

        // A `space -> account-root` delegation, the shape space creation mints.
        let space = dialog_credentials::Ed25519Signer::import(&[7u8; 32])
            .await
            .unwrap();
        let subject = space.did();
        let root = super::super::identity::local_root(&state).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(space))
            .audience(&root.root_did)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        let chain = DelegationChain::new(delegation);

        assert!(
            retain_space_delegation(&state, &chain).await,
            "a ready account must retain the delegation"
        );
        assert!(
            proves_space_access(&state, &subject).await,
            "the retained delegation must prove access to the space it delegates"
        );

        // Content-addressed, so re-retaining the same chain commits nothing.
        assert!(
            !retain_space_delegation(&state, &chain).await,
            "re-retaining an identical chain must not write again"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// Whether the account branch's retained delegations prove the local root
    /// may act on `subject`.
    ///
    /// Asserts through dialog's own reader rather than by inspecting
    /// `dialog.ucan/*` rows: proving is what these facts EXIST for, so a
    /// passing proof is the claim that matters, and it cannot pass on facts
    /// whose envelope does not back them.
    #[cfg(not(target_arch = "wasm32"))]
    async fn proves_space_access(state: &TonkState, subject: &dialog_varsig::Did) -> bool {
        use dialog_ucan::{Parameters, Scope};
        use dialog_ucan_core::command::Command;
        use dialog_ucan_core::subject::Subject as UcanSubject;

        let root = super::super::identity::local_root(state).await.unwrap();
        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        branch
            .handle()
            .delegations()
            .prove(
                root.root_did.clone(),
                Scope {
                    subject: UcanSubject::Specific(subject.clone()),
                    command: Command::parse("/").unwrap(),
                    parameters: Parameters::default(),
                },
            )
            .perform(&state.operator)
            .await
            .is_ok()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_seeds_nothing_when_the_local_root_has_no_passkey_metadata() {
        let (state, service, _descriptor, _root, _remote) = ready_account_state(None).await;

        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        assert!(
            recorded_passkey_facts(&state, &ready).await.is_empty(),
            "a device that only evaluated an existing passkey has nothing to record"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_mounts_hydrates_and_keeps_readiness_offline() {
        let (state, service, descriptor, _root, _remote) = ready_account_state(None).await;

        let before = crate::router::profile_name::resolve_display_name(&state).await;
        assert!(matches!(
            rename_display_name(&state, "must-not-fallback").await,
            Err(TonkWorkerError::AccountStateUnavailable(_))
        ));
        assert_eq!(
            crate::router::profile_name::resolve_display_name(&state).await,
            before,
            "an unhydrated linked rename must not change the local cache"
        );

        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        assert_eq!(ready.subject, descriptor.account_subject().clone());
        assert!(
            !state.reactor.repos().read().contains_key(&ready.key),
            "the account key routes the sweep; no repository — and no \
             database — exists behind it",
        );

        let initialized = initialize_display_name(&state).await.unwrap();
        assert_eq!(initialized.name, before);
        let renamed = rename_display_name(&state, "linked-name")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name, "linked-name");
        let initialized_again = initialize_display_name(&state).await.unwrap();
        assert_eq!(
            initialized_again.name, "linked-name",
            "initialization must not overwrite an existing account fact"
        );
        let account = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let names: Vec<tonk_schema::AccountDisplayName> = account
            .handle()
            .query()
            .select(Query::<tonk_schema::AccountDisplayName> {
                this: Term::from(ready.subject.this()),
                name: Term::var("name"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(names[0].name.0, "linked-name");

        service.stop().await.unwrap();
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );

        discard(state, &ready.key);
    }
}
