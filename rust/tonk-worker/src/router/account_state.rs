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
    AccountStateStatus, CreateGenesis, RemotePresence, probe_remote_main, publish_genesis_if_absent,
};
use tonk_common::log;
use tonk_identity::sealed::RecipientKey;
use tonk_schema::{
    AccountSealedInbox, Replica, SecretMessage, SecretPrincipal, SeedKind, prelude::DidExt as _,
};
use zeroize::Zeroizing;

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
    subject: &dialog_varsig::Did,
) -> Result<(), TonkWorkerError> {
    tonk.profile
        .credential()
        .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
        .save(subject.as_str().as_bytes().to_vec())
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!(
                "failed to save account trusted-base marker: {error}"
            ))
        })
}

/// Whether the trusted base already recorded names this account.
///
/// The account DID, not a descriptor hash: what the marker answers is
/// "did I trust a base for THIS account", and the subject is what says
/// which account.
fn marker_matches(marker: Option<&[u8]>, subject: &dialog_varsig::Did) -> bool {
    marker == Some(subject.as_str().as_bytes())
}

/// Where the account syncs.
///
/// The address the access service named at enrollment, when one is
/// recorded — that service is the authority on where it serves from.
/// Otherwise this deployment's own `/ucan/` endpoint: the origin
/// serving the page IS the access service that serves it, so a device
/// knows the address before it knows anything about an account, with
/// nothing fetched and nothing published to learn it.
pub(crate) async fn account_remote(tonk: &TonkState) -> Result<String, TonkWorkerError> {
    if let Some(address) = super::customer::provider_address(tonk).await {
        return Ok(address);
    }
    // What the email lookup resolved, before enrollment has recorded
    // anything: the account's own document naming where it syncs.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if let Some(address) = super::email_status::resolved_service() {
        return Ok(address);
    }
    // What the link named, which is where the linking party was
    // actually talking to the service.
    if let Some(remote) = super::account::attachment(tonk)
        .await
        .and_then(|record| record.remote().map(ToOwned::to_owned))
    {
        return Ok(remote);
    }
    // This deployment's own endpoint. The origin serving the page IS
    // the access service that serves it, so a device knows where to
    // sync before it knows anything about an account.
    Ok(format!(
        "{}ucan/",
        super::customer::service_origin()?.as_str()
    ))
}

/// Whether this profile has an account to sync at all.
///
/// The attachment, not an address: every device can name an address
/// (its own origin, at worst), so the address does not say whether
/// there is an account behind it. The attachment does.
async fn account_configured(tonk: &TonkState) -> bool {
    super::account::attachment(tonk).await.is_some()
}

/// Current durable account-state status, without a network request.
pub(crate) async fn status(tonk: &TonkState) -> AccountStateStatus {
    let Ok(root) = super::identity::local_root(tonk).await else {
        return AccountStateStatus::Unconfigured;
    };
    if !account_configured(tonk).await {
        return AccountStateStatus::Unconfigured;
    }
    match trusted_marker(tonk).await {
        Ok(marker) if marker_matches(marker.as_deref(), &root.root_did) => {
            AccountStateStatus::Ready
        }
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

/// Every routing key that names this profile's account. The account
/// subject IS the local root, so one key derives from it.
async fn resolve_account_keys(tonk: &TonkState) -> HashSet<String> {
    let mut keys = HashSet::new();
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
    subject: &dialog_varsig::Did,
) -> Result<String, TonkWorkerError> {
    let subject = subject.clone();
    let key = subject.repo_key().to_owned();
    let repository = Repository::from(&tonk.profile);

    let address = SiteAddress::from(UcanAddress::new(account_remote(tonk).await?.as_str()));
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

    // The REACTOR's cached session, not a fresh handle. A branch handle
    // captures its upstream cell when first opened, and the reactor's
    // profile-main session opens early — boot chores, enrollment fact
    // writes — before any account remote exists. Setting the upstream on
    // a separately opened handle published it durably but left the
    // cached cell empty, so every sweep's pull and push through the
    // reactor answered `Branch main has no upstream` FOREVER: the exact
    // wedge that left a signed-up browser watching its 403s turn into
    // 200s with nothing able to hydrate. Performed on the cached handle,
    // the write lands in both places at once.
    let session = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("failed to open profile main branch: {error}"))
        })?;
    let branch = session.handle();
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
    let pushed = session.handle().push().perform(&tonk.operator).await;
    // Whether this push was served is the registration state, said by
    // the only party that knows. Nothing polls for it: an unactivated
    // account is refused here on every drain, and activation is that
    // refusal turning into a success.
    //
    // The refusal is read out here, before any await: an `&dyn Error` is
    // not `Send`, and holding one across a suspension point would make
    // every caller's future non-`Send` — which natively is every axum
    // handler that reaches this.
    let declined = pushed
        .as_ref()
        .err()
        .and_then(|error| super::sync::authorization_reason(error))
        .cloned();
    observe_registration(tonk, declined).await;
    pushed.map_err(|error| format!("account push failed: {error}"))?;
    Ok(())
}

/// What a push outcome says about registration, if anything.
///
/// `None` means it said nothing and no fact should be written. That is
/// the common answer: most pushes succeed for reasons unrelated to
/// registration, and most failures are offline or a lapsed session.
///
/// Success only means activation for an account that had been told it
/// was waiting. Writing `Active` from every successful push would
/// fabricate a registration for accounts that never had one.
fn observed_status(
    declined: Option<&dialog_capability::access::AuthorizeError>,
    was_awaiting: bool,
) -> Option<tonk_account::customer::CustomerStatus> {
    use dialog_capability::access::{AuthorizeError, Recourse};
    use tonk_account::customer::CustomerStatus;

    match declined {
        None => was_awaiting.then_some(CustomerStatus::Active),
        Some(AuthorizeError::Declined { recourse, .. }) => Some(match recourse {
            // Still waiting on the email. Worth writing even when this
            // device already believed it: another device may have
            // enrolled, and this is how that reaches here.
            Recourse::Retry => CustomerStatus::Registered,
            Recourse::None => CustomerStatus::Suspended,
        }),
        // Every other refusal is about the proof, not the registration.
        Some(_) => None,
    }
}

