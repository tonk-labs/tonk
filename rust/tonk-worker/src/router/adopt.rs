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
    // Where each space has a worker of its own, that worker mounts it, and
    // this one, the person's profile's, mounts none: what it keeps of a space
    // is in its directory (see [`space_directory`](super::space_directory)).
    if tonk.spaces_elsewhere() {
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
                if configuration
                    .as_ref()
                    .is_some_and(|(configuration, changed)| {
                        *changed
                            && configuration
                                .branch
                                .get(super::repository::CONTENT_BRANCH)
                                .is_some_and(|branch| branch.upstream.is_some())
                    })
                {
                    // A page's Load command stamps its route immediately after
                    // admission. Fetch content before that stamp when legacy
                    // mount facts have just repaired an empty local branch.
                    pull_content_on_mount(tonk, &subject).await;
                }
                let upstreams = configuration
                    .into_iter()
                    .flat_map(|(configuration, _)| {
                        let subject = subject.clone();
                        let remotes = configuration.remote;
                        configuration
                            .branch
                            .into_iter()
                            .filter_map(move |(name, branch)| {
                                let upstream = branch.upstream?;
                                let remote = remotes.get(&upstream.remote)?;
                                let recorded = super::remotes::RecordedRemote {
                                    subject: remote
                                        .subject
                                        .clone()
                                        .unwrap_or_else(|| subject.clone()),
                                    address: remote.address.clone(),
                                };
                                Some((name, recorded, upstream.branch))
                            })
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
    pull_content_on_mount(tonk, subject).await;
    super::repository::record_initialized_replica_in_profile(tonk, subject)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("record adopted space '{subject}': {error}"))
        })
}

