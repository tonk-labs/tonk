//! Hidden root-owned account repository mounting and trusted hydration.
//!
//! Mounting is deliberately separate from readiness. The local verifier-only
//! repository may exist while the remote is unavailable, but no account-state
//! mutation API receives its routing key until the trusted-base marker matches
//! the signed descriptor.

use std::collections::HashSet;
use std::sync::Mutex;

use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_query::{Output as _, Query, Term};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{Repository, RepositoryExt as _, SiteAddress, Upstream};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::DelegationChain;
use tonk_account::{
    AccountRepositoryDescriptorV1, AccountStateStatus, CreateGenesis, RemotePresence,
    probe_remote_main, publish_genesis_if_absent,
};
use tonk_common::log;
use tonk_schema::{AccountPasskeyCreated, Replica, prelude::DidExt as _};

use crate::TonkWorkerError;
use crate::worker::TonkState;

const META_BRANCH: &str = "meta";

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

/// The account routing keys this profile must keep hidden, resolved once.
///
/// [`is_account_key`] runs in middleware ahead of *every* repository-addressed
/// request, and resolving it from scratch is not cheap: reading the link record
/// costs a credential load plus a delegation and a descriptor signature
/// verification, and on a profile whose link is absent — every profile that has
/// never signed in — that lookup misses and the index fallback below pays a
/// profile-meta acquire and a `Replica` query instead. None of that changes
/// between the few writes that install a link or index a replica, so the answer
/// is resolved once and held here.
///
/// The inner `None` means "not resolved yet". An empty set is a real answer:
/// this profile has no account repository to hide.
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

/// Whether `key` names the configured or already-indexed account repository.
///
/// The profile-index fallback keeps a previously mounted account hidden even
/// when the local link record later becomes unreadable.
pub(crate) async fn is_account_key(tonk: &TonkState, key: &str) -> bool {
    if let Some(keys) = tonk.account_keys.get() {
        return keys.contains(key);
    }
    let (keys, complete) = resolve_account_keys(tonk).await;
    let hidden = keys.contains(key);
    // A failed index read yields a partial answer. Serve this request from it,
    // exactly as the uncached path did, but do not freeze it: caching a
    // transient miss would un-hide the account until the next write.
    if complete {
        tonk.account_keys.set(keys);
    }
    hidden
}

/// Every routing key that names this profile's account repository, plus whether
/// the answer is complete enough to cache.
async fn resolve_account_keys(tonk: &TonkState) -> (HashSet<String>, bool) {
    let mut keys = HashSet::new();
    if let Some(descriptor) = configured_descriptor(tonk).await {
        keys.insert(descriptor.account_subject().repo_key().to_owned());
    }

    let Ok(meta) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        return (keys, false);
    };
    let rows: Result<Vec<Replica>, _> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::from(Replica::account_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await;
    let Ok(rows) = rows else {
        return (keys, false);
    };
    keys.extend(rows.into_iter().filter_map(|replica| {
        replica
            .subject
            .0
            .to_string()
            .parse::<dialog_varsig::Did>()
            .ok()
            .map(|subject| subject.repo_key().to_owned())
    }));
    (keys, true)
}

