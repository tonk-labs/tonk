//! Directory-driven space adoption — the account DB replacing the
//! account-service spot-backup escrow.
//!
//! The account DB (the account repository, reached through profile
//! main) is the source of truth for which spaces exist: a [`Space`]
//! directory row per space, a [`SpaceRemote`] mount record for each
//! synced one, and the retained delegation carrying the authority.
//! Adoption reads exactly that: every directory row with a mount
//! record and no local replica is mounted from the account DB alone.
//!
//! Deletion needs no special machinery here: a removed space's rows
//! are retracted, retraction replicates like any fact, and adoption
//! only ever sees asserted rows — so a deleted space cannot resurrect
//! the way the escrow restore resurrected it (the escrow was
//! append-only and out-of-band, so "not installed locally" and
//! "deliberately removed" were indistinguishable).
//!
//! [`record_missing_space_remotes`] is the self-healing writer: any
//! locally mounted space whose directory entity lacks a mount record
//! gets one, resolved from its configured remote. That migrates
//! existing spaces (created before [`SpaceRemote`] existed) without a
//! dedicated migration, and heals any create path that failed to
//! record.

use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use tonk_common::log;
use tonk_schema::{SpaceRemote, prelude::DidExt as _};

use crate::worker::TonkState;

/// Mount every space the account DB lists that this device lacks.
///
/// Runs at worker boot and after account pulls. Errors are per-space
/// and logged — one unmountable space must not block the rest.
pub(crate) async fn adopt_directory_spaces(tonk: &TonkState) {
    let main = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("space adoption: open profile main: {error}");
            return;
        }
    };
    let rows: Vec<SpaceRemote> = match main
        .handle()
        .query()
        .select(Query::<SpaceRemote> {
            this: Term::var("this"),
            remote: Term::var("remote"),
            revocation: Term::var("revocation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            log!("space adoption: mount-record query: {error:?}");
            return;
        }
    };

    for row in rows {
        let subject: dialog_varsig::Did = match row.this.to_string().parse() {
            Ok(did) => did,
            Err(error) => {
                log!(
                    "space adoption: directory entity '{}' is not a DID: {error:?}",
                    row.this
                );
                continue;
            }
        };
        match super::join::find_replica_for_subject(tonk, &subject).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                log!("space adoption: replica probe for '{subject}': {error}");
                continue;
            }
        }
        log!("space adoption: mounting '{subject}' from the account directory");
        if let Err(error) = super::join::mount_replica(
            tonk,
            &subject,
            Some(row.remote.0.as_str()),
            Some(row.revocation.0.as_str()),
        )
        .await
        {
            log!("space adoption: mount '{subject}' failed: {error}");
            continue;
        }
        if let Err(error) =
            super::repository::record_initialized_replica_in_profile(tonk, &subject).await
        {
            log!("space adoption: record '{subject}' failed: {error}");
        }
    }
}

/// Assert the [`SpaceRemote`] mount record for every locally mounted,
/// remote-configured space whose directory entity lacks one.
///
/// The self-healing writer behind adoption: covers spaces created
/// before the record existed and any create path that failed to write
/// it, so no dedicated migration is needed.
pub(crate) async fn record_missing_space_remotes(tonk: &TonkState) {
    use tonk_schema::Space;

    let main = match tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    {
        Ok(main) => main,
        Err(error) => {
            log!("mount-record backstop: open profile main: {error}");
            return;
        }
    };
    let directory: Vec<Space> = match main
        .handle()
        .query()
        .select(Query::<Space> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            status: Term::var("status"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            log!("mount-record backstop: directory query: {error:?}");
            return;
        }
    };
    let recorded: std::collections::HashSet<String> = match main
        .handle()
        .query()
        .select(Query::<SpaceRemote> {
            this: Term::var("this"),
            remote: Term::var("remote"),
            revocation: Term::var("revocation"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
    {
        Ok(rows) => rows.into_iter().map(|r| r.this.to_string()).collect(),
        Err(error) => {
            log!("mount-record backstop: mount-record query: {error:?}");
            return;
        }
    };

    for entry in directory {
        if recorded.contains(&entry.this.to_string()) {
            continue;
        }
        let subject: dialog_varsig::Did = match entry.subject.0.to_string().parse() {
            Ok(did) => did,
            Err(_) => continue,
        };
        // Only a locally mounted space can tell us its remote.
        match super::join::find_replica_for_subject(tonk, &subject).await {
            Ok(true) => {}
            _ => continue,
        }
        let repository = match tonk
            .profile
            .repository(subject.repo_key())
            .load()
            .perform(&tonk.operator)
            .await
        {
            Ok(repository) => repository,
            Err(error) => {
                log!("mount-record backstop: load '{subject}': {error}");
                continue;
            }
        };
        let urls = match super::create_invite::resolve_remote_url_with(&repository, &tonk.operator)
            .await
        {
            Ok(super::create_invite::RemoteRequirement::Ready(urls)) => urls,
            Ok(_) => continue, // local-only or misconfigured: nothing to record
            Err(error) => {
                log!("mount-record backstop: resolve remote for '{subject}': {error}");
                continue;
            }
        };
        let record = SpaceRemote::new(
            &subject,
            urls.access_url.to_string(),
            urls.revocation_url.to_string(),
        );
        match main
            .handle()
            .transaction()
            .assert(record)
            .commit()
            .perform(&tonk.operator)
            .await
        {
            Ok(_) => log!("mount-record backstop: recorded remote for '{subject}'"),
            Err(error) => log!("mount-record backstop: record '{subject}': {error}"),
        }
    }
}