/// Pull the content branch of a space that was just mounted, before the
/// mount is reported complete.
///
/// Mounting records the replica and configures its remote; the content
/// itself used to arrive with the next background sync drain. The page
/// that triggered the mount does not wait for that: its site stamp ran
/// against the still-empty branch, found no route table, logged
/// `SKIPPED: no route match`, and nothing re-ran it when the pull landed
/// a second later. The "downloading" placeholder then stayed up for
/// good. Pulling here, while the replicating marker is still asserted,
/// makes "it opens as soon as enough of it has arrived" literally what
/// happens: the stamp that follows the mount sees the routes.
///
/// Best-effort: a failed pull is logged and the mount still completes,
/// since the background drain retries the pull and a mount without
/// content is what it was before.
async fn pull_content_on_mount(tonk: &TonkState, subject: &dialog_varsig::Did) {
    // The space's own origin holds its content, and pulls it there.
    if tonk.spaces_elsewhere() {
        return;
    }
    let key = subject.as_str();
    match tonk
        .reactor
        .repository(key)
        .branch(super::repository::CONTENT_BRANCH)
        .pull()
        .perform(&tonk.operator)
        .await
    {
        Ok(_) => log!("space adoption: pulled '{subject}' content"),
        Err(error) => log!("space adoption: content pull for '{subject}' failed: {error}"),
    }
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
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
#[cfg(feature = "persona")]
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
        // The space's own worker is the one that pulls it: reaching that
        // worker brings it up, and it takes the space up from there.
        if tonk.spaces_elsewhere() {
            let listed = super::space_directory::held(&tonk, &key).await.is_ok();
            drop(tonk);
            if !listed {
                log!("ReplicateSpace '{key}': nothing to mount");
                return;
            }
            match super::space_reach::ask(&key, "GET", &format!("/api/repository/{key}"), None)
                .await
            {
                Ok(_) => log!("ReplicateSpace '{key}': its own worker holds it"),
                Err(error) => log!("ReplicateSpace '{key}': {error}"),
            }
            return;
        }
        match ensure_space_mounted(&tonk, &key).await {
            Ok(true) => {
                schedule_seed_upgrade(&tonk, &key).await;
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

/// Catch a mounted space's seed up with the shipped bundle.
///
/// Best-effort by design: at most once per worker instance per space, and
/// a failed attempt simply waits for the next worker to try again.
pub(crate) async fn schedule_seed_upgrade(tonk: &TonkState, key: &str) {
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
    match super::repository::upgrade_seed(tonk, &key).await {
        Ok(true) => log!("seed upgrade: '{key}' caught up with the shipped bundle"),
        Ok(false) => {}
        Err(error) => log!("seed upgrade for mounted '{key}': {error}"),
    }
}

/// Parse either the canonical full repository key or the legacy bare suffix.
pub(super) fn space_subject(key: &str) -> Option<dialog_varsig::Did> {
    if key.starts_with("did:key:") {
        key.parse().ok()
    } else {
        format!("did:key:{key}").parse().ok()
    }
}

/// Apply the account directory's latest mount facts to one replica that is
/// already present locally. Returns whether a mount record existed.
#[cfg(target_arch = "wasm32")]
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
) -> Result<Option<(RepositoryConfiguration, bool)>, crate::TonkWorkerError> {
    let Some(configuration) = directory_configuration_strict(tonk, subject).await? else {
        return Ok(None);
    };
    let repository = tonk
        .profile
        .space(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!(
                "load mounted space '{subject}' for directory reconcile: {error}"
            ))
        })?;
    if mounted_configuration_is_current(tonk, key, &repository, &configuration).await? {
        return Ok(Some((configuration, false)));
    }
    super::repository::ensure_remote_config(tonk, &repository, key, &configuration)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!(
                "reconcile mounted space '{subject}' from directory: {error}"
            ))
        })?;
    Ok(Some((configuration, true)))
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
                replica: Term::var("replica"),
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
                replica: Term::from(replica.this().clone()),
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
                if branch.replica.0 != *replica.this()
                    || remotes_by_entity.contains_key(&branch.this)
                {
                    return None;
                }
                let target = branches_by_entity.get(links_by_entity.get(&branch.this)?)?;
                let remote = remotes_by_entity.get(&target.replica.0)?;
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
        let Some(recorded) =
            super::remotes::find(repository, &upstream.remote, &tonk.operator).await?
        else {
            return Ok(false);
        };
        if !matches!(
            tonk_account::peer::upstream(session.handle()),
            Some(dialog_repository::Upstream::Remote {
                ref remote,
                ref branch,
                ..
            }) if recorded.is(remote) && *branch == upstream.branch
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
#[cfg(target_arch = "wasm32")]
pub(crate) async fn reconcile_account_spaces(tonk: &TonkState) {
    use std::collections::HashMap;

    let directory_names: Option<HashMap<String, String>> = match tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
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

        // The space's own worker holds it, and is told where it syncs with
        // its delegation: there is no mount here to bring up to date, only
        // a space the directory names no remote for to attach one to.
        if tonk.spaces_elsewhere() {
            let Some(remote) = account_remote.as_deref() else {
                continue;
            };
            match super::repository::attach_account_remote_if_local(tonk, &key, remote).await {
                Ok(true) => {
                    log!("space reconcile: attached account remote to local space '{subject}'")
                }
                Ok(false) => {}
                Err(error) => {
                    log!("space reconcile: attach account remote to '{subject}': {error}")
                }
            }
            continue;
        }

        let repository = match tonk
            .profile
            .space(&key)
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
                tonk.sync_queue.mark_dirty(&key, super::sync::now_millis());
            }
            Ok(false) => {}
            Err(error) => log!("space reconcile: attach account remote to '{subject}': {error}"),
        }
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
        .branch(&tonk.active_branch)
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
        .branch(&tonk.active_branch)
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

/// Rebuild a space's configuration from the account directory — the
/// shared `tonk_schema::directory` reader, converted into the worker's
/// [`RepositoryConfiguration`].
pub(super) async fn directory_configuration_strict(
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
        .branch(&tonk.active_branch)
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
