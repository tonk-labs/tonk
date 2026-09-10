//! Lazy, directory-driven space adoption — the account DB as the sole
//! source of truth for which spaces exist and how to mount them.
//!
//! The account DB carries, per space, plain facts on directory-anchored
//! entities (see `repository::record_space_mount`): the [`Space`] row,
//! a [`SpaceName`] mirror, and the full remote/branch configuration as
//! [`Remote`] / [`RemoteExecution`] / [`Branch`] / [`TrackingBranch`]
//! concepts. Nothing mounts eagerly: the Hub renders straight from the
//! directory, and [`ensure_space_mounted`] replicates a space on first
//! use — the data-plane routes call it when a request names a repo this
//! device has not mounted.
//!
//! Already-mounted replicas are reconciled too. First use and every
//! successful account sweep re-read the latest mount facts, while a ready,
//! served account additionally adopts genuinely local-only repositories by
//! provisioning and attaching its provider.
//!
//! Deletion is a retraction: adoption reads asserted rows only, so a
//! removed space is simply absent — no escrow, no backfill, nothing to
//! resurrect it.
//!
//! [`Space`]: tonk_schema::Space
//! [`SpaceName`]: tonk_schema::SpaceName
//! [`Remote`]: tonk_schema::Remote
//! [`RemoteExecution`]: tonk_schema::RemoteExecution
//! [`Branch`]: tonk_schema::Branch
//! [`TrackingBranch`]: tonk_schema::TrackingBranch

pub(crate) mod cache;

use dialog_repository::RepositoryExt as _;
use tonk_common::log;

use super::repository::{
    BranchConfiguration, RemoteConfiguration, RepositoryConfiguration, UpstreamConfiguration,
};
use crate::worker::TonkState;

/// Mount `key`'s space from the account directory if this device lacks
/// it and the directory records how. Returns whether the space is
/// mounted (already or just now); `Ok(false)` means the directory has
/// no mountable record for it — the caller proceeds and fails with its
/// ordinary not-found.
pub(crate) async fn ensure_space_mounted(
    tonk: &TonkState,
    key: &str,
) -> Result<bool, crate::TonkWorkerError> {
    // Routes address a space by either spelling: the bare routing key
    // (the DID's method-specific suffix) or the full did:key URI —
    // pages query with the full form. Normalize before parsing, or the
    // full form silently fails the parse and adoption never fires.
    let suffix = key.strip_prefix("did:key:").unwrap_or(key);
    let subject = match space_subject(key) {
        Some(did) => did,
        None => return Ok(false), // not a space key (e.g. a named repo)
    };
    if super::account_state::is_account_key(tonk, key).await
        || super::account_state::is_account_key(tonk, suffix).await
    {
        return Ok(false);
    }
    let key = subject.as_str();
    let entry = tonk.admission.entry(key);
    if entry.valid(tonk, key) {
        return Ok(true);
    }
    let _slow = entry.slow.lock().await;
    if entry.valid(tonk, key) {
        return Ok(true);
    }
    entry.forget();
    #[cfg(test)]
    tonk.admission
        .observations
        .slow
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Preserve membership-before-open ordering: looking up an unmounted
    // repository can itself reopen storage after removal.
    let started = entry.start(tonk);
    let local = super::join::find_replica_for_subject(tonk, &subject).await?;
    #[cfg(test)]
    entry.pause_membership_if_requested().await;
    let before = if local {
        let _ = tonk
            .reactor
            .repository(key)
            .branch(super::repository::META_BRANCH)
            .acquire(&tonk.operator)
            .await;
        entry
            .stamp(tonk, key)
            .filter(|stamp| stamp.started_at(started.as_ref()))
    } else {
        None
    };
    #[cfg(test)]
    entry.pause_if_requested().await;
    if local {
        match reconcile_mounted_configuration(tonk, key, &subject).await {
            Ok(configuration) => {
                let upstreams = configuration
                    .into_iter()
                    .flat_map(|configuration| configuration.branch)
                    .filter_map(|(name, branch)| {
                        branch
                            .upstream
                            .map(|upstream| (name, upstream.remote, upstream.branch))
                    })
                    .collect();
                entry.install(tonk, key, before, upstreams);
            }
            Err(error) => {
                log!("space adoption: directory reconcile for mounted '{subject}': {error}")
            }
        }
        // Seed catch-up is NOT run here: it fetches the seed source over
        // HTTP before its version compare can short-circuit, and this
        // path sits on every data-plane request. The routes that mount
        // call [`schedule_seed_upgrade`], which runs it detached, once
        // per worker instance per space — a new worker is a new bundle,
        // which is exactly when a shipped redesign can have appeared.
        return Ok(true);
    }
    let Some(configuration) = directory_configuration_strict(tonk, &subject).await? else {
        return Ok(false);
    };
    log!("space adoption: mounting '{subject}' from the account directory");

    // Announce the pull BEFORE it starts, so the state is observable
    // however the mount was triggered — the lazy first-use path included,
    // not just an explicit `space/replicate`. Without this a large pull
    // leaves the row looking remote until it abruptly becomes local.
    stamp_space_replicating(tonk, &subject, true).await;

    let mounted = mount_and_record(tonk, &subject, configuration).await;

    // Settled either way: on failure the marker must go too, or the row
    // stays "replicating" with no pull behind it.
    stamp_space_replicating(tonk, &subject, false).await;
    mounted?;

    stamp_space_locality(tonk, &subject).await;
    // The mount wires the upstream but the content arrives over a pull;
    // mark the repo dirty so the next drain (the page's own follow-up
    // requests trigger one) fills the space in promptly instead of
    // waiting for an idle beat.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue
        .mark_dirty(subject.as_str(), js_sys::Date::now());
    Ok(true)
}