async fn mount_account_repository(
    tonk: &TonkState,
    descriptor: &AccountRepositoryDescriptorV1,
) -> Result<(String, Repository), TonkWorkerError> {
    let subject = descriptor.account_subject().clone();
    let key = subject.repo_key().to_owned();

    let repository = match tonk
        .profile
        .repository(&key)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(_) => {
            let verifier: Ed25519Verifier = subject.to_string().parse().map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "account subject is not an Ed25519 did:key: {error:?}"
                ))
            })?;
            let local = Subject::from(tonk.profile.did()).attenuate(Space::new(&key));
            let credential = local
                .create(Credential::from(verifier))
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to mount local account repository: {error}"
                    ))
                })?;
            Repository::from(credential)
        }
    };

    let address = SiteAddress::from(UcanAddress::new(descriptor.remote().as_str()));
    let remote = match repository
        .remote(tonk_account::ORIGIN_REMOTE)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(remote) => {
            if remote.address().site() != &address || remote.did() != subject {
                return Err(TonkWorkerError::Conflict(
                    "mounted account repository has different immutable remote configuration"
                        .to_string(),
                ));
            }
            remote
        }
        Err(_) => repository
            .remote(tonk_account::ORIGIN_REMOTE)
            .create(address.clone())
            .subject(subject.clone())
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "failed to configure account repository remote: {error}"
                ))
            })?,
    };

    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open account main branch: {error}"))
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
        Some(_) => {
            return Err(TonkWorkerError::Conflict(
                "mounted account main branch tracks a different upstream".to_string(),
            ));
        }
        None => branch
            .set_upstream(&remote_branch)
            .perform(&tonk.operator)
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("failed to set account main upstream: {error}"))
            })?,
    }

    record_account_meta(tonk, &repository, &address).await?;

    // Open the configured branch through the reactor. This is what puts the
    // hidden repository in the background drain's pull population.
    tonk.reactor
        .repository(&key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open account branch in reactor: {error}"))
        })?;

    Ok((key, repository))
}

async fn record_account_meta(
    tonk: &TonkState,
    repository: &Repository,
    address: &SiteAddress,
) -> Result<(), TonkWorkerError> {
    let subject = repository.did();
    let replica = Replica::account(tonk.profile.did(), subject.clone());
    let remote = replica.remote(tonk_account::ORIGIN_REMOTE, subject, address);
    let tracked = remote.branch(tonk_account::MAIN_BRANCH);

    let meta = repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open account meta branch: {error}"))
        })?;
    meta.transaction()
        .assert(replica.clone())
        .assert(replica.branch(META_BRANCH))
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
            TonkWorkerError::Internal(format!("failed to record account repository meta: {error}"))
        })?;

    // The profile index receives only the explicit account replica. No user
    // status, roster, pause preference, template, invite, or backup facts.
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(replica)
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to index account repository: {error}"))
        })?;
    // The index the routing-key fallback reads just gained a row. Mounting
    // implies the link record was readable, so a resolve *now* would find the
    // same key from the descriptor — but a resolve that ran during an earlier
    // transient read failure cached an empty set, and only this clears it. The
    // cost is one re-resolve per heartbeat, against leaving the repository
    // visible until the next write.
    tonk.account_keys.invalidate();
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

