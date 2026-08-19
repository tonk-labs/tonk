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
    // Routes address a space by either spelling: the bare routing key
    // (the DID's method-specific suffix) or the full did:key URI —
    // pages query with the full form. Normalize before parsing, or the
    // full form silently fails the parse and adoption never fires.
    let suffix = key.strip_prefix("did:key:").unwrap_or(key);
    let subject: dialog_varsig::Did = match format!("did:key:{suffix}").parse() {
        Ok(did) => did,
        Err(_) => return Ok(false), // not a space key (e.g. a named repo)
    };
    if super::account_state::is_account_key(tonk, key).await
        || super::account_state::is_account_key(tonk, suffix).await
    {
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
    stamp_space_locality(tonk, &subject).await;
    // The mount wires the upstream but the content arrives over a pull;
    // mark the repo dirty so the next drain (the page's own follow-up
    // requests trigger one) fills the space in promptly instead of
    // waiting for an idle beat.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    tonk.sync_queue.mark_dirty(key, js_sys::Date::now());
    Ok(true)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use super::*;

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
        super::super::repository::record_space_mount(
            &tonk,
            &subject,
            &configuration,
            Some("Foreign Spot"),
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

/// Boot pass: stamp locality for every replica this device holds.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn stamp_local_spaces(tonk: &TonkState) {
    for key in super::profile_name::real_space_keys(tonk).await {
        if let Ok(subject) = format!("did:key:{key}").parse::<dialog_varsig::Did>() {
            stamp_space_locality(tonk, &subject).await;
        }
    }
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