/// Mount the replica and record it, as one fallible step.
///
/// Split out so the in-flight marker above can be retracted on the way
/// out whichever way this goes — a `?` in the caller would skip the
/// retraction and strand the row.
async fn mount_and_record(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
    configuration: crate::router::repository::RepositoryConfiguration,
) -> Result<(), crate::TonkWorkerError> {
    super::join::mount_replica_with_configuration(tonk, subject, configuration).await?;
    super::repository::record_initialized_replica_in_profile(tonk, subject)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("record adopted space '{subject}': {error}"))
        })
}

/// Pull a space this account has but this device does not.
///
/// The work is [`ensure_space_mounted`]'s — the same routine the lazy
/// first-use path runs, so an explicit request and an implicit one take
/// exactly one code path and report the same state. What the command
/// adds is the ability to ASK, rather than tripping replication as a
/// side effect of some unrelated query.
///
/// Target-agnostic: every host that can mount a space can run this, and
/// the mount itself is already portable.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::ReplicateSpace>
    for crate::router::CommandEnv
{
    async fn execute(&self, command: tonk_schema::command::ReplicateSpace) {
        let key = command.space.0.to_string();
        // A space may only ask for ITSELF; the profile may ask for any.
        if !self.may_target_space(&key) {
            log!("ReplicateSpace '{key}': refused from another space's branch");
            return;
        }
        let tonk = self.state().read().await;
        match ensure_space_mounted(&tonk, &key).await {
            Ok(true) => {
                schedule_seed_upgrade(&tonk, self.state().clone(), &key).await;
                log!("ReplicateSpace '{key}': mounted");
            }
            // Not an error: the directory has no mount record for it, so
            // there is nothing this device could pull.
            Ok(false) => log!("ReplicateSpace '{key}': nothing to mount"),
            Err(error) => log!("ReplicateSpace '{key}': {error}"),
        }
    }
}

/// Spaces whose seed this worker instance has already checked, by full
/// subject DID. In memory on purpose: a worker instance corresponds to
/// one shipped bundle, so once-per-instance is once-per-bundle for any
/// space that gets used — an upgrade lands with the SW upgrade rather
/// than being re-verified on every load.
#[derive(Default)]
pub(crate) struct SeedUpgrades(std::sync::Mutex<std::collections::HashSet<String>>);

impl SeedUpgrades {
    /// Claim the once-per-instance check for `key`. `false` means some
    /// earlier request already claimed it.
    fn begin(&self, key: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.to_string())
    }
}

/// Catch a mounted space's seed up with the shipped bundle, detached
/// from the request that touched it.
///
/// Best-effort by design: the work runs off the request path (a load is
/// never blocked on the seed source fetch), at most once per worker
/// instance per space, and a failed attempt simply waits for the next
/// worker to try again. On wasm the task is not tied to the fetch
/// lifetime, so an idling worker may cut it short — the same next-boot
/// retry covers that. Native (the single-threaded test and host builds,
/// same as `spawn_dispatch`) runs it inline instead.
pub(crate) async fn schedule_seed_upgrade(
    tonk: &TonkState,
    state: crate::router::AppState,
    key: &str,
) {
    let Some(subject) = space_subject(key) else {
        return;
    };
    let key = subject.to_string();
    // Claim under the CALLER's guard, before anything is spawned: the
    // detached task re-locks for itself, and taking a second read here
    // while the caller holds one could park behind a queued writer.
    if !tonk.seed_upgrades.begin(&key) {
        return;
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_futures::spawn_local(async move {
        let tonk = state.read().await;
        match super::repository::upgrade_seed(&tonk, &key).await {
            Ok(true) => log!("seed upgrade: '{key}' caught up with the shipped bundle"),
            Ok(false) => {}
            Err(error) => log!("seed upgrade for mounted '{key}': {error}"),
        }
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = state;
        match super::repository::upgrade_seed(tonk, &key).await {
            Ok(true) => log!("seed upgrade: '{key}' caught up with the shipped bundle"),
            Ok(false) => {}
            Err(error) => log!("seed upgrade for mounted '{key}': {error}"),
        }
    }
}

/// Parse either the canonical full repository key or the legacy bare suffix.
fn space_subject(key: &str) -> Option<dialog_varsig::Did> {
    if key.starts_with("did:key:") {
        key.parse().ok()
    } else {
        format!("did:key:{key}").parse().ok()
    }
}

/// Apply the account directory's latest mount facts to one replica that is
/// already present locally. Returns whether a mount record existed.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn reconcile_mounted_space_from_directory(
    tonk: &TonkState,
    key: &str,
    subject: &dialog_varsig::Did,
) -> Result<bool, crate::TonkWorkerError> {
    Ok(reconcile_mounted_configuration(tonk, key, subject)
        .await?
        .is_some())
}

async fn reconcile_mounted_configuration(
    tonk: &TonkState,
    key: &str,
    subject: &dialog_varsig::Did,
) -> Result<Option<RepositoryConfiguration>, crate::TonkWorkerError> {
    let Some(configuration) = directory_configuration_strict(tonk, subject).await? else {
        return Ok(None);
    };
    let repository = tonk
        .profile
        .repository(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!(
                "load mounted space '{subject}' for directory reconcile: {error}"
            ))
        })?;
    if mounted_configuration_is_current(tonk, key, &repository, &configuration).await? {
        return Ok(Some(configuration));
    }
    super::repository::ensure_remote_config(tonk, &repository, key, &configuration)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!(
                "reconcile mounted space '{subject}' from directory: {error}"
            ))
        })?;
    Ok(Some(configuration))
}

