//! `GET /api/migrate/repo-vs-profile`: backfill the `kind`
//! attribute on existing [`Replica`] records, then redirect to `/`.
//!
//! The `kind` field (added after replicas were already in the
//! wild) distinguishes the profile's own self-replica
//! (`tonk:profile`) from the spaces it has joined or created
//! (`tonk:repository`). Records written before the field existed
//! carry no `kind`, so they don't match a `kind`-pinned query
//! (e.g. the Hub's `space` concept). This one-shot migration
//! stamps every such record.
//!
//! Classification mirrors [`Replica::new`]: the self-replica is
//! the one whose `subject` equals the profile entity; everything
//! else is a repository.
//!
//! Exposed as a plain `GET` so it can be triggered by visiting the
//! URL; it redirects to the Hub (`/`) when done. Idempotent:
//! records that already carry a `kind` are skipped, so re-running
//! (or a stray prefetch) stamps nothing.

use std::collections::HashSet;

use ::axum::extract::State;
use ::axum::response::Redirect;
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Repository;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::domain::replica::Profile as ProfileEntity;
use tonk_schema::{LegacyReplica, Replica, SpaceKind, prelude::DidExt as _};

use super::AppState;
use crate::TonkWorkerError;

/// Meta branch name on the profile repository. Mirrors the private
/// copies in sibling modules.
const META_BRANCH: &str = "meta";

/// Tally of what the migration stamped.
#[derive(Clone, Copy, Debug, Default)]
struct MigrationReport {
    /// How many replicas were stamped this run (0 on a no-op
    /// re-run).
    migrated: usize,
    /// Of those, how many were classified as the profile's own
    /// self-replica (`tonk:profile`). Normally 0 or 1.
    profile: usize,
    /// Of those, how many were classified as spaces
    /// (`tonk:repository`).
    repository: usize,
}

/// Handler. Runs the migration then redirects to the Hub (`/`).
#[wasm_compat]
pub async fn repo_vs_profile(
    State(state): State<AppState>,
) -> Result<Redirect, TonkWorkerError> {
    run_migration(state).await?;
    Ok(Redirect::to("/"))
}

/// Enumerate every replica on the profile meta branch via
/// [`LegacyReplica`] (which has no `kind` and so matches records
/// written before the field existed), skip those that already
/// carry a `kind`, and stamp the rest with [`SpaceKind`].
#[wasm_compat]
async fn run_migration(state: AppState) -> Result<MigrationReport, TonkWorkerError> {
    log!("GET /api/migrate/repo-vs-profile");

    let tonk = state.read().await;
    let profile_entity = tonk.profile.did().this();
    let profile_repository = Repository::from(&tonk.profile);
    let meta = profile_repository
        .branch(META_BRANCH)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open profile meta branch: {e}"))
        })?;

    // Every replica, kind or no kind: `LegacyReplica` omits `kind`,
    // so it matches records that predate the field as well as new
    // ones.
    let all: Vec<LegacyReplica> = meta
        .query()
        .select(Query::<LegacyReplica> {
            this: Term::var("this"),
            name: Term::var("name"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(profile_entity.clone())),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("legacy-replica query failed: {e:?}")))?;

    // Replicas that already carry a `kind` — these match the full
    // concept. Anything in `all` but not here needs stamping.
    let already: HashSet<_> = meta
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            name: Term::var("name"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(profile_entity.clone())),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("kinded-replica query failed: {e:?}")))?
        .into_iter()
        .map(|replica| replica.this)
        .collect();

    let profile_kind = Replica::profile_kind();
    let repository_kind = Replica::repository_kind();

    let mut transaction = meta.transaction();
    let mut report = MigrationReport::default();

    for replica in &all {
        if already.contains(&replica.this) {
            continue;
        }
        // The self-replica is the one whose `subject` is the
        // profile itself.
        let kind = if replica.subject.0 == profile_entity {
            report.profile += 1;
            profile_kind.clone()
        } else {
            report.repository += 1;
            repository_kind.clone()
        };
        transaction = transaction.assert(SpaceKind::new(replica.this.clone(), kind));
        report.migrated += 1;
    }

    if report.migrated > 0 {
        transaction
            .commit()
            .perform(&tonk.operator)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("commit migration failed: {e}")))?;
    }

    log!(
        "Migration repo-vs-profile: migrated={} profile={} repository={}",
        report.migrated,
        report.profile,
        report.repository
    );
    Ok(report)
}