async fn hydrate_untrusted(
    tonk: &TonkState,
    key: &str,
    repository: &Repository,
) -> Result<(), TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let remote = repository
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
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to hydrate account repository: {error}"
                    ))
                })?;
        }
        Ok(RemotePresence::Absent) => {
            tonk.reactor
                .repository(key)
                .branch(tonk_account::MAIN_BRANCH)
                .transaction()
                .commit()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "failed to create local account genesis: {error}"
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

/// Reconcile the ready account branch: pull, project, push.
///
/// This is the account repository's whole sweep. It is deliberately not the
/// generic per-branch [`sync`](crate::router::sync) route: a hidden system
/// replica has no pause preference to honor and no status chip to stamp, and
/// routing it through both would pull and push it twice per heartbeat.
///
/// `Err` names the step that did not land, so the caller can report a sweep
/// worth retrying. Convergence failing is not one of those: it is per-space and
/// keeps its own retry list, so it is logged and the sweep still counts.
async fn sync_ready(tonk: &TonkState, key: &str) -> Result<(), String> {
    let session = tonk
        .reactor
        .repository(key)
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| format!("account branch unavailable: {error}"))?;
    session
        .handle()
        .pull()
        .perform(&tonk.operator)
        .await
        .map_err(|error| format!("account pull failed: {error}"))?;
    // After the pull, so the seed sees what other devices already recorded,
    // and before the push, so anything it writes leaves with this sweep.
    if seed_passkey_facts(tonk).await {
        log!("recorded this device's passkey creation facts in the account space");
    }
    if let Err(error) = converge_account_state(tonk).await {
        log!("account-state convergence after sync failed: {error}");
    }
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
    let Some(descriptor) = configured_descriptor(tonk).await else {
        return (AccountStateStatus::Unconfigured, Ok(()));
    };

    let (key, repository) = match mount_account_repository(tonk, &descriptor).await {
        Ok(mounted) => mounted,
        Err(error) => {
            log!("account repository mount failed: {error}");
            return (AccountStateStatus::Unhydrated, Ok(()));
        }
    };

    match trusted_marker(tonk).await {
        Ok(marker) if marker_matches(marker.as_deref(), &descriptor) => {
            let swept = sync_ready(tonk, &key).await;
            (AccountStateStatus::Ready, swept)
        }
        Ok(_) => match hydrate_untrusted(tonk, &key, &repository).await {
            Ok(()) => match mark_trusted(tonk, &descriptor).await {
                Ok(()) => {
                    // The path a freshly created account takes, where the
                    // ready sweep above has not run yet.
                    if seed_passkey_facts(tonk).await {
                        log!("recorded this device's passkey creation facts in the account space");
                    }
                    if let Err(error) = converge_account_state(tonk).await {
                        log!("account-state convergence after hydration failed: {error}");
                    }
                    (AccountStateStatus::Ready, Ok(()))
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
        .repository(&key)
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
        .repository(&ready.key)
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

/// Retain a `space → account-root` delegation into the account space, so the
/// authority it carries is durable data rather than a device-local artifact.
///
/// The account repository is the durable home of delegations: a device that
/// pulls it regains access, because the delegations are just facts in a branch
/// it syncs. Retaining is content-addressed, so re-retaining the same chain
/// commits nothing — a caller may run this on every space creation without
/// checking first.
///
/// Best-effort by design, and deliberately not fatal to the operation that
/// triggered it. A space is fully usable on this device the moment its
/// delegation is saved to the profile's own access branch; retaining into the
/// account is what makes it recoverable on the *next* device. Failing space
/// creation because a hidden system repository was mid-hydration would trade a
/// working space for a recoverable one. Returns whether it retained, and logs
/// every reason it did not.
pub(crate) async fn retain_space_delegation(tonk: &TonkState, chain: &DelegationChain) -> bool {
    let ready = match require_ready_account_state(tonk).await {
        Ok(ready) => ready,
        // Unconfigured and unhydrated are ordinary states for a signed-out or
        // still-hydrating profile, not failures worth a line in the log.
        Err(_) => return false,
    };
    let branch = match tonk
        .reactor
        .repository(&ready.key)
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
    match branch
        .handle()
        .delegations()
        .retain(UcanDelegation(chain.clone()))
        .perform(&tonk.operator)
        .await
    {
        // An empty list means every certificate was already retained.
        Ok(retained) => {
            let wrote = !retained.is_empty();
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            if wrote {
                tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
            }
            wrote
        }
        Err(error) => {
            log!("retain space delegation into account space: {error}");
            false
        }
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
        .repository(&ready.key)
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
        .repository(&ready.key)
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
        .repository(&ready.key)
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
        .repository(&ready.key)
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

        (state, service, descriptor)
    }

    /// Every recorded creation fact on the ready account branch.
    #[cfg(not(target_arch = "wasm32"))]
    async fn recorded_passkey_facts(
        state: &TonkState,
        ready: &ReadyAccountBranch,
    ) -> Vec<tonk_schema::AccountPasskeyCreated> {
        state
            .reactor
            .repository(&ready.key)
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
        let (state, service, _descriptor) =
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

        let (state, service, _descriptor) = ready_account_state(None).await;
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
            .issuer(space)
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
            proves_space_access(&state, &ready, &subject).await,
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
    async fn proves_space_access(
        state: &TonkState,
        ready: &ReadyAccountBranch,
        subject: &dialog_varsig::Did,
    ) -> bool {
        use dialog_ucan::{Parameters, Scope};
        use dialog_ucan_core::command::Command;
        use dialog_ucan_core::subject::Subject as UcanSubject;

        let root = super::super::identity::local_root(state).await.unwrap();
        let branch = state
            .reactor
            .repository(&ready.key)
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
        let (state, service, _descriptor) = ready_account_state(None).await;

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
        let (state, service, descriptor) = ready_account_state(None).await;

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
        assert!(state.reactor.repos().read().contains_key(&ready.key));

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
            .repository(&ready.key)
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