/// Check both durable replica meta and the reactor's cached branch handles.
/// The latter matters because sync reads the cache: a durable tracking fact
/// with a stale cached `None` is exactly the `BranchHasNoUpstream` state this
/// reconciliation repairs.
/// Only the durable facts used by admission. Presentation and content do not
/// participate in deciding whether mount configuration needs repair.
struct MountedConfiguration {
    remotes: std::collections::HashSet<String>,
    tracking: std::collections::HashMap<String, UpstreamConfiguration>,
}

impl MountedConfiguration {
    async fn read<C: dialog_varsig::Principal + Clone>(
        tonk: &TonkState,
        repository: &dialog_repository::Repository<C>,
    ) -> Result<Self, crate::TonkWorkerError> {
        use dialog_query::{Output as _, Query, Term};
        use tonk_schema::{Branch, Remote, Replica, TrackingBranch};

        #[cfg(test)]
        tonk.admission
            .observations
            .configuration
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let read_error = |error: String| {
            crate::TonkWorkerError::Internal(format!("read mounted configuration: {error}"))
        };
        let meta = repository
            .branch(super::repository::META_BRANCH)
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| read_error(e.to_string()))?;
        let cached = tonk
            .reactor
            .repos()
            .read()
            .get(repository.did().as_str())
            .cloned();
        if let Some(cached) = cached {
            let cached_meta = cached
                .branches()
                .read()
                .get(super::repository::META_BRANCH)
                .cloned();
            if let Some(cached_meta) = cached_meta
                && cached_meta.branch.revision() != meta.revision()
            {
                cached_meta
                    .branch
                    .refresh(&tonk.operator)
                    .await
                    .map_err(|e| read_error(e.to_string()))?;
            }
        }
        let replica = Replica::new(tonk.profile.did(), repository.did());
        let branches = meta
            .query()
            .select(Query::<Branch> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::var("origin"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|e| read_error(e.to_string()))?;
        let remotes = meta
            .query()
            .select(Query::<Remote> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(replica.this().clone()),
                subject: Term::var("subject"),
                address: Term::var("address"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|e| read_error(e.to_string()))?;
        let links = meta
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::var("this"),
                upstream: Term::var("upstream"),
                origin: Term::from(replica.this().clone()),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|e| read_error(e.to_string()))?;
        let branches_by_entity: std::collections::HashMap<_, _> = branches
            .iter()
            .map(|branch| (&branch.this, branch))
            .collect();
        let remotes_by_entity: std::collections::HashMap<_, _> = remotes
            .iter()
            .map(|remote| (&remote.this, remote))
            .collect();
        let links_by_entity: std::collections::HashMap<_, _> = links
            .iter()
            .map(|link| (&link.this, &link.upstream.0))
            .collect();
        let tracking = branches
            .iter()
            .filter_map(|branch| {
                if branch.origin.0 != *replica.this()
                    || remotes_by_entity.contains_key(&branch.this)
                {
                    return None;
                }
                let target = branches_by_entity.get(links_by_entity.get(&branch.this)?)?;
                let remote = remotes_by_entity.get(&target.origin.0)?;
                Some((
                    branch.name.0.clone(),
                    UpstreamConfiguration::new(remote.name.0.clone(), target.name.0.clone()),
                ))
            })
            .collect();
        Ok(Self {
            remotes: remotes.into_iter().map(|remote| remote.name.0).collect(),
            tracking,
        })
    }
}