/// Record what the remote just said about this account's registration.
///
/// Reads both directions, not only the clearing one: an account
/// suspended after it was active is refused where it used to be served,
/// and a status that only ever moved forward would leave every device
/// believing it still syncs.
///
/// Best-effort throughout. This is an observation ridden along on a sync
/// that has already done its job, so a failure to record must not turn a
/// successful push into a failed one.
async fn observe_registration(
    tonk: &TonkState,
    declined: Option<dialog_capability::access::AuthorizeError>,
) {
    let was_awaiting = matches!(
        super::customer::registration(tonk).await,
        super::customer::Registration::AwaitingActivation { .. }
    );
    let Some(observed) = observed_status(declined.as_ref(), was_awaiting) else {
        return;
    };

    // The address rides along because `record_customer_status` writes
    // the whole fact; it is not being changed here. No recorded address
    // means no enrollment on this device, and nothing to complete.
    let email = match super::customer::registration(tonk).await {
        super::customer::Registration::AwaitingActivation { email } => email,
        _ => match super::customer::account_registration(tonk).await.email {
            Some(email) => email,
            None => return,
        },
    };
    if let Err(error) = super::customer::record_customer_status(tonk, observed, &email, None).await
    {
        log!("registration observation not recorded: {error}");
    }
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
    // The pull above went through, which is the gate saying this account
    // is served. Recorded on THIS arm too, not only on first hydration:
    // the browser that enrolled already holds the trusted marker, so it
    // takes this path on every sweep and never took the hydrate one. Its
    // "awaiting confirmation" row waited on a fact nothing on that device
    // would ever write, so opening the emailed link left the screen that
    // sent you there unchanged.
    super::customer::record_activation(tonk).await;
    // After the pull, so the seed sees what other devices already recorded,
    // and before the push, so anything it writes leaves with this sweep.
    if seed_sealed_inbox(tonk).await {
        log!("published the account sealed-inbox address in the account space");
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

    // An account with somewhere to sync. A root exists from the moment
    // a passkey does, so the root alone does not mean the account is
    // configured; an address does, and it comes from the customer fact,
    // the resolved DID document, or the link.
    let Ok(root) = super::identity::local_root(tonk).await else {
        return (AccountStateStatus::Unconfigured, Ok(()));
    };
    if !account_configured(tonk).await {
        return (AccountStateStatus::Unconfigured, Ok(()));
    }

    let key = match configure_account_upstream(tonk, &root.root_did).await {
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
        Ok(marker) if marker_matches(marker.as_deref(), &root.root_did) => {
            let swept = sync_ready(tonk, &key).await;
            (AccountStateStatus::Ready, swept)
        }
        Ok(_) => match hydrate_untrusted(tonk).await {
            Ok(()) => match mark_trusted(tonk, &root.root_did).await {
                Ok(()) => {
                    // The path a freshly created account takes, where the
                    // ready sweep above has not run yet.
                    if seed_sealed_inbox(tonk).await {
                        log!("published the account sealed-inbox address in the account space");
                    }
                    describe_own_device(tonk).await;
                    // Hydrating IS activation, observed rather than asked
                    // for: the account remote is attached from enrollment
                    // and the gate refuses an unconfirmed customer, so a
                    // pull that succeeds is the service saying the emailed
                    // link was opened. Recording it here is what replaced
                    // polling a status endpoint — and it works on a device
                    // that never opened the link, since it learns from the
                    // sync it was already doing.
                    super::customer::record_activation(tonk).await;
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
    let root = super::identity::local_root(tonk)
        .await
        .map_err(|_| TonkWorkerError::Conflict("account state is unconfigured".to_string()))?;
    let marker = trusted_marker(tonk).await?;
    if !marker_matches(marker.as_deref(), &root.root_did) {
        return Err(TonkWorkerError::Conflict(
            "account state has no trusted remote base".to_string(),
        ));
    }
    let subject = root.root_did.clone();
    let key = subject.repo_key().to_owned();
    tonk.reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    Ok(ReadyAccountBranch { key, subject })
}

/// The passkeys that can recover this account, newest first.
///
/// Found through the envelopes rather than by a field: each passkey's row is
/// keyed on the custody DID its PRF output derives, which is the `to` of the
/// `secret:message` this account sealed to it. So the account is reached by
/// the message's `sender`, and no row carries a second copy of it.
async fn read_passkeys(
    tonk: &TonkState,
    ready: &ReadyAccountBranch,
) -> Result<Vec<tonk_schema::RecoveryPasskey>, TonkWorkerError> {
    use dialog_query::{Output as _, Query, Term};

    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open ready account state: {error}")))?;

    // `from` is optional on the concept, so it binds as a variable and the
    // sender is matched here.
    let envelopes: Vec<SecretMessage> = branch
        .handle()
        .query()
        .select(Query::<SecretMessage> {
            this: Term::var("this"),
            to: Term::var("to"),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read the account envelopes: {error:?}"))
        })?;

    let mut passkeys = Vec::new();
    for envelope in envelopes {
        if envelope
            .from
            .as_ref()
            .is_none_or(|sender| sender.0 != ready.subject.this())
        {
            continue;
        }
        let rows: Vec<tonk_schema::RecoveryPasskey> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::RecoveryPasskey> {
                this: Term::from(envelope.to.0.clone()),
                credential_id: Term::var("credential_id"),
                created_at: Term::var("created_at"),
                created_on: Term::var("created_on"),
                name: Term::var("name"),
                display_name: Term::var("display_name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|error| {
                TonkWorkerError::Internal(format!("read the passkey facts: {error:?}"))
            })?;
        passkeys.extend(rows);
    }
    passkeys.sort_by_key(|passkey| std::cmp::Reverse(passkey.seconds()));
    Ok(passkeys)
}

/// The most recently created passkey's metadata, for the account panel.
///
/// Best-effort by design — the dashboard has an explicit unavailable state and
/// must not fail because a hidden system repository is mid-hydration. Every
/// `None` that is not simply "no fact" is logged, so an unreadable branch is
/// visible rather than silent.
pub(crate) async fn passkey_facts(tonk: &TonkState) -> Option<tonk_worker_api::PasskeyMetadata> {
    // No readiness gate: these are display facts the enrolling device
    // itself wrote on profile main, so they are answerable the moment
    // the account exists — which is exactly the window the dashboard
    // first renders in. The gate guards authoritative edits, and a
    // summary read is not one.
    let subject = super::identity::local_root(tonk).await.ok()?.root_did;
    let key = subject.repo_key().to_owned();
    let branch = ReadyAccountBranch { key, subject };
    match read_passkeys(tonk, &branch).await {
        Ok(passkeys) => {
            passkeys
                .into_iter()
                .next()
                .map(|passkey| tonk_worker_api::PasskeyMetadata {
                    created_at: passkey.seconds(),
                    created_on: passkey.created_on.0,
                })
        }
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
    let Ok(root) = super::identity::local_root(tonk).await else {
        return false;
    };
    let subject = root.root_did.clone();
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
    let Ok(remote) = account_remote(tonk).await else {
        log!("the worker origin is unavailable, so the account has no remote");
        return false;
    };
    let address = SiteAddress::from(UcanAddress::new(remote.as_str()));
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
pub(crate) async fn describe_own_device(tonk: &TonkState) {
    // No root is an ordinary state for a signed-out profile, reached on
    // every sweep; not worth a line in the log.
    let Ok(root) = super::identity::local_root(tonk).await else {
        return;
    };
    if let Err(error) = crate::onboarding::describe_device_link(
        tonk,
        &root.delegation,
        crate::onboarding::device_title(),
    )
    .await
    {
        log!("describe this device's link: {error}");
    }
}

/// The account's published encryption key, when the account is ready and
/// has one.
pub(crate) async fn read_sealed_inbox(
    tonk: &TonkState,
    ready: &ReadyAccountBranch,
) -> Result<Option<dialog_varsig::Did>, TonkWorkerError> {
    published_sealed_inbox(tonk, &ready.subject).await
}

/// The encryption key published for `account` on profile `main`, if any.
/// Reads the branch as it is, ready or not: the fact arrives with the
/// account pull or is written locally, and neither needs the descriptor
/// gate to be read back.
pub(crate) async fn published_sealed_inbox(
    tonk: &TonkState,
    account: &dialog_varsig::Did,
) -> Result<Option<dialog_varsig::Did>, TonkWorkerError> {
    let branch = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("open profile main: {error}")))?;
    let rows: Vec<AccountSealedInbox> = branch
        .handle()
        .query()
        .select(Query::<AccountSealedInbox> {
            this: Term::from(account.this()),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("read account sealed-inbox address: {error:?}"))
        })?;
    rows.into_iter()
        .next()
        .map(|row| {
            row.address.0.to_string().parse().map_err(|error| {
                TonkWorkerError::Internal(format!(
                    "the published sealed-inbox address is not a DID: {error}"
                ))
            })
        })
        .transpose()
}

/// Publish the recipient the root record carries as the account's
/// encryption key, when the account space does not already say so.
/// Returns whether it wrote.
///
/// Only a ceremony that held the secret records a recipient, so a
/// device that merely links has nothing to contribute and returns
/// `false`. A differing published key is replaced: the record comes
/// from the most recent ceremony, and rotation is what changes it.
pub(crate) async fn seed_sealed_inbox(tonk: &TonkState) -> bool {
    let Ok(ready) = require_ready_account_state(tonk).await else {
        return false;
    };
    let Ok(root) = super::identity::local_root(tonk).await else {
        return false;
    };
    let Some(recipient) = root.encryption_key else {
        return false;
    };
    match read_sealed_inbox(tonk, &ready).await {
        Ok(Some(published)) if published == recipient => return false,
        Ok(_) => {}
        Err(error) => {
            log!("account encryption key unreadable before seeding: {error}");
            return false;
        }
    }
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        .assert(AccountSealedInbox::new(
            ready.subject.this(),
            recipient.this(),
        ))
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("commit account encryption key: {error}");
        return false;
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
    true
}

/// Seal `seed` (the signing seed `subject` derives from) to the account's
/// published sealed-inbox address and record it as a [`SecretMessage`]
/// plus the [`SecretPrincipal`] naming it, in the
/// account space, so any device on the account can re-issue the subject
/// after a passkey ceremony opens it. Returns whether it wrote.
///
/// Best-effort, like [`retain_space_delegation`]: the subject is usable
/// the moment its signer exists locally, and an account that has not
/// published a key yet (one predating the key, or not ready) is logged
/// and left for the next sweep rather than failing the caller.
pub(crate) async fn custody_seed(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
    kind: SeedKind,
    seed: Zeroizing<[u8; 32]>,
) -> bool {
    let (recipient, ready) = match custody_recipient(tonk).await {
        Ok(found) => found,
        Err(error) => {
            log!("seed for {subject} not custodied: {error}");
            return false;
        }
    };
    let sealed = match RecipientKey::try_from(&recipient) {
        Ok(key) => match key.secret().conceal(&seed, subject) {
            Ok(sealed) => sealed.encode(),
            Err(error) => {
                log!("seed for {subject} not custodied: {error}");
                return false;
            }
        },
        Err(error) => {
            log!("seed for {subject} not custodied: {error}");
            return false;
        }
    };
    let message = SecretMessage::new(&recipient, sealed);
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .transaction()
        // Two rows: the envelope, and the principal whose seed it carries.
        // Asserted together — a principal naming a message that was never
        // written would be a seed nothing can open.
        .assert(message.clone())
        .assert(SecretPrincipal::new(subject, kind, message.this()))
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("commit custodied seed for {subject}: {error}");
        return false;
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if let Some(ready) = ready {
        tonk.sync_queue.mark_dirty(&ready.key, js_sys::Date::now());
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = ready;
    true
}

/// The recipient custodied seeds are sealed to on this device, and the
/// ready account branch when the recipient is a linked passkey account's.
///
/// A linked account publishes its key from the ceremony that held the
/// secret; an onboarding account's secret is local, so its key is derived
/// here and published on profile `main` the first time it is needed. That
/// is the branch the account becomes the upstream of at accreditation, so
/// rows sealed before it land where rows sealed after it do.
async fn custody_recipient(
    tonk: &TonkState,
) -> Result<(dialog_varsig::Did, Option<ReadyAccountBranch>), TonkWorkerError> {
    match super::identity::local_root(tonk).await {
        Ok(root) => {
            let ready = require_ready_account_state(tonk).await.ok();
            if let Some(recipient) = published_sealed_inbox(tonk, &root.root_did).await? {
                return Ok((recipient, ready));
            }
            // A ceremony recorded the key with the root but the sweep has
            // not published it yet (the account branch may not be ready).
            // Publish it here: profile main is where it lives either way.
            let Some(recipient) = root.encryption_key else {
                return Err(TonkWorkerError::Conflict(
                    "the account has not published its encryption key on this device; a \
                     passkey assertion derives it"
                        .to_string(),
                ));
            };
            tonk.reactor
                .profile_repository()
                .branch(tonk_account::MAIN_BRANCH)
                .transaction()
                .assert(AccountSealedInbox::new(
                    root.root_did.this(),
                    recipient.this(),
                ))
                .commit()
                .perform(&tonk.operator)
                .await
                .map_err(|error| {
                    TonkWorkerError::Internal(format!("publish account encryption key: {error}"))
                })?;
            Ok((recipient, ready))
        }
        Err(TonkWorkerError::RootRequired) => {
            let secret = crate::onboarding::account(tonk).await?;
            let recipient = secret.secret().did();
            let account = secret
                .signer()
                .await
                .map_err(|error| TonkWorkerError::Internal(format!("{error}")))?
                .did();
            if published_sealed_inbox(tonk, &account).await?.is_none() {
                tonk.reactor
                    .profile_repository()
                    .branch(tonk_account::MAIN_BRANCH)
                    .transaction()
                    .assert(AccountSealedInbox::new(account.this(), recipient.this()))
                    .commit()
                    .perform(&tonk.operator)
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!(
                            "publish onboarding encryption key: {error}"
                        ))
                    })?;
            }
            Ok((recipient, None))
        }
        Err(error) => Err(error),
    }
}

/// Project the authoritative account display name into the local profile
/// name cache and every known real-space roster.
///
/// The idempotent catch-up: a rename projects at the moment it happens,
/// and this runs on every sweep for whatever that moment could not reach
/// (a space that was unmounted, a name another device chose). Every
/// space is checked and written independently; a failure is logged and
/// left for the next sweep rather than failing the others.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn converge_account_state(tonk: &TonkState) -> Result<(), TonkWorkerError> {
    use tonk_schema::{AccountDisplayName, ProfileName};

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
        return Ok(());
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

    project_member_names(tonk, &ready.subject, &name, profile_changed).await;
    Ok(())
}

/// Project `name` onto `member`'s row in every known real space's roster,
/// queueing each space that changed for sync. `republish` also refreshes
/// the self-identity overlay of the spaces that did not change, for a
/// profile-name cache that did.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn project_member_names(
    tonk: &TonkState,
    member: &dialog_varsig::Did,
    name: &str,
    republish: bool,
) {
    for key in crate::router::profile_name::real_space_keys(tonk).await {
        let changed = match crate::router::profile_name::project_member_name(
            tonk, &key, member, name,
        )
        .await
        {
            Ok(changed) => {
                if changed {
                    tonk.sync_queue.mark_dirty(&key, js_sys::Date::now());
                }
                changed
            }
            Err(error) => {
                log!("member name projection for '{key}' failed: {error}");
                false
            }
        };
        if changed || republish {
            crate::router::sync::publish_self_identity(tonk, &key, tonk_account::MAIN_BRANCH).await;
        }
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub(crate) async fn converge_account_state(_tonk: &TonkState) -> Result<(), TonkWorkerError> {
    Ok(())
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
    // The rename's own fan-out: every space this device can reach gets
    // the name now and is queued for sync; the sweep catches up the rest.
    converge_account_state(tonk).await?;
    Ok(tonk_worker_api::AccountDisplayNameResponse {
        name: name.to_string(),
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
        converge_account_state(tonk).await?;
        return Ok(tonk_worker_api::AccountDisplayNameResponse {
            name: existing.name.0,
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
    use tonk_worker_api::AccountDisplayNameResponse;

    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }

    if super::account::provider(tonk).await.is_some() {
        if ensure_account_state(tonk).await != AccountStateStatus::Ready {
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

    // Keyed the way the membership rows were written: on the persisted
    // root when there is one, else on the device.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let member = super::account::member_did(tonk).await?;
        project_member_names(tonk, &member, name, true).await;
    }

    // Roster upkeep: the switcher row shows the new name.
    super::profiles::upsert_active_entry(tonk, None).await;

    Ok(Some(AccountDisplayNameResponse {
        name: name.to_string(),
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

    /// The marker answers "did I trust a base for THIS account", so a
    /// marker naming another account — or none at all — is not ready.
    #[dialog_common::test]
    async fn it_requires_the_marker_to_name_this_account() {
        use dialog_varsig::Principal as _;

        let root = Ed25519Signer::import(&[7; 32]).await.unwrap();
        let subject = root.did();
        let other = Ed25519Signer::import(&[9; 32]).await.unwrap().did();

        assert!(marker_matches(Some(subject.as_str().as_bytes()), &subject));
        assert!(!marker_matches(None, &subject));
        assert!(!marker_matches(Some(other.as_str().as_bytes()), &subject));
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_projects_the_name_to_each_space_and_catches_up_the_ones_it_could_not_reach() {
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
        let missing = Ed25519Signer::import(&[77; 32]).await.unwrap().did();
        let missing_key = missing.repo_key().to_owned();
        {
            let tonk = state.read().await;
            let matching = crate::router::account::tests_matching_request(&tonk).await;
            crate::router::account::persist_link(&tonk, &matching)
                .await
                .unwrap();
            tonk.profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .save(root.did().as_str().as_bytes().to_vec())
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
            assert_eq!(response.name, "shared-name");
        }
        // The rename projected to the spaces it could reach, and the one
        // it could not is left for the sweep rather than failing the rest.
        let member_name = |names: Vec<tonk_schema::MemberName>| {
            names
                .into_iter()
                .map(|row| row.name.0)
                .find(|name| name == "shared-name")
        };
        for key in [&key_a, &key_c] {
            assert_eq!(
                member_name(crate::router::tests::content_member_names(&state, key).await)
                    .as_deref(),
                Some("shared-name"),
                "the rename projects to a mounted space"
            );
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

            // The sweep's catch-up reaches the space the rename could not.
            converge_account_state(&tonk).await.unwrap();
            let caught_up = tonk
                .reactor
                .repository(&missing_key)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            assert!(
                caught_up.is_some(),
                "the catch-up writes the name into the space it reached"
            );

            // Idempotent: a second pass finds nothing stale and writes nothing.
            converge_account_state(&tonk).await.unwrap();
            let settled = tonk
                .reactor
                .repository(&missing_key)
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&tonk.operator)
                .await
                .unwrap()
                .handle()
                .revision();
            assert_eq!(caught_up, settled, "a converged space receives no write");

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
        Ed25519Signer,
        String,
    ) {
        linked_account_state(passkey, true).await
    }

    /// A linked account against a real access service. `activated`
    /// decides how far the customer got: `true` is the state every
    /// steady-state test wants — the emailed link opened, the trusted
    /// marker in place — while `false` stops at `Registered`, which is
    /// the state a browser that just signed up is actually in: enrolled,
    /// refused by the gate, and never yet hydrated.
    #[cfg(not(target_arch = "wasm32"))]
    async fn linked_account_state(
        passkey: Option<tonk_worker_api::PasskeyMetadata>,
        activated: bool,
    ) -> (
        TonkState,
        dialog_common::helpers::Service<
            tonk_access_service::helpers::AccessServiceAddress,
            tonk_access_service::helpers::AccessServer,
        >,
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
            retiring: std::sync::atomic::AtomicBool::new(false),
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
        if activated {
            service
                .address
                .activate_customer(&root, "worker-account-state@example.com")
                .await
                .unwrap();
        } else {
            service
                .address
                .enroll_customer(&root, "worker-account-state@example.com")
                .await
                .unwrap();
        }
        // The endpoint the account syncs against, not the service root:
        // the remote is the address a link names, and that is `/ucan/`.
        let remote = format!(
            "{}/ucan/",
            service.address.access_service_url.trim_end_matches('/')
        );
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
                encryption_key: None,
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
                remote: remote.clone(),
                initialize_name: false,
            },
        )
        .await
        .unwrap();
        // The marker only exists once a hydration succeeded, and none
        // can while the customer is still `Registered`: pre-setting it
        // there would put the fixture in a state no real browser reaches.
        if activated {
            state
                .profile
                .credential()
                .site(tonk_account::TRUSTED_BASE_CREDENTIAL_SITE)
                .save(root_signer.did().as_str().as_bytes().to_vec())
                .perform(&state.operator)
                .await
                .unwrap();
        }

        (state, service, root_signer, remote)
    }

    /// Every passkey recorded for the ready account, through its envelopes.
    #[cfg(not(target_arch = "wasm32"))]
    async fn recorded_passkey_facts(
        state: &TonkState,
        ready: &ReadyAccountBranch,
    ) -> Vec<tonk_schema::RecoveryPasskey> {
        read_passkeys(state, ready).await.unwrap()
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

    /// Activation writes the fact the bar subscribes to.
    ///
    /// `account/active` resolves only when both `activated-at` and
    /// `provider-address` are present, and the provider comes from the
    /// service's receipt. A device that confirmed elsewhere learns it from
    /// the status probe rather than from the activation page, so this pins
    /// the write itself rather than either caller.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_records_the_activation_the_bar_subscribes_to() {
        use tonk_account::customer::CustomerStatus;
        use tonk_schema::{AccountActive, AccountRegistered};

        let (state, service, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let account = super::super::identity::root_did(&state).await.unwrap();

        // Enrollment: registered, and nothing more. The service names no
        // provider until the emailed link is opened.
        super::super::customer::record_customer_status(
            &state,
            CustomerStatus::Registered,
            "person@example.com",
            None,
        )
        .await
        .unwrap();

        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let registered: Vec<AccountRegistered> = branch
            .handle()
            .query()
            .select(Query::<AccountRegistered> {
                this: Term::from(account.this()),
                registered_at: Term::var("registered_at"),
                email: Term::var("email"),
                provider: Term::var("provider"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(registered.len(), 1, "enrollment records the registration");

        let active: Vec<AccountActive> = branch
            .handle()
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.this()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert!(active.is_empty(), "an enrolled account is not yet served");

        // Activation, as the receipt reports it.
        super::super::customer::record_customer_status(
            &state,
            CustomerStatus::Active,
            "person@example.com",
            Some("http://localhost:8080/ucan/"),
        )
        .await
        .unwrap();

        let active: Vec<AccountActive> = branch
            .handle()
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.this()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(active.len(), 1, "activation records the served fact");
        assert!(active[0].activated_at.0 > 0, "carrying when it happened");

        // Where the account syncs rides on the REGISTRATION: it is known at
        // enrollment and unchanged by activation, so a client attaches its
        // remote immediately and learns it was activated from the gate
        // answering 200 instead of 403 — not from asking a status endpoint.
        let registered: Vec<AccountRegistered> = branch
            .handle()
            .query()
            .select(Query::<AccountRegistered> {
                this: Term::from(account.this()),
                registered_at: Term::var("registered_at"),
                email: Term::var("email"),
                provider: Term::var("provider"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(
            registered[0].provider(),
            "http://localhost:8080/ucan/",
            "the registration names where to sync"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// The account db's record of a provisioned space, round-tripped:
    /// recording a provider makes the `SpaceProvider` fact present —
    /// what lets a share skip `/provider/add` — and retraction (what
    /// the sync engine does when the gate stops serving the subject)
    /// returns the space to local-only. Both write the ACCOUNT's did as
    /// the value, so every replica asserts the identical fact.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_records_and_retracts_the_space_provider() {
        use tonk_schema::SpaceProvider;

        let (state, service, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let account = super::super::identity::root_did(&state).await.unwrap();
        let space: dialog_varsig::Did = "did:key:z6MknYwGXCDLuJnBUR4bbWFPiD2Saos16CHQQ2ex6U1Ti2t"
            .parse()
            .unwrap();

        async fn recorded(
            state: &crate::worker::TonkState,
            space: &dialog_varsig::Did,
        ) -> Vec<SpaceProvider> {
            let branch = state
                .reactor
                .profile_repository()
                .branch(tonk_account::MAIN_BRANCH)
                .acquire(&state.operator)
                .await
                .unwrap();
            branch
                .handle()
                .query()
                .select(Query::<SpaceProvider> {
                    this: Term::from(space.this()),
                    provider: Term::var("provider"),
                })
                .perform(&state.operator)
                .try_vec()
                .await
                .unwrap()
        }

        assert!(
            recorded(&state, &space).await.is_empty(),
            "a fresh space records no provider"
        );

        super::super::customer::record_space_provider(&state, &space).await;
        let rows = recorded(&state, &space).await;
        assert_eq!(rows.len(), 1, "provisioning records the provider");
        assert_eq!(
            rows[0].provider.0,
            account.this(),
            "the value is the providing account"
        );

        // Re-recording converges rather than accumulating: the value is
        // the account did, identical from every writer.
        super::super::customer::record_space_provider(&state, &space).await;
        assert_eq!(recorded(&state, &space).await.len(), 1);

        super::super::customer::retract_space_provider(&state, &space).await;
        assert!(
            recorded(&state, &space).await.is_empty(),
            "retraction returns the space to local-only"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// The registering browser's whole wait, end to end: enrolled and
    /// refused, activated somewhere else, noticed at the FIRST sweep the
    /// gate serves. Every other test here starts `Active` with the
    /// trusted marker pre-set, so none of them ran this lifecycle — and
    /// the live flow broke exactly inside it: a browser that had never
    /// hydrated sat on "awaiting confirmation" watching its pulls turn
    /// from 403 to 200 with nothing recording the transition.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_notices_activation_at_the_first_served_sweep() {
        use tonk_account::customer::CustomerStatus;
        use tonk_schema::AccountActive;

        let (state, service, _root, remote) = linked_account_state(None, false).await;
        // What enrollment records on the device: registered, and where
        // the account will sync — the receipt names the provider now.
        super::super::customer::record_customer_status(
            &state,
            CustomerStatus::Registered,
            "worker-account-state@example.com",
            Some(&remote),
        )
        .await
        .unwrap();

        // While the customer is `Registered` the gate refuses the
        // sweep, and nothing may claim the account is served.
        let (status, swept) = ensure_account_state_swept(&state).await;
        assert!(
            status != AccountStateStatus::Ready || swept.is_err(),
            "the gate refuses while awaiting activation, got {status:?} / {swept:?}"
        );
        let account = super::super::identity::root_did(&state).await.unwrap();
        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let read_active = || async {
            let rows: Vec<AccountActive> = branch
                .handle()
                .query()
                .select(Query::<AccountActive> {
                    this: Term::from(account.this()),
                    activated_at: Term::var("activated_at"),
                })
                .perform(&state.operator)
                .try_vec()
                .await
                .unwrap();
            rows
        };
        assert!(
            read_active().await.is_empty(),
            "no sweep may record activation the gate has not granted"
        );

        // The emailed link is opened somewhere this browser cannot see.
        service
            .address
            .confirm_email("worker-account-state@example.com")
            .await
            .unwrap();

        // The next sweep is served, and being served IS the signal: it
        // must both hydrate and record the fact the ceremony waits on,
        // in this same pass — not on some later one.
        let (status, swept) = ensure_account_state_swept(&state).await;
        assert_eq!(
            status,
            AccountStateStatus::Ready,
            "the first served sweep hydrates"
        );
        swept.unwrap();
        assert_eq!(
            read_active().await.len(),
            1,
            "and records the activation the confirm row waits on"
        );
        assert!(
            matches!(
                super::super::customer::registration(&state).await,
                super::super::customer::Registration::Served { .. }
            ),
            "the registration reads served"
        );

        let ready = require_ready_account_state(&state).await.unwrap();
        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// A sweep of an ALREADY-READY account records the activation.
    ///
    /// The browser that enrolled holds the trusted marker from the moment
    /// it created the account, so every later sweep takes the ready arm
    /// and never the first-hydration one. Activation was recorded on the
    /// hydrate arm alone, so that browser's "awaiting confirmation" row
    /// waited on a fact nothing there would ever write: opening the
    /// emailed link changed every other device and left the screen that
    /// sent you there saying "awaiting confirmation" forever.
    ///
    /// The pull inside the sweep is the signal. It goes through only
    /// because the gate served this account, so a sweep that completes IS
    /// the 403 -> 200 transition, and that is what writes `activated-at`.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_records_the_activation_when_a_ready_account_syncs() {
        use tonk_schema::AccountActive;

        let (state, service, _root, _remote) = ready_account_state(None).await;
        // The first sweep hydrates. The second is the one under test: the
        // marker matches now, which is the enrolling browser's every sweep.
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let account = super::super::identity::root_did(&state).await.unwrap();

        // Enrolled and not yet confirmed: the only state with a
        // transition to observe, and the state the browser that just
        // signed up is actually in.
        super::super::customer::record_customer_status(
            &state,
            tonk_account::customer::CustomerStatus::Registered,
            "person@example.com",
            None,
        )
        .await
        .unwrap();

        // Clear what the hydrating sweep recorded, so what is asserted
        // below can only have come from the ready arm.
        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let recorded: Vec<AccountActive> = branch
            .handle()
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.this()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        for row in recorded {
            branch
                .handle()
                .transaction()
                .retract(row)
                .commit()
                .perform(&state.operator)
                .await
                .unwrap();
        }
        let cleared: Vec<AccountActive> = branch
            .handle()
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.this()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert!(cleared.is_empty(), "nothing says served going in");

        let (status, swept) = ensure_account_state_swept(&state).await;
        assert_eq!(status, AccountStateStatus::Ready, "the marker matches");
        swept.unwrap();

        let active: Vec<AccountActive> = branch
            .handle()
            .query()
            .select(Query::<AccountActive> {
                this: Term::from(account.this()),
                activated_at: Term::var("activated_at"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(
            active.len(),
            1,
            "a served sync records the fact the confirm row waits on"
        );
        assert!(active[0].activated_at.0 > 0, "carrying when it happened");

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// An account that is already active is not written again.
    ///
    /// The sweep runs on a heartbeat, so an unconditional assert would
    /// commit a cardinality-one row it already holds every time round --
    /// a transaction, a branch head and a push, forever, to record
    /// something that cannot change again. Activation is one-way: once
    /// the fact is there nobody is waiting on it, so there is nothing
    /// left to watch for.
    ///
    /// Pinned on the timestamp, which is what a rewrite would move.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_stops_watching_once_the_account_is_active() {
        use tonk_schema::AccountActive;

        let (state, service, _root, remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let account = super::super::identity::root_did(&state).await.unwrap();

        // Active, and recorded a while ago. The provider is the fixture's
        // own service: naming another address would repoint the upstream
        // and the sweeps below would fail on the network rather than on
        // what they are here to observe.
        super::super::customer::record_customer_status(
            &state,
            tonk_account::customer::CustomerStatus::Active,
            "person@example.com",
            Some(&remote),
        )
        .await
        .unwrap();

        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let read = || async {
            let rows: Vec<AccountActive> = branch
                .handle()
                .query()
                .select(Query::<AccountActive> {
                    this: Term::from(account.this()),
                    activated_at: Term::var("activated_at"),
                })
                .perform(&state.operator)
                .try_vec()
                .await
                .unwrap();
            rows
        };
        let before = read().await;
        assert_eq!(before.len(), 1, "active going in");

        // Two more heartbeats.
        for _ in 0..2 {
            let (status, swept) = ensure_account_state_swept(&state).await;
            assert_eq!(status, AccountStateStatus::Ready);
            swept.unwrap();
        }

        let after = read().await;
        assert_eq!(after.len(), 1, "still one row");
        assert_eq!(
            after[0].activated_at.0, before[0].activated_at.0,
            "the sweep left the activation alone rather than restamping it"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// A passkey's own row lands on the custody DID its envelope is
    /// addressed to, and is reachable from the account through that
    /// envelope's sender.
    ///
    /// This replaces two tests that pinned a sweep reading the local root:
    /// the row is keyed per passkey now, and only the ceremony holds both
    /// the custody DID and the creation label, so the sweep had nothing to
    /// key on and was retired.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_records_a_passkey_against_the_custody_its_envelope_names() {
        use dialog_varsig::Principal as _;

        let (state, service, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        assert!(
            recorded_passkey_facts(&state, &ready).await.is_empty(),
            "nothing is recorded before a ceremony runs"
        );

        let custody = dialog_credentials::Ed25519Signer::import(&[21u8; 32])
            .await
            .unwrap()
            .did();
        super::super::customer::record_custody_cell(
            &state,
            custody.as_ref(),
            &hex::encode([9u8; 16]),
            Some(tonk_worker_api::PasskeyMetadata {
                created_at: 1_754_380_800,
                created_on: "Chrome on macOS".to_string(),
            }),
            "credential-one",
            Some("person@example.com"),
        )
        .await
        .unwrap();

        let recorded = recorded_passkey_facts(&state, &ready).await;
        assert_eq!(recorded.len(), 1, "one passkey, found through its envelope");
        assert_eq!(recorded[0].this, custody.this(), "keyed on the custody");
        assert_eq!(recorded[0].credential_id.0, "credential-one");
        assert_eq!(recorded[0].seconds(), 1_754_380_800);
        assert_eq!(recorded[0].created_on.0, "Chrome on macOS");

        // A second passkey is a second row rather than a merge — what the
        // account-keyed shape could not do.
        let other = dialog_credentials::Ed25519Signer::import(&[22u8; 32])
            .await
            .unwrap()
            .did();
        super::super::customer::record_custody_cell(
            &state,
            other.as_ref(),
            &hex::encode([8u8; 16]),
            Some(tonk_worker_api::PasskeyMetadata {
                created_at: 1_754_380_900,
                created_on: "Safari on iOS".to_string(),
            }),
            "credential-two",
            Some("person@example.com"),
        )
        .await
        .unwrap();

        let recorded = recorded_passkey_facts(&state, &ready).await;
        assert_eq!(recorded.len(), 2, "two passkeys are two rows");
        assert_eq!(
            recorded[0].created_on.0, "Safari on iOS",
            "newest first, each keeping its own clock and label"
        );

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// The recipient a ceremony recorded on the local root is published
    /// as the account's encryption key, and a seed sealed to it lands as
    /// a `SecretMessage` that the account's own key opens.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_publishes_the_encryption_key_and_custodies_a_seed() {
        use tonk_identity::envelope::AccountSecret;
        use tonk_identity::sealed::Sealed;

        let (state, service, root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();

        // A device that only linked recorded no recipient: nothing to seed.
        assert!(!seed_sealed_inbox(&state).await);
        assert_eq!(read_sealed_inbox(&state, &ready).await.unwrap(), None);
        let subject: dialog_varsig::Did = "did:key:z6MkSpaceUnderTest".parse().unwrap();
        assert!(!custody_seed(&state, &subject, SeedKind::Space, Zeroizing::new([7u8; 32])).await);

        // A ceremony that held the secret re-saves the root with the recipient.
        let account = AccountSecret::from_bytes(Zeroizing::new([5u8; 32]));
        let recipient = account.secret().did();
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &state.profile.did())
                .await
                .unwrap();
        crate::router::identity::persist_root(
            &state,
            tonk_worker_api::SaveRootRequest {
                credential_id: "credential".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().unwrap()),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();
        assert!(seed_sealed_inbox(&state).await);
        assert!(!seed_sealed_inbox(&state).await, "already published");
        assert_eq!(
            read_sealed_inbox(&state, &ready).await.unwrap(),
            Some(recipient.clone())
        );

        assert!(custody_seed(&state, &subject, SeedKind::Space, Zeroizing::new([7u8; 32])).await);
        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        // The principal names the message; the message carries the seed.
        let principals: Vec<SecretPrincipal> = branch
            .handle()
            .query()
            .select(Query::<SecretPrincipal> {
                this: Term::from(subject.this()),
                kind: Term::var("kind"),
                seed: Term::var("seed"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(principals.len(), 1);
        assert_eq!(principals[0].kind.0.to_string(), SeedKind::SPACE);

        let rows: Vec<SecretMessage> = branch
            .handle()
            .query()
            .select(Query::<SecretMessage> {
                this: Term::from(principals[0].seed.0.clone()),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&state.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "the principal names a real message");
        assert_eq!(rows[0].to.0, recipient.this());
        let sealed = Sealed::decode(&rows[0].message.0).unwrap();
        let opened = account.secret().reveal(&sealed, &subject).unwrap();
        assert_eq!(*opened, [7u8; 32]);

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    /// The sweep describes this device's own link in the account space,
    /// so its row is where every device's list reads — including a list
    /// rendered on this device before anything else replicates.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_describes_this_device_on_the_account_sweep() {
        let (state, service, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();

        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .expect("account branch opens");
        let links = tonk_schema::device_link::device_links(branch.handle(), &state.operator)
            .await
            .expect("device-link query runs");
        assert_eq!(links.len(), 1, "exactly this device's row: {links:?}");
        assert_eq!(
            links[0].1,
            state.profile.did().to_string(),
            "the row names this device"
        );

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

        let (state, service, _root, _remote) = ready_account_state(None).await;
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

    /// The ledger read grant a registration receipt carries is retained
    /// as a delegation, not kept as the hex string it travelled as.
    ///
    /// The service mints `ledger -> account` for `/use/get` and names it
    /// in the receipt. Retaining is what makes it usable and what makes
    /// it reach a second device: dialog decomposes the proof onto its
    /// own entity on the account branch, so the authority syncs as facts
    /// rather than sitting in a blob this device alone can read.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_retains_the_ledger_read_grant_a_receipt_carries() {
        use dialog_ucan::{Parameters, Scope};
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::command::Command;
        use dialog_ucan_core::subject::Subject as UcanSubject;
        use dialog_varsig::Principal as _;

        let (state, service, _root, _remote) = ready_account_state(None).await;
        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        let root = super::super::identity::local_root(&state).await.unwrap();

        // The shape `Registration::ledger` mints: the ledger space
        // grants the account every read over itself, and nothing else.
        let ledger = dialog_credentials::Ed25519Signer::import(&[11u8; 32])
            .await
            .unwrap();
        let subject = ledger.did();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(ledger))
            .audience(&root.root_did)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec!["use".to_string(), "get".to_string()])
            .try_build()
            .await
            .unwrap();
        let read_hex = hex::encode(
            dialog_ucan_core::DelegationChain::new(delegation)
                .to_bytes()
                .unwrap(),
        );

        let receipt = tonk_account::customer::Receipt {
            customer: root.root_did.clone(),
            status: tonk_account::customer::CustomerStatus::Active,
            provider: None,
            ledger: Some(tonk_account::customer::Ledger {
                did: subject.clone(),
                read_hex,
            }),
        };
        super::super::customer::retain_ledger(&state, &receipt).await;

        let branch = state
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&state.operator)
            .await
            .unwrap();
        let proven = branch
            .handle()
            .delegations()
            .prove(
                root.root_did.clone(),
                Scope {
                    subject: UcanSubject::Specific(subject.clone()),
                    command: Command::parse("/use/get").unwrap(),
                    parameters: Parameters::default(),
                },
            )
            .perform(&state.operator)
            .await;
        assert!(
            proven.is_ok(),
            "the retained ledger grant must prove the account may read it"
        );

        // A receipt naming no ledger leaves the account branch alone
        // rather than failing: every enrollment receipt predating the
        // field is one.
        let bare = tonk_account::customer::Receipt {
            customer: root.root_did.clone(),
            status: tonk_account::customer::CustomerStatus::Active,
            provider: None,
            ledger: None,
        };
        super::super::customer::retain_ledger(&state, &bare).await;

        service.stop().await.unwrap();
        discard(state, &ready.key);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_seeds_nothing_when_the_local_root_has_no_passkey_metadata() {
        let (state, service, _root, _remote) = ready_account_state(None).await;

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
        let (state, service, root, _remote) = ready_account_state(None).await;

        let renamed = rename_display_name(&state, "linked-name")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name, "linked-name");

        assert_eq!(
            ensure_account_state(&state).await,
            AccountStateStatus::Ready
        );
        let ready = require_ready_account_state(&state).await.unwrap();
        assert_eq!(ready.subject, root.did());
        assert!(
            !state.reactor.repos().read().contains_key(&ready.key),
            "the account key routes the sweep; no repository — and no \
             database — exists behind it",
        );

        let initialized = initialize_display_name(&state).await.unwrap();
        assert_eq!(
            initialized.name, "linked-name",
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

    mod observing_registration {
        use super::super::observed_status;
        use dialog_capability::access::{AuthorizeError, Recourse};
        use tonk_account::customer::CustomerStatus;

        fn declined(recourse: Recourse) -> AuthorizeError {
            AuthorizeError::Declined {
                recourse,
                reason: "the subject's own registration awaits email activation".into(),
            }
        }

        /// Activation is the refusal clearing, which is the whole reason
        /// nothing polls: the sync that was being refused starts working
        /// and that IS the signal.
        #[dialog_common::test]
        fn it_reads_a_served_push_as_activation_when_one_was_pending() {
            assert_eq!(
                observed_status(None, true),
                Some(CustomerStatus::Active),
                "a push that succeeds after a wait means the email was confirmed"
            );
        }

        /// A push succeeds constantly for accounts that never registered
        /// anything. Writing `Active` from those would invent a
        /// registration and, worse, tell the UI an unregistered account
        /// syncs.
        #[dialog_common::test]
        fn it_reads_nothing_from_a_push_that_was_never_being_refused() {
            assert_eq!(observed_status(None, false), None);
        }

        /// Both directions, not only the clearing one. An account
        /// suspended after it was active is refused where it used to be
        /// served, and a status that only moved forward would leave
        /// every device believing it still syncs.
        #[dialog_common::test]
        fn it_reads_a_refusal_in_either_direction() {
            assert_eq!(
                observed_status(Some(&declined(Recourse::Retry)), false),
                Some(CustomerStatus::Registered)
            );
            assert_eq!(
                observed_status(Some(&declined(Recourse::None)), true),
                Some(CustomerStatus::Suspended)
            );
        }

        /// A refusal about the proof says nothing about registration.
        /// Recording one would overwrite a real status with a guess
        /// drawn from an unrelated failure.
        #[dialog_common::test]
        fn it_reads_nothing_from_a_refusal_about_the_proof() {
            for unrelated in [
                AuthorizeError::Revoked {
                    subject: "did:key:z6MkrCD1csqtgdj8sRHYRPGLYcMFXAoDhkgvHNq2FML2xqCX"
                        .parse()
                        .expect("a valid did"),
                },
                AuthorizeError::Expired {
                    expiration: 1,
                    at: 2,
                },
                AuthorizeError::Unavailable {
                    detail: "offline".into(),
                },
            ] {
                assert_eq!(
                    observed_status(Some(&unrelated), true),
                    None,
                    "{unrelated} must not be read as a registration state"
                );
            }
        }
    }
}
