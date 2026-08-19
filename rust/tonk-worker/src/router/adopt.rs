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

use std::collections::HashMap;

use dialog_query::{Output as _, Query, Term};
use tonk_common::log;
use tonk_schema::domain::remote::{Address as RemoteAddress, Origin as RemoteOrigin};
use tonk_schema::{
    Branch as BranchConcept, Remote, RemoteExecution, TrackingBranch, prelude::DidExt as _,
};

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
    // Routing keys are the DID's method-specific suffix.
    let subject: dialog_varsig::Did = match format!("did:key:{key}").parse() {
        Ok(did) => did,
        Err(_) => return Ok(false), // not a space key (e.g. a named repo)
    };
    if super::account_state::is_account_key(tonk, key).await {
        return Ok(false);
    }
    if super::join::find_replica_for_subject(tonk, &subject).await? {
        return Ok(true);
    }
    let Some(configuration) = directory_configuration(tonk, &subject).await else {
        return Ok(false);
    };
    log!("space adoption: mounting '{subject}' from the account directory");
    super::join::mount_replica_with_configuration(tonk, &subject, configuration).await?;
    super::repository::record_initialized_replica_in_profile(tonk, &subject)
        .await
        .map_err(|error| {
            crate::TonkWorkerError::Internal(format!("record adopted space '{subject}': {error}"))
        })?;
    Ok(true)
}

/// Rebuild a space's [`RepositoryConfiguration`] from remote/branch
/// facts anchored on `anchor`, read through `branch`. `None` when no
/// remotes are recorded there.
async fn configuration_from_facts(
    tonk: &TonkState,
    branch: &crate::reactor::BranchSession,
    anchor: &dialog_artifacts::Entity,
    default_subject: &dialog_varsig::Did,
    skip_branch: Option<&str>,
) -> Option<RepositoryConfiguration> {
    let remotes: Vec<Remote> = branch
        .handle()
        .query()
        .select(Query::<Remote> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(RemoteOrigin::from(anchor.clone())),
            subject: Term::var("subject"),
            address: Term::var("address"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    if remotes.is_empty() {
        return None;
    }

    let mut configuration = RepositoryConfiguration::default();
    // Remote entity → local name, to resolve tracking upstreams below.
    let mut remote_names: HashMap<String, String> = HashMap::new();
    for row in &remotes {
        let address = RemoteAddress::decode(&row.address).ok()?;
        let target: dialog_varsig::Did = row
            .subject
            .0
            .to_string()
            .parse()
            .unwrap_or_else(|_| default_subject.clone());
        let mut remote = RemoteConfiguration::new(address).subject(target);
        let executions: Vec<RemoteExecution> = branch
            .handle()
            .query()
            .select(Query::<RemoteExecution> {
                this: Term::from(row.this.clone()),
                revocation_url: Term::var("revocation_url"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap_or_default();
        if let Some(execution) = executions.into_iter().next()
            && let Ok(url) = url::Url::parse(&execution.revocation_url.0)
        {
            remote = remote.revocation_url(url);
        }
        configuration = configuration.remote(row.name.0.clone(), remote);
        remote_names.insert(row.this.to_string(), row.name.0.clone());
    }

    // Local branches anchored on `anchor`, and their tracking links.
    // An upstream branch entity is anchored on its remote concept, so
    // mapping it back to (remote name, branch name) goes through the
    // remote-side branch rows.
    let locals: Vec<BranchConcept> = branch
        .handle()
        .query()
        .select(Query::<BranchConcept> {
            this: Term::var("this"),
            name: Term::var("name"),
            origin: Term::from(tonk_schema::domain::branch::Origin::from(anchor.clone())),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();
    let mut remote_branches: HashMap<String, (String, String)> = HashMap::new();
    for (remote_entity, remote_name) in &remote_names {
        let origin: dialog_artifacts::Entity = match remote_entity.parse() {
            Ok(entity) => entity,
            Err(_) => continue,
        };
        let rows: Vec<BranchConcept> = branch
            .handle()
            .query()
            .select(Query::<BranchConcept> {
                this: Term::var("this"),
                name: Term::var("name"),
                origin: Term::from(tonk_schema::domain::branch::Origin::from(origin)),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap_or_default();
        for row in rows {
            remote_branches.insert(
                row.this.to_string(),
                (remote_name.clone(), row.name.0.clone()),
            );
        }
    }
    for local in locals {
        if Some(local.name.0.as_str()) == skip_branch {
            continue;
        }
        let tracking: Vec<TrackingBranch> = branch
            .handle()
            .query()
            .select(Query::<TrackingBranch> {
                this: Term::from(local.this.clone()),
                upstream: Term::var("upstream"),
                origin: Term::var("origin"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap_or_default();
        let upstream = tracking.into_iter().next().and_then(|link| {
            remote_branches
                .get(&link.upstream.0.to_string())
                .map(|(remote, branch)| UpstreamConfiguration::new(remote.clone(), branch.clone()))
        });
        configuration = configuration.branch(
            local.name.0.clone(),
            BranchConfiguration {
                upstream,
                revision: None,
            },
        );
    }
    Some(configuration)
}

/// Rebuild a space's configuration from the account directory.
async fn directory_configuration(
    tonk: &TonkState,
    subject: &dialog_varsig::Did,
) -> Option<RepositoryConfiguration> {
    let main = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    configuration_from_facts(tonk, &main, &subject.this(), subject, None).await
}