async fn mounted_configuration_is_current<C>(
    tonk: &TonkState,
    key: &str,
    repository: &dialog_repository::Repository<C>,
    desired: &RepositoryConfiguration,
) -> Result<bool, crate::TonkWorkerError>
where
    C: dialog_varsig::Principal + Clone,
{
    let current = MountedConfiguration::read(tonk, repository).await?;
    if desired
        .remote
        .keys()
        .any(|name| !current.remotes.contains(name))
    {
        return Ok(false);
    }
    for (branch_name, branch) in &desired.branch {
        let Some(upstream) = &branch.upstream else {
            continue;
        };
        let durable_matches = current.tracking.get(branch_name).is_some_and(|current| {
            current.remote == upstream.remote && current.branch == upstream.branch
        });
        if !durable_matches {
            return Ok(false);
        }
        let Ok(session) = tonk
            .reactor
            .repository(key)
            .branch(branch_name)
            .acquire(&tonk.operator)
            .await
        else {
            return Ok(false);
        };
        if !matches!(
            session.handle().upstream(),
            Some(dialog_repository::Upstream::Remote {
                ref remote,
                ref branch,
                ..
            }) if *remote == upstream.remote && *branch == upstream.branch
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Reconcile every local space after the account branch has pulled its latest
/// facts.
///
/// The directory is authoritative for mount configuration. If it has no mount
/// record and the customer is now served, a repository carrying zero remotes
/// is the one safe ownership shape we automatically provision and attach.
/// Every repository is isolated: malformed or temporarily unavailable state
/// is logged and retried on the next account sweep without blocking siblings.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn reconcile_account_spaces(tonk: &TonkState) {
    use std::collections::HashMap;

    let directory_names: Option<HashMap<String, String>> = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => match tonk_schema::directory::spaces(main.handle(), &tonk.operator).await {
            Ok(rows) => Some(
                rows.into_iter()
                    .filter_map(|row| row.name.map(|name| (row.subject.to_string(), name)))
                    .collect(),
            ),
            Err(error) => {
                log!("space reconcile: read directory names: {error:?}");
                None
            }
        },
        Err(error) => {
            log!("space reconcile: open account directory: {error}");
            None
        }
    };
    let account_remote = if super::customer::is_active(tonk).await {
        match super::account_state::account_remote(tonk).await {
            Ok(remote) => Some(remote),
            Err(error) => {
                log!("space reconcile: active account has no usable remote: {error}");
                None
            }
        }
    } else {
        None
    };

    for key in super::profile_name::real_space_keys(tonk).await {
        let subject = match space_subject(&key) {
            Some(subject) => subject,
            None => {
                log!("space reconcile: invalid repository key '{key}'");
                continue;
            }
        };

        let repository = match tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(repository) => repository,
            Err(error) => {
                log!("space reconcile: load '{subject}': {error}");
                continue;
            }
        };
        if let Some(names) = &directory_names
            && let Some(name) =
                super::repository::repository_display_name(tonk, &repository, &key).await
            && names.get(subject.to_string().as_str()) != Some(&name)
        {
            super::repository::record_space_name(tonk, &subject, &name).await;
        }

        match reconcile_mounted_space_from_directory(tonk, &key, &subject).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                log!("space reconcile: directory configuration for '{subject}': {error}");
                continue;
            }
        }
        let Some(remote) = account_remote.as_deref() else {
            continue;
        };
        match super::repository::attach_account_remote_if_local(tonk, &key, remote).await {
            Ok(true) => {
                log!("space reconcile: attached account remote to local space '{subject}'");
                tonk.sync_queue.mark_dirty(&key, js_sys::Date::now());
            }
            Ok(false) => {}
            Err(error) => log!("space reconcile: attach account remote to '{subject}': {error}"),
        }
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use super::*;

    /// The in-flight marker is retracted even when the pull fails.
    ///
    /// The hazard this pins: the marker is asserted before the mount and
    /// the mount can fail, so a `?` on the way out would leave the row
    /// reading `case:replicating` with no pull behind it — stuck, and
    /// with nothing to clear it, since the marker outlives the attempt
    /// that wrote it. Only a worker restart would drop it.
    #[dialog_common::test]
    async fn it_clears_the_in_flight_marker_when_a_pull_fails() {
        use dialog_credentials::ed25519::Ed25519Signer;
        use dialog_query::{Output as _, Query, Term};
        use dialog_varsig::Principal as _;

        let tonk = crate::router::tests::test_state().await;

        // Directory facts naming a remote that cannot answer, so the
        // mount is attempted and fails rather than being skipped.
        let foreign = Ed25519Signer::generate().await.unwrap();
        let subject: dialog_varsig::Did = foreign.did();
        let address = dialog_repository::SiteAddress::from(
            dialog_remote_ucan_s3::UcanAddress::new("https://unreachable.invalid/ucan/"),
        );
        let configuration = super::super::repository::RepositoryConfiguration::default()
            .remote(
                "origin",
                super::super::repository::RemoteConfiguration::new(address)
                    .subject(subject.clone())
                    .revocation_url("https://relay.example.test/revocations/".parse().unwrap()),
            )
            .branch(
                "main",
                super::super::repository::BranchConfiguration::default().upstream("origin", "main"),
            );
        super::super::repository::record_space_mount(&tonk, &subject, &configuration, None).await;

        // Whether this attempt succeeds or fails is not the point; that
        // no marker survives it is.
        let _ = ensure_space_mounted(&tonk, subject.as_str()).await;

        let main = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("profile main acquires");
        let in_flight: Vec<tonk_schema::SpaceReplicating> = main
            .handle()
            .query()
            .select(Query::<tonk_schema::SpaceReplicating> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                replicating: Term::var("replicating"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("the in-flight query runs");
        assert!(
            in_flight.is_empty(),
            "a settled pull leaves no in-flight marker: {in_flight:?}"
        );
    }

    #[dialog_common::test]
    async fn it_admits_without_content_projections() {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "admission-meta-only").await;
        let tonk = state.read().await;
        let subject = key.parse().unwrap();
        let configuration = RepositoryConfiguration::default().remote(
            "origin",
            RemoteConfiguration::new(dialog_repository::SiteAddress::from(
                dialog_remote_ucan_s3::UcanAddress::new("https://sync.example.test/ucan/"),
            )),
        );
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        super::super::repository::ensure_remote_config(&tonk, &repository, &key, &configuration)
            .await
            .unwrap();
        super::super::repository::record_space_mount(&tonk, &subject, &configuration, None).await;
        tonk.reject_admission_content_reads
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
    }

    #[dialog_common::test]
    async fn it_checks_tracking_and_repairs_a_stale_cached_upstream() {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "admission-tracking").await;
        let tonk = state.read().await;
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        // Open a separate pre-attachment handle to reproduce stale cached None.
        let stale = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let configuration = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(dialog_repository::SiteAddress::from(
                    dialog_remote_ucan_s3::UcanAddress::new("https://sync.example.test/ucan/"),
                )),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        assert!(
            !mounted_configuration_is_current(&tonk, &key, &repository, &configuration)
                .await
                .unwrap(),
            "missing remote needs repair"
        );
        super::super::repository::ensure_remote_config(&tonk, &repository, &key, &configuration)
            .await
            .unwrap();
        assert!(
            mounted_configuration_is_current(&tonk, &key, &repository, &configuration)
                .await
                .unwrap()
        );
        let mismatched = configuration.clone().branch(
            "main",
            BranchConfiguration::default().upstream("origin", "other"),
        );
        assert!(
            !mounted_configuration_is_current(&tonk, &key, &repository, &mismatched)
                .await
                .unwrap(),
            "durable tracking must match"
        );
        let cached = tonk
            .reactor
            .repository(&key)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        cached.branches().write().insert(
            "main".into(),
            std::sync::Arc::new(dialog_reactor::BranchState::new(stale)),
        );
        assert!(
            !mounted_configuration_is_current(&tonk, &key, &repository, &configuration)
                .await
                .unwrap(),
            "durable tracking alone cannot validate stale cached None"
        );
        super::super::repository::record_space_mount(
            &tonk,
            &key.parse().unwrap(),
            &configuration,
            None,
        )
        .await;
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        assert!(
            mounted_configuration_is_current(&tonk, &key, &repository, &configuration)
                .await
                .unwrap(),
            "admission must repair the cached upstream"
        );
    }

    async fn configured_fixture() -> (super::super::AppState, String, RepositoryConfiguration) {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "admission-cache").await;
        let configuration = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(dialog_repository::SiteAddress::from(
                    dialog_remote_ucan_s3::UcanAddress::new("https://sync.example.test/ucan/"),
                )),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        {
            let tonk = state.read().await;
            super::super::repository::record_space_mount(
                &tonk,
                &key.parse().unwrap(),
                &configuration,
                None,
            )
            .await;
            warm(&tonk, &key).await;
        }
        (state, key, configuration)
    }

    async fn warm(tonk: &TonkState, key: &str) {
        for _ in 0..3 {
            assert!(ensure_space_mounted(tonk, key).await.unwrap());
        }
        assert!(
            tonk.admission.entry(key).valid(tonk, key),
            "stable verification installs receipt"
        );
    }

    #[dialog_common::test]
    async fn it_reuses_admission_for_both_spellings_and_content_changes() {
        let (state, key, _) = configured_fixture().await;
        let tonk = state.read().await;
        let before = tonk.admission.observations.counts();
        tonk.reject_admission_content_reads
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            ensure_space_mounted(&tonk, key.strip_prefix("did:key:").unwrap())
                .await
                .unwrap()
        );
        let main = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        main.state
            .assert_overlay(tonk_schema::SpaceLocal::new(&key.parse().unwrap(), true));
        main.handle()
            .transaction()
            .assert(tonk_schema::SpaceName::new(
                &key.parse().unwrap(),
                "changed content",
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        assert_eq!(
            tonk.admission.observations.counts(),
            before,
            "warm admission performs no directory/configuration reads"
        );
    }

    #[dialog_common::test]
    async fn it_invalidates_on_directory_meta_and_upstream_changes() {
        let (state, key, configuration) = configured_fixture().await;
        let tonk = state.read().await;
        let entry = tonk.admission.entry(&key);
        let changed = configuration.branch(
            "main",
            BranchConfiguration::default().upstream("origin", "other"),
        );
        super::super::repository::record_space_mount(&tonk, &key.parse().unwrap(), &changed, None)
            .await;
        assert!(!entry.valid(&tonk, &key));
        warm(&tonk, &key).await;
        let main = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        assert!(
            matches!(main.handle().upstream(), Some(dialog_repository::Upstream::Remote { branch, .. }) if branch == "other")
        );
        tonk.reactor
            .repository(&key)
            .branch(super::super::repository::META_BRANCH)
            .transaction()
            .assert(tonk_schema::SpaceName::new(
                &key.parse().unwrap(),
                "meta changed",
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
        assert!(!entry.valid(&tonk, &key));
        warm(&tonk, &key).await;
        // Replace just the cached content handle with a different upstream;
        // receipt validity includes upstream values, not content revisions.
        let repo = tonk
            .reactor
            .repository(&key)
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let unrelated = repo
            .repository()
            .branch("untracked")
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap();
        repo.branches().write().insert(
            "main".into(),
            std::sync::Arc::new(dialog_reactor::BranchState::new(unrelated)),
        );
        assert!(!entry.valid(&tonk, &key));
    }

    #[dialog_common::test]
    async fn it_retries_directory_failure_and_caches_local_absence() {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "admission-local-only").await;
        let tonk = state.read().await;
        use std::sync::atomic::Ordering::Relaxed;
        tonk.admission
            .observations
            .fail_directory
            .store(true, Relaxed);
        assert!(
            ensure_space_mounted(&tonk, &key).await.unwrap(),
            "known local replica stays readable"
        );
        assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
        let failed = tonk.admission.observations.counts();
        tonk.admission
            .observations
            .fail_directory
            .store(false, Relaxed);
        warm(&tonk, &key).await;
        assert!(tonk.admission.observations.counts().1 > failed.1);
        let before = tonk.admission.observations.counts();
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        assert_eq!(tonk.admission.observations.counts(), before);
    }

    #[dialog_common::test]
    async fn it_invalidates_across_eviction_removal_and_profile_replacement() {
        let (state, key, _) = configured_fixture().await;
        {
            let tonk = state.read().await;
            let before = tonk.admission.observations.counts();
            tonk.reactor.evict(&key);
            assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
            warm(&tonk, &key).await;
            assert!(tonk.admission.observations.counts().0 > before.0);
        }
        super::super::repository::remove_space_inner(&state, &key.parse().unwrap())
            .await
            .unwrap();
        let tonk = state.read().await;
        assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
        // Re-run the existing adoption path, including its storage/authority
        // outcome, rather than returning the removed replica's old receipt.
        let before = tonk.admission.observations.counts();
        let _lookup = ensure_space_mounted(&tonk, &key).await;
        assert!(tonk.admission.observations.counts().0 > before.0);
        assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
        let other = crate::router::tests::test_state().await;
        assert!(!tonk.admission.entry(&key).valid(&other, &key));
        assert!(!other.admission.entry(&key).valid(&other, &key));
    }

    #[dialog_common::test]
    async fn it_invalidates_before_and_after_cancelled_configuration_writes() {
        let (state, key, _) = configured_fixture().await;
        let tonk = state.read().await;
        let entry = tonk.admission.entry(&key);
        let before = entry.stamp(&tonk, &key);
        let guard = tonk.admission.mutation(&key);
        assert!(!entry.valid(&tonk, &key));
        assert!(entry.stamp(&tonk, &key).is_none());
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        assert!(
            !entry.valid(&tonk, &key),
            "in-flight writer blocks receipts"
        );
        drop(guard);
        entry.install(&tonk, &key, before, Vec::new());
        assert!(
            !entry.valid(&tonk, &key),
            "pre-mutation stamp cannot be published"
        );
        warm(&tonk, &key).await;
    }

    fn pause_next(
        tonk: &TonkState,
        key: &str,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let entry = tonk.admission.entry(key);
        entry.forget();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *entry.gate.lock() = Some((entered_tx, release_rx));
        (entered_rx, release_tx)
    }

    #[dialog_common::test]
    async fn it_shares_one_slow_check_among_sixteen_callers() {
        let (state, key, _) = configured_fixture().await;
        let tonk = state.read().await;
        let (entered, release) = pause_next(&tonk, &key);
        let before = tonk.admission.observations.counts();
        let leader = ensure_space_mounted(&tonk, &key);
        let waiters = async {
            entered.await.unwrap();
            let mut waiters: Vec<_> = (0..15)
                .map(|_| Box::pin(ensure_space_mounted(&tonk, &key)))
                .collect();
            for waiter in &mut waiters {
                assert!(futures_util::poll!(waiter.as_mut()).is_pending());
            }
            assert_eq!(tonk.admission.observations.counts().0, before.0 + 1);
            release.send(()).unwrap();
            for result in futures_util::future::join_all(waiters).await {
                assert!(result.unwrap());
            }
        };
        let (result, ()) = futures_util::join!(leader, waiters);
        assert!(result.unwrap());
        let after = tonk.admission.observations.counts();
        assert_eq!(after, (before.0 + 1, before.1 + 1, before.2 + 1));
    }

    #[dialog_common::test]
    async fn it_releases_slow_admission_when_the_leader_is_cancelled() {
        let (state, key, _) = configured_fixture().await;
        let tonk = state.read().await;
        let (entered, release) = pause_next(&tonk, &key);
        match futures_util::future::select(Box::pin(ensure_space_mounted(&tonk, &key)), entered)
            .await
        {
            futures_util::future::Either::Right((entered, leader)) => {
                entered.unwrap();
                drop(leader);
            }
            _ => panic!("leader must suspend at the gate"),
        }
        drop(release);
        assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        assert!(tonk.admission.entry(&key).valid(&tonk, &key));
    }

    #[dialog_common::test]
    async fn it_does_not_publish_a_receipt_after_mid_check_changes() {
        let (state, key, configuration) = configured_fixture().await;
        let tonk = state.read().await;
        for evict in [false, true] {
            warm(&tonk, &key).await;
            let (entered, release) = pause_next(&tonk, &key);
            let leader = ensure_space_mounted(&tonk, &key);
            let mutate = async {
                entered.await.unwrap();
                if evict {
                    tonk.reactor.evict(&key);
                } else {
                    super::super::repository::record_space_mount(
                        &tonk,
                        &key.parse().unwrap(),
                        &configuration,
                        Some("new directory revision"),
                    )
                    .await;
                }
                release.send(()).unwrap();
            };
            let (result, ()) = futures_util::join!(leader, mutate);
            assert!(result.unwrap());
            assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
            warm(&tonk, &key).await;
        }
    }

    #[dialog_common::test]
    async fn it_retries_a_failed_leader_and_other_subjects_progress_independently() {
        let (state, key, _) = configured_fixture().await;
        let (app, _lsp) = super::super::api_router_from_state(state.clone());
        let other = crate::router::tests::put_repo(&app, "independent-admission").await;
        let tonk = state.read().await;
        warm(&tonk, &key).await;
        let (entered, release) = pause_next(&tonk, &key);
        let leader = ensure_space_mounted(&tonk, &key);
        let follower = async {
            entered.await.unwrap();
            assert!(
                ensure_space_mounted(&tonk, &other).await.unwrap(),
                "unrelated subject does not wait for this leader"
            );
            tonk.admission
                .observations
                .fail_directory
                .store(true, std::sync::atomic::Ordering::Relaxed);
            release.send(()).unwrap();
        };
        let (result, ()) = futures_util::join!(leader, follower);
        assert!(
            result.unwrap(),
            "failed reconciliation preserves local availability"
        );
        assert!(!tonk.admission.entry(&key).valid(&tonk, &key));
        tonk.admission
            .observations
            .fail_directory
            .store(false, std::sync::atomic::Ordering::Relaxed);
        warm(&tonk, &key).await;
    }

    #[dialog_common::test]
    async fn it_mounts_once_on_sixteen_concurrent_first_uses() {
        use dialog_varsig::Principal as _;
        let tonk = crate::router::tests::test_state().await;
        let subject = dialog_credentials::ed25519::Ed25519Signer::generate()
            .await
            .unwrap()
            .did();
        let key = subject.as_str();
        let configuration = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(dialog_repository::SiteAddress::from(
                    dialog_remote_ucan_s3::UcanAddress::new("https://sync.example.test/ucan/"),
                ))
                .subject(subject.clone()),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        super::super::repository::record_space_mount(&tonk, &subject, &configuration, None).await;
        let (entered, release) = pause_next(&tonk, key);
        let leader = async {
            assert!(ensure_space_mounted(&tonk, key).await.unwrap());
            let repository: dialog_repository::Repository = tonk
                .profile
                .repository(key)
                .load()
                .perform(&tonk.operator)
                .await
                .unwrap();
            repository
                .branch(super::super::repository::META_BRANCH)
                .open()
                .perform(&tonk.operator)
                .await
                .unwrap()
                .revision()
        };
        let waiters = async {
            entered.await.unwrap();
            let mut waiters: Vec<_> = (0..15)
                .map(|_| Box::pin(ensure_space_mounted(&tonk, key)))
                .collect();
            for waiter in &mut waiters {
                assert!(futures_util::poll!(waiter.as_mut()).is_pending());
            }
            release.send(()).unwrap();
            for result in futures_util::future::join_all(waiters).await {
                assert!(result.unwrap());
            }
        };
        let (first_revision, ()) = futures_util::join!(leader, waiters);
        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let final_revision = repository
            .branch(super::super::repository::META_BRANCH)
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap()
            .revision();
        assert_eq!(
            first_revision, final_revision,
            "waiters must not recommit mount metadata"
        );
        assert_eq!(
            tonk.admission.observations.counts().0,
            2,
            "one mount followed by one read-only verification"
        );
        assert!(
            super::super::join::find_replica_for_subject(&tonk, &subject)
                .await
                .unwrap()
        );
    }

    #[dialog_common::test]
    async fn it_rejects_profile_changes_during_replica_lookup() {
        let (state, key, configuration) = configured_fixture().await;
        let tonk = state.read().await;
        let entry = tonk.admission.entry(&key);
        entry.forget();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *entry.membership_gate.lock() = Some((entered_tx, release_rx));
        let admission = ensure_space_mounted(&tonk, &key);
        let mutation = async {
            entered_rx.await.unwrap();
            super::super::repository::record_space_mount(
                &tonk,
                &key.parse().unwrap(),
                &configuration,
                Some("changed during replica lookup"),
            )
            .await;
            release_tx.send(()).unwrap();
        };
        let (result, ()) = futures_util::join!(admission, mutation);
        assert!(result.unwrap());
        assert!(
            !entry.valid(&tonk, &key),
            "profile lookup and reconciliation must share one freshness window"
        );
        warm(&tonk, &key).await;
    }

    /// The cross-device flow's device-B half, pinned: another device
    /// recorded a space's directory facts (mount records included);
    /// this device — which has never seen the space — must mount it on
    /// first use, by either key spelling the routes produce.
    #[dialog_common::test]
    async fn it_mounts_a_directory_space_on_first_use() {
        use dialog_credentials::ed25519::Ed25519Signer;
        use dialog_repository::SiteAddress;
        use dialog_varsig::Principal as _;

        let tonk = crate::router::tests::test_state().await;

        // A space that exists only as directory facts — as if another
        // device on the account created it and the rows synced in.
        let foreign = Ed25519Signer::generate().await.unwrap();
        let subject: dialog_varsig::Did = foreign.did();
        let address = SiteAddress::from(dialog_remote_ucan_s3::UcanAddress::new(
            "https://sync.example.test/ucan/",
        ));
        let configuration = super::super::repository::RepositoryConfiguration::default()
            .remote(
                "origin",
                super::super::repository::RemoteConfiguration::new(address)
                    .subject(subject.clone())
                    .revocation_url("https://relay.example.test/revocations/".parse().unwrap()),
            )
            .branch(
                "main",
                super::super::repository::BranchConfiguration::default().upstream("origin", "main"),
            );
        assert!(
            !ensure_space_mounted(&tonk, subject.as_str()).await.unwrap(),
            "an absent record must not become a negative cache entry",
        );
        super::super::repository::record_space_mount(
            &tonk,
            &subject,
            &configuration,
            Some("Foreign Space"),
        )
        .await;
        assert!(
            !super::super::join::find_replica_for_subject(&tonk, &subject)
                .await
                .unwrap(),
            "the space must start unmounted for the pin to mean anything"
        );

        // Pages address repos by the FULL did:key URI — the spelling
        // that regressed. Both spellings must mount.
        let full = subject.to_string();
        assert!(
            ensure_space_mounted(&tonk, &full).await.unwrap(),
            "first use mounts the directory space (full-DID spelling)"
        );
        assert!(
            super::super::join::find_replica_for_subject(&tonk, &subject)
                .await
                .unwrap(),
            "the mount records a local replica"
        );
        // Idempotent — and the bare-suffix spelling resolves too.
        let suffix = full.strip_prefix("did:key:").unwrap();
        assert!(ensure_space_mounted(&tonk, suffix).await.unwrap());
    }

    /// A mounted replica is not proof that its configuration is current.
    /// The account directory may have gained the remote and tracking facts
    /// after this device first recorded the local replica, so first use must
    /// reconcile those facts rather than returning early.
    #[dialog_common::test]
    async fn it_reconciles_a_mounted_space_from_the_latest_directory_record() {
        use dialog_repository::{SiteAddress, Upstream};

        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, "late-directory-remote").await;
        let subject: dialog_varsig::Did = key.parse().unwrap();
        let configuration = super::super::repository::RepositoryConfiguration::default()
            .remote(
                "origin",
                super::super::repository::RemoteConfiguration::new(SiteAddress::from(
                    dialog_remote_ucan_s3::UcanAddress::new("https://sync.example.test/ucan/"),
                ))
                .subject(subject.clone()),
            )
            .branch(
                "main",
                super::super::repository::BranchConfiguration::default().upstream("origin", "main"),
            );
        {
            let tonk = state.read().await;
            super::super::repository::record_space_mount(
                &tonk,
                &subject,
                &configuration,
                Some("Late Remote"),
            )
            .await;
        }

        let tonk = state.read().await;
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        assert!(
            matches!(
                session.handle().upstream(),
                Some(Upstream::Remote { ref remote, ref branch, .. })
                    if remote == "origin" && branch == "main"
            ),
            "the mounted replica must adopt the directory's origin/main tracking facts",
        );

        let repository: dialog_repository::Repository = tonk
            .profile
            .repository(&key)
            .load()
            .perform(&tonk.operator)
            .await
            .unwrap();
        let before = repository
            .branch(super::super::repository::META_BRANCH)
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap()
            .revision();
        assert!(ensure_space_mounted(&tonk, &key).await.unwrap());
        let after = repository
            .branch(super::super::repository::META_BRANCH)
            .open()
            .perform(&tonk.operator)
            .await
            .unwrap()
            .revision();
        assert_eq!(
            after, before,
            "a later reconcile over identical facts must not commit again",
        );
    }
}

/// Stamp a space's device-locality into the profile-main OVERLAY so
/// the Hub can style directory rows this device has not replicated.
/// Overlay facts are device-local and die with the worker, so callers
/// stamp at boot and again whenever locality changes.
pub(crate) async fn stamp_space_locality(tonk: &TonkState, subject: &dialog_varsig::Did) {
    let main = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("locality stamp: open profile main: {error}");
            return;
        }
    };
    main.state
        .assert_overlay(tonk_schema::SpaceLocal::new(subject, true));
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&main.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Assert or retract the in-flight replication marker for `subject`.
///
/// Overlay on profile main, beside the locality stamp: device-local and
/// never replicated, because a pull running HERE says nothing about any
/// other device. The fact's presence is the state, so settling retracts
/// it rather than writing false.
pub(crate) async fn stamp_space_replicating(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
    replicating: bool,
) {
    let main = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("replicating stamp: open profile main: {error}");
            return;
        }
    };
    let fact = tonk_schema::SpaceReplicating::new(tonk.profile.did(), subject.clone());
    if replicating {
        main.state.assert_overlay(fact);
    } else {
        // Cleared by DROPPING the entity's overlay facts, not by
        // retracting: an overlay retract records a tombstone beside the
        // assertion rather than removing it, so the fact would still
        // read back. The marker owns its entity (the replica), so
        // dropping the entity takes nothing else with it.
        let entity = fact.this.clone();
        main.state
            .retain_overlay_entities(|overlaid| overlaid != &entity);
    }
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&main.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Boot pass: stamp locality for every replica this device holds.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn stamp_local_spaces(tonk: &TonkState) {
    for key in super::profile_name::real_space_keys(tonk).await {
        if let Some(subject) = space_subject(&key) {
            stamp_space_locality(tonk, &subject).await;
        }
    }
}

/// Rebuild a space's configuration from the account directory — the
/// shared `tonk_schema::directory` reader, converted into the worker's
/// [`RepositoryConfiguration`].
async fn directory_configuration_strict(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
) -> Result<Option<RepositoryConfiguration>, crate::TonkWorkerError> {
    #[cfg(test)]
    {
        tonk.admission
            .observations
            .directory
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if tonk
            .admission
            .observations
            .fail_directory
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(crate::TonkWorkerError::Internal(
                "injected directory failure".into(),
            ));
        }
    }
    let main = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|e| crate::TonkWorkerError::Internal(format!("open directory: {e}")))?;
    let Some(record) =
        tonk_schema::directory::mount_record_strict(main.handle(), subject, &tonk.operator)
            .await
            .map_err(|e| crate::TonkWorkerError::Internal(format!("read directory: {e}")))?
    else {
        return Ok(None);
    };
    let mut configuration = RepositoryConfiguration::default();
    for remote in record.remotes {
        let mut remote_configuration =
            RemoteConfiguration::new(remote.address).subject(remote.subject);
        if let Some(revocation) = remote.revocation
            && let Ok(url) = url::Url::parse(&revocation)
        {
            remote_configuration = remote_configuration.revocation_url(url);
        }
        configuration = configuration.remote(remote.name, remote_configuration);
    }
    for branch in record.branches {
        configuration = configuration.branch(
            branch.name,
            BranchConfiguration {
                upstream: branch
                    .upstream
                    .map(|(remote, branch)| UpstreamConfiguration::new(remote, branch)),
                revision: None,
            },
        );
    }
    Ok(Some(configuration))
}
