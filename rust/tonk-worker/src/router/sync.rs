//! Sync routes for push/pull operations with upstream.
//!
//! These endpoints allow synchronizing a branch with its upstream remote.
//! Each response carries the local branch revision before and after the
//! operation so the UI can render a diff (or detect "nothing changed").
//!
//! Pull/push run through the reactor's chain so a successful pull
//! re-polls every subscription on the branch (push doesn't change
//! local state, so it doesn't re-poll).

use std::collections::HashMap;

use ::axum::{Json, extract::Path, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_capability::access::AuthorizeError;
use dialog_effects::Rejection;
use dialog_repository::{PublishError, PullError, Revision};
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{SyncState, classify};
use tonk_worker_api::SyncDisposition;
pub use tonk_worker_api::{SyncResponse, SyncStatusResponse};

use super::{AppState, BranchConfiguration};
use crate::TonkWorkerError;
use crate::broadcast::{Notification, broadcast};

/// Announce on the branch's broadcast channel that its head may have
/// moved, so subscribed UIs refresh their revision and sync-state
/// badges without a full refetch. The channel mirrors the endpoint
/// path ("channel name == endpoint path"). A `None` revision (branch
/// with no commits) is skipped — there's nothing to announce.
///
/// Posted after a *successful* pull/push/sync. A push leaves the
/// local head where it was, so its announcement carries the unchanged
/// revision; listeners still re-read the read-only sync status, which
/// is what actually flips (e.g. `ahead` → `synced`).
fn announce_head(repo: &str, branch: &str, revision: Option<Revision>) {
    if let Some(revision) = revision {
        broadcast(
            &format!("/api/repository/{repo}/branch/{branch}"),
            &Notification {
                branch: branch.to_string(),
                revision,
            },
        );
    }
}

/// Publish the live sync `status` to the SPACE branch's OVERLAY, keyed on
/// the fixed `state:here` entity (`Replica::SYNC_STATE_HERE`).
///
/// The status is a transient observation (idle / pending / offline / local),
/// so it goes to the overlay — never persisted, never replicated — and folds
/// into the chip's `tonk:sync` subscription. A well-known singleton entity
/// (like `tonk:join/status`); one space is in scope per page, so the chip
/// needs no replica entity.
///
/// It lives on the SPACE branch (the one the page is showing), not the
/// profile meta branch: a sealed-guest chip can only reach the branch the
/// `<tonk-portal>` is mounted under (the bridge annotates the outer
/// `<tonk-repository name={space}>` context), so the overlay must be where
/// the chip already queries. Overlay-only: no commit.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish_sync_status(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    state: SyncState,
) {
    publish_sync_status_attr(
        tonk,
        repo,
        branch,
        tonk_schema::Replica::sync_status_attr(state),
    )
    .await;
}

/// Stamp a specific `tonk/sync` `status` value into the `state:here` overlay
/// (e.g. `offline` on a fetch failure, where there is no `SyncState` to map).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn publish_sync_status_attr(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    status: tonk_schema::domain::sync::Status,
) {
    use std::sync::Arc;
    use tonk_schema::{Replica, ReplicaSyncStatus};

    let Ok(entity) = Replica::SYNC_STATE_HERE.parse() else {
        return;
    };
    let stamp = ReplicaSyncStatus::new(entity, status);

    let session = match tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            log!("publish_sync_status: failed to acquire {repo}/{branch}: {e}");
            return;
        }
    };
    // `status` is a cardinality-one attribute, so asserting supersedes the
    // prior value at `state:here` rather than accumulating — the chip's fold
    // always sees exactly the latest status.
    session.state.assert_overlay(stamp);
    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Stamp `sync:paused` into the `state:here` overlay so the chip reflects a
/// just-paused replica without waiting for a status sweep (which a paused
/// replica skips). Called by the pause-sync command handler after it commits
/// the durable preference.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn publish_paused_status(tonk: &crate::worker::TonkState, repo: &str, branch: &str) {
    publish_sync_status_attr(tonk, repo, branch, tonk_schema::Replica::paused_status()).await;
}

/// Stamp the self-identity overlay (`state:self`) on a space branch so
/// the topbar chip can render the member's sigil + name without seeing
/// the profile branch. Overlay-only — never committed, never replicated.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn publish_self_identity(tonk: &crate::worker::TonkState, repo: &str, branch: &str) {
    use tonk_schema::{ProfileIdentity, Replica, prelude::DidExt as _};

    let Ok(entity) = Replica::SELF_STATE_HERE.parse() else {
        return;
    };
    let did = tonk.profile.did().this();
    let name = crate::router::profile_name::resolve_display_name(tonk).await;
    let stamp = ProfileIdentity::new(entity, did, name);

    let session = match tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            log!("publish_self_identity: failed to acquire {repo}/{branch}: {e}");
            return;
        }
    };
    session.state.assert_overlay(stamp);
    tonk.reactor
        .schedule_poll(std::sync::Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Whether auto-sync is enabled for `repo`'s content branch: reads the durable
/// boolean [`ReplicaSyncEnabled`](tonk_schema::ReplicaSyncEnabled) preference
/// keyed on this device's replica entity, on the space content branch. An
/// absent fact means "never paused" → enabled, so a fresh replica syncs by
/// default.
///
/// The gate the service worker's background sweep and the in-page coordinator
/// both consult before pulling/pushing. Keyed on the replica entity (derived
/// from `(profile, subject)`) rather than the `state:here` singleton, so the
/// preference is this device's own.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn is_sync_enabled(tonk: &crate::worker::TonkState, repo: &str, branch: &str) -> bool {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{Replica, ReplicaSyncEnabled};

    let session = match tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            log!("is_sync_enabled: failed to acquire {repo}/{branch}: {e}");
            return true;
        }
    };

    // The replica key: the branch handle knows its own subject DID, so derive
    // the replica entity from `(profile, subject)` straight off the session.
    let replica = Replica::new(tonk.profile.did(), session.handle().of().clone())
        .this()
        .clone();

    let enabled: Vec<ReplicaSyncEnabled> = session
        .handle()
        .query()
        .select(Query::<ReplicaSyncEnabled> {
            this: Term::from(replica),
            enabled: Term::var("enabled"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    // Paused only when a fact explicitly says `enabled = false`; absent →
    // enabled.
    !enabled.iter().any(|fact| !fact.enabled.0)
}

/// Re-classify `repo`/`branch` against a fresh upstream fetch and publish the
/// settled `tonk/sync` status to the chip's overlay (`synced` / `syncing…` via
/// `ahead`/`behind`/`diverged` / `offline`). This is the settle step a sync run
/// owes the chip: the `sync` op flips to `pending` at the start but must
/// publish the resolved state when it finishes, or the chip stays `syncing…`
/// forever (the regression after the in-page status controller was removed).
///
/// Honors the pause preference (a replica paused mid-sync keeps `paused`) and
/// treats an unreachable remote as `offline` rather than clobbering with a
/// stale reading. A branch with no upstream publishes `offline` (nothing to
/// sync against). Best-effort: a failure to acquire/fetch is logged, not
/// surfaced — the caller's sync result already carried the real outcome.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish_settled_status(tonk: &crate::worker::TonkState, repo: &str, branch: &str) {
    let account = super::account_state::is_account_key(tonk, repo).await;
    // A user space paused mid-sync keeps `paused`. Account-system replicas
    // ignore user pause preferences and always remain in the sync population.
    if !account && !is_sync_enabled(tonk, repo, branch).await {
        publish_paused_status(tonk, repo, branch).await;
        return;
    }

    let session = match tonk
        .reactor
        .repository(repo)
        .branch(branch)
        .acquire(&tonk.operator)
        .await
    {
        Ok(session) => session,
        Err(e) => {
            log!("publish_settled_status: failed to acquire {repo}/{branch}: {e}");
            return;
        }
    };
    let handle = session.handle();
    let local = handle.revision();

    if handle.upstream().is_none() {
        publish_sync_status(tonk, repo, branch, SyncState::NoUpstream).await;
        return;
    }

    let remote = match handle.fetch().perform(&tonk.operator).await {
        Ok(remote) => remote,
        Err(e) => {
            log!("publish_settled_status: fetch {repo}/{branch} failed: {e}");
            publish_sync_status_attr(tonk, repo, branch, tonk_schema::Replica::offline_status())
                .await;
            return;
        }
    };

    let state = SyncState::from(classify(local.as_ref(), remote.as_ref()));
    // Re-check pause: it could have landed during the (awaiting) fetch.
    if is_sync_enabled(tonk, repo, branch).await {
        publish_sync_status(tonk, repo, branch, state).await;
    } else {
        publish_paused_status(tonk, repo, branch).await;
    }
}

/// How many times a `/sync` pull refreshes-and-retries when a concurrent
/// commit advances the branch head mid-merge. Each retry reuses the blocks the
/// prior attempt already fetched, so this only re-runs the cheap merge+CAS, not
/// the network pull. A small bound: in practice one local writer races at a
/// time, so a single retry settles it; the extra attempts guard against a burst
/// of commits.
const SYNC_RETRY_LIMIT: usize = 4;

/// Whether `error` is the typed "branch head moved under us" mismatch raised
/// when a concurrent commit advances the local head during pull.
fn is_head_moved(error: &crate::reactor::ReactorError) -> bool {
    matches!(
        error,
        crate::reactor::ReactorError::Pull(PullError::Publish(
            PublishError::VersionMismatch { .. }
        ))
    )
}

/// Walk an error's `source()` chain and probe each node for the typed
/// reason it carries.
///
/// Dialog's effect errors keep their reasons intact — an
/// [`AuthorizeError`] or [`Rejection`] built at the service boundary
/// rides the chain to the caller — but the carrier variants are
/// `#[error(transparent)]`, which forwards `source()` PAST the reason,
/// so a plain downcast walk never lands on it. Each node is therefore
/// probed for the known carrier enums and the reason read out of the
/// variant directly.
macro_rules! chain_reason {
    ($name:ident, $reason:ty, $variant:ident) => {
        pub(crate) fn $name<'a>(
            error: &'a (dyn std::error::Error + 'static),
        ) -> Option<&'a $reason> {
            use dialog_artifacts::DialogArtifactsError;
            use dialog_effects::archive::ArchiveError;
            use dialog_effects::blob::BlobError;
            use dialog_effects::memory::MemoryError;
            use dialog_repository::ResolveError;
            let mut current: Option<&'a (dyn std::error::Error + 'static)> = Some(error);
            while let Some(node) = current {
                if let Some(reason) = node.downcast_ref::<$reason>() {
                    return Some(reason);
                }
                if let Some(PublishError::$variant(reason)) = node.downcast_ref::<PublishError>() {
                    return Some(reason);
                }
                if let Some(ResolveError::$variant(reason)) = node.downcast_ref::<ResolveError>() {
                    return Some(reason);
                }
                if let Some(MemoryError::$variant(reason)) = node.downcast_ref::<MemoryError>() {
                    return Some(reason);
                }
                if let Some(ArchiveError::$variant(reason)) = node.downcast_ref::<ArchiveError>() {
                    return Some(reason);
                }
                if let Some(BlobError::$variant(reason)) = node.downcast_ref::<BlobError>() {
                    return Some(reason);
                }
                if let Some(DialogArtifactsError::$variant(reason)) =
                    node.downcast_ref::<DialogArtifactsError>()
                {
                    return Some(reason);
                }
                current = node.source();
            }
            None
        }
    };
}

chain_reason!(authorization_reason, AuthorizeError, Authorization);
chain_reason!(rejection_reason, Rejection, Rejected);

fn classified_service_failure(
    error: &(dyn std::error::Error + 'static),
) -> Option<TonkWorkerError> {
    if let Some(authorization) = authorization_reason(error) {
        return Some(match authorization {
            AuthorizeError::Revoked { .. } => TonkWorkerError::Upstream {
                status: 403,
                code: Some("CREDENTIAL_REVOKED".to_string()),
                message: "remote access has been revoked".to_string(),
            },
            AuthorizeError::Unavailable { .. } | AuthorizeError::UnavailableProof { .. } => {
                TonkWorkerError::Upstream {
                    status: 503,
                    code: Some("SYNC_UNAVAILABLE".to_string()),
                    message: "synchronization is temporarily unavailable".to_string(),
                }
            }
            _ => TonkWorkerError::Upstream {
                status: 502,
                code: Some("UPSTREAM_ERROR".to_string()),
                message: "the upstream service could not complete synchronization".to_string(),
            },
        });
    }
    let rejection = rejection_reason(error)?;
    Some(if rejection.is_transient() {
        TonkWorkerError::Upstream {
            status: 503,
            code: Some("SYNC_UNAVAILABLE".to_string()),
            message: "synchronization is temporarily unavailable".to_string(),
        }
    } else {
        TonkWorkerError::Upstream {
            status: 502,
            code: Some("UPSTREAM_ERROR".to_string()),
            message: "the upstream service could not complete synchronization".to_string(),
        }
    })
}

fn sync_failure(error: &crate::reactor::ReactorError) -> TonkWorkerError {
    if is_head_moved(error)
        || matches!(
            error,
            crate::reactor::ReactorError::Push(dialog_repository::PushError::NonFastForward { .. })
        )
    {
        return TonkWorkerError::Upstream {
            status: 409,
            code: Some("SYNC_CONFLICT".to_string()),
            message: "synchronization conflicted with another update".to_string(),
        };
    }
    if let Some(error) = classified_service_failure(error) {
        return error;
    }
    TonkWorkerError::Upstream {
        status: 503,
        code: Some("SYNC_UNAVAILABLE".to_string()),
        message: "synchronization is temporarily unavailable".to_string(),
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn publish_failure_status(
    tonk: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    error: &TonkWorkerError,
) {
    let status = match error {
        TonkWorkerError::Upstream {
            code: Some(code), ..
        } if code == "CREDENTIAL_REVOKED" => tonk_schema::Replica::revoked_status(),
        TonkWorkerError::Upstream {
            code: Some(code), ..
        } if code == "SYNC_CONFLICT" => tonk_schema::Replica::conflict_status(),
        _ => tonk_schema::Replica::unavailable_status(),
    };
    publish_sync_status_attr(tonk, repo, branch, status).await;
}

/// Branch names in `branches` that have an upstream, sorted for a
/// stable sweep order. Branches without an upstream have nowhere to
/// sync to, so they're skipped. Shared with the in-page sweep so the
/// background `sync` event and the in-page polyfill pick exactly the
/// same branches.
pub fn branches_to_sync(branches: &HashMap<String, BranchConfiguration>) -> Vec<String> {
    let mut names: Vec<String> = branches
        .iter()
        .filter(|(_, config)| config.upstream.is_some())
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names
}

/// Stamp `sync:offline` at `state:here` on every open repository's upstream
/// branches, WITHOUT touching the network. The per-fetch drain calls this
/// instead of sweeping while the browser reports offline, so the chip/disc
/// reflect the disconnect — skipping silently left them frozen on the last
/// online status. Overlay-only and idempotent: a re-stamp of the same value
/// changes nothing, so subscribers see exactly one `offline` frame.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn mark_offline(state: &AppState) {
    let open: Vec<String> = {
        let tonk = state.read().await;
        tonk.reactor.repos().read().keys().cloned().collect()
    };
    for repo in open {
        let info = match super::repository::get_repository(State(state.clone()), Path(repo.clone()))
            .await
        {
            Ok(Json(info)) => info,
            Err(_) => continue,
        };
        let tonk = state.read().await;
        for branch in branches_to_sync(&info.branch) {
            publish_sync_status_attr(
                &tonk,
                &repo,
                &branch,
                tonk_schema::Replica::offline_status(),
            )
            .await;
        }
    }
}

/// Sweep every upstream branch of `repo`, reusing the per-branch
/// [`sync`] route. This is what a durable background `sync` event runs
/// once it has parsed the repo out of its tag; it makes the same
/// branch selection ([`branches_to_sync`]) as the in-page sweep.
///
/// `Ok(())` means every selected branch reconciled, or there was
/// nothing to do. `Err` means at least one branch did not land — the
/// caller surfaces that as a rejected `sync` so the user agent retries
/// with backoff. An unknown repo is not an error: there is nothing to
/// retry, so it resolves as a no-op.
pub async fn sync_repository(state: &AppState, repo: &str) -> Result<(), String> {
    let account = {
        let tonk = state.read().await;
        super::account_state::is_account_key(&tonk, repo).await
    };
    if account {
        // The account repository's sweep is `ensure_account_state_swept` and
        // stops there: it mounts, hydrates when it must, then pulls, projects
        // and pushes. Falling through to the generic per-branch route below
        // would reconcile the same branch a second time every heartbeat, and
        // stamp status on a replica that has no chip to read it.
        let (status, swept) = {
            let tonk = state.read().await;
            super::account_state::ensure_account_state_swept(&tonk).await
        };
        if status != tonk_account::AccountStateStatus::Ready {
            return Err("account repository remains unhydrated".to_string());
        }
        return swept;
    }

    // Honor the durable pause preference: a paused replica skips the whole
    // sweep (no pull, no push) until resumed. This is the gate localStorage
    // couldn't provide — the SW can read this branch fact. Keyed on the
    // content branch, where the chip writes the preference.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk = state.read().await;
        // The account repository returned above, so everything reaching here is
        // a user space with a pause preference a chip can have written.
        if !is_sync_enabled(&tonk, repo, "main").await {
            log!("background sync of '{repo}' skipped: paused");
            // Re-stamp `sync:paused` on the way out. The status lives in the
            // SW's in-memory overlay and is lost on reload / worker restart;
            // the durable `enabled=false` preference survives, but without
            // this the chip's `state:here` subscription comes back empty
            // (the sweep is the only thing that stamps status, and a paused
            // replica took the early return before ever stamping) — so a
            // paused space rendered as un-paused after every reload. The chip
            // reads `state:here` on the content branch (`main`);
            // `publish_paused_status` asserts the overlay and drains the poll.
            publish_paused_status(&tonk, repo, "main").await;
            return Ok(());
        }
    }

    let info = match super::repository::get_repository(State(state.clone()), Path(repo.to_string()))
        .await
    {
        Ok(Json(info)) => info,
        Err(e) => {
            log!("background sync: could not load '{repo}': {e}");
            return Ok(());
        }
    };

    let mut failed = false;
    for branch in branches_to_sync(&info.branch) {
        let params = SyncPath {
            repo: repo.to_string(),
            branch: branch.clone(),
        };
        // HTTP success now means reconciliation completed or was deliberately
        // skipped; operational failures are typed route errors.
        match sync(State(state.clone()), Path(params)).await {
            Ok(_) => {}
            Err(e) => {
                log!("background sync of {repo}/{branch} failed: {e}");
                failed = true;
            }
        }
    }

    if failed {
        Err(format!(
            "background sync of '{repo}' did not fully reconcile"
        ))
    } else {
        Ok(())
    }
}

/// Path parameters for sync endpoints.
#[derive(Debug, Deserialize)]
pub struct SyncPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Pull changes from the upstream remote.
#[wasm_compat]
pub async fn pull(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pulling from upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    // Before the lock, because renewal takes one of its own — and before
    // the presign below, because a worker that has just restarted holds
    // guest chains addressed to an operator that no longer exists.
    ensure_session_authority(&state).await?;

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    // A branch that tracks nothing has nothing to reconcile, which is a
    // configuration state rather than a failure — the same reading
    // [`sync`] and [`publish_settled_status`] give it. Without this the
    // pull reaches dialog, comes back `BranchHasNoUpstream`, and lands in
    // the catch-all as a 503 "temporarily unavailable", which is untrue:
    // nothing is going to become available until a remote is attached.
    if session.handle().upstream().is_none() {
        log!(
            "Pull skipped, no upstream: {}@{}",
            params.branch,
            params.repo
        );
        return Ok(Json(SyncResponse {
            success: true,
            disposition: SyncDisposition::Completed,
            before: before.clone(),
            after: before,
            error: None,
        }));
    }

    // Authorization-bearing branches (the account, and through it the
    // profile's access branch) must never be left partial: the session
    // open at the next boot walks them with no network reach, and a
    // head adopted by reference with blocks still remote bricks that
    // boot with "Blob not found". Content spaces stay lazy.
    let hydrate = super::account_state::is_account_key(&tonk_state, &params.repo).await;
    let pulled = if hydrate {
        tonk_state
            .reactor
            .repository(&params.repo)
            .branch(&params.branch)
            .pull()
            .download()
            .perform(&tonk_state.operator)
            .await
    } else {
        tonk_state
            .reactor
            .repository(&params.repo)
            .branch(&params.branch)
            .pull()
            .perform(&tonk_state.operator)
            .await
    };
    match pulled {
        Ok(after) => {
            log!("Pull succeeded: {}@{}", params.branch, params.repo);
            if hydrate
                && let Err(error) = super::account_state::converge_account_state(&tonk_state).await
            {
                log!("account-state convergence after pull failed: {error}");
            }
            // A pull that moved the head changed what every live view
            // over this branch shows; deliver it. Nothing else will — a
            // pull commits nothing locally, so no commit-time poll runs,
            // and a view left waiting repainted only on its next
            // unrelated poll (or a manual refresh).
            if after != before {
                let session = tonk_state
                    .reactor
                    .repository(&params.repo)
                    .branch(&params.branch)
                    .acquire(&tonk_state.operator)
                    .await;
                if let Ok(session) = session {
                    tonk_state
                        .reactor
                        .schedule_poll(std::sync::Arc::clone(&session.state));
                    tonk_state
                        .reactor
                        .run_scheduled_polls(&tonk_state.operator)
                        .await;
                }
            }
            announce_head(&params.repo, &params.branch, after.clone());
            Ok(Json(SyncResponse {
                success: true,
                disposition: SyncDisposition::Completed,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {}@{}: {e:?}", params.branch, params.repo);
            let error = sync_failure(&e);
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_failure_status(&tonk_state, &params.repo, &params.branch, &error).await;
            Err(error)
        }
    }
}

/// Push local changes to the upstream remote.
#[wasm_compat]
pub async fn push(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pushing to upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    ensure_session_authority(&state).await?;

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .push()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(_) => {
            log!("Push succeeded: {}@{}", params.branch, params.repo);
            let after = session.handle().revision();
            announce_head(&params.repo, &params.branch, after.clone());
            Ok(Json(SyncResponse {
                success: true,
                disposition: SyncDisposition::Completed,
                before: before.clone(),
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {}@{}: {e:?}", params.branch, params.repo);
            let error = sync_failure(&e);
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_failure_status(&tonk_state, &params.repo, &params.branch, &error).await;
            Err(error)
        }
    }
}

/// Classify a branch against its upstream without mutating local
/// state.
///
/// Reads the local head, fetches the upstream head read-only (no
/// merge, no push), and runs the shared classifier. A branch with
/// no upstream returns [`SyncState::NoUpstream`] rather than an
/// error, so the indicator has something to render for every
/// branch.
#[wasm_compat]
pub async fn sync_status(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncStatusResponse>, TonkWorkerError> {
    // Renewal first: it takes the write lock, so it cannot run under the
    // read lock this route then holds for its duration. The fetch below
    // presigns, so a lapsed or restarted-worker credential has to be
    // replaced before it, not by whichever drain happens to run next.
    ensure_session_authority(&state).await?;

    // Read-only: status acquires a branch and does a remote fetch but never
    // mutates `TonkState`. A read lock lets concurrent queries proceed instead
    // of blocking on the (network-bound) status request.
    let tonk_state = state.read().await;

    // A status check fires whenever a space is opened, so this is the load-time
    // hook for the topbar identity chip: re-stamp the `state:self` overlay here,
    // before any sync-state branch returns, so a freshly opened space (even a
    // paused or upstream-less one) always carries the member's identity.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    publish_self_identity(&tonk_state, &params.repo, &params.branch).await;

    // A paused replica reports `paused` and skips the upstream fetch — so a
    // status sweep can't clobber the chip's `paused` with a fresh `synced`/
    // `ahead` reading, and we don't hit the network while paused.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if !is_sync_enabled(&tonk_state, &params.repo, &params.branch).await {
        let local = tonk_state
            .reactor
            .repository(&params.repo)
            .branch(&params.branch)
            .acquire(&tonk_state.operator)
            .await
            .ok()
            .and_then(|session| session.handle().revision());
        publish_paused_status(&tonk_state, &params.repo, &params.branch).await;
        return Ok(Json(SyncStatusResponse {
            state: SyncState::NoUpstream,
            local,
            remote: None,
        }));
    }

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let handle = session.handle();
    let local = handle.revision();

    if handle.upstream().is_none() {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        publish_sync_status(
            &tonk_state,
            &params.repo,
            &params.branch,
            SyncState::NoUpstream,
        )
        .await;
        return Ok(Json(SyncStatusResponse {
            state: SyncState::NoUpstream,
            local,
            remote: None,
        }));
    }

    let remote = match handle.fetch().perform(&tonk_state.operator).await {
        Ok(remote) => remote,
        Err(e) => {
            let error = classified_service_failure(&e).unwrap_or(TonkWorkerError::Upstream {
                status: 503,
                code: Some("SYNC_UNAVAILABLE".to_string()),
                message: "synchronization is temporarily unavailable".to_string(),
            });
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_failure_status(&tonk_state, &params.repo, &params.branch, &error).await;
            return Err(error);
        }
    };

    let sync_state = SyncState::from(classify(local.as_ref(), remote.as_ref()));
    // Publish the live status to the replica's `tonk/sync` overlay so the
    // chip's subscription reflects it — the same path the HTTP response
    // carries, now also a fact the UI can subscribe to.
    //
    // Re-check the pause preference first: the enabled check at the top of this
    // route was before the (awaiting) upstream fetch, so a pause could have
    // landed in between. Now that transactions interleave with sync, publishing
    // a settled `idle`/`local` here would clobber the `paused` the pause command
    // just set. If it became paused mid-fetch, keep `paused`.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if is_sync_enabled(&tonk_state, &params.repo, &params.branch).await {
        publish_sync_status(&tonk_state, &params.repo, &params.branch, sync_state).await;
    } else {
        publish_paused_status(&tonk_state, &params.repo, &params.branch).await;
    }
    Ok(Json(SyncStatusResponse {
        state: sync_state,
        local,
        remote,
    }))
}

/// Full sync: pull then push.
#[wasm_compat]
pub async fn sync(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Syncing with upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    // Don't touch the network while offline. This is the single chokepoint every
    // sync path flows through (the per-fetch drain, the self-scheduled loop,
    // `POST /api/sync`, an SSE reconnect), so gating here stops ALL of them from
    // hammering an unreachable upstream — an offline branch would otherwise
    // retry `handle.fetch()` on every tick, re-queue on failure, and retry
    // again. Stamp `offline` so the chip reflects the disconnect and report
    // success (nothing to reconcile until connectivity returns; any traffic or
    // the page's `online` event restarts real syncing).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if crate::worker::offline() {
        // Read lock — the overlay stamp goes through the reactor (its own
        // locks); a write lock here would block concurrent reads, same as
        // `mark_offline`, which stamps the identical status under `read()`.
        let tonk_state = state.read().await;
        publish_sync_status_attr(
            &tonk_state,
            &params.repo,
            &params.branch,
            tonk_schema::Replica::offline_status(),
        )
        .await;
        log!("sync of {}/{} skipped: offline", params.repo, params.branch);
        return Ok(Json(SyncResponse {
            success: true,
            disposition: SyncDisposition::Offline,
            before: None,
            after: None,
            error: None,
        }));
    }

    // Honor the durable pause preference at the single chokepoint every sync
    // path flows through (the in-page interval coordinator, the background
    // sweep, a manual sync all call `/sync`). A paused replica neither pulls
    // nor pushes; we re-stamp `paused` so a status check that raced doesn't
    // leave a stale `pending` on the chip, and report success (nothing to do).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk_state = state.read().await;
        if !super::account_state::is_account_key(&tonk_state, &params.repo).await
            && !is_sync_enabled(&tonk_state, &params.repo, &params.branch).await
        {
            drop(tonk_state);
            let tonk_state = state.write().await;
            publish_paused_status(&tonk_state, &params.repo, &params.branch).await;
            log!("sync of {}/{} skipped: paused", params.repo, params.branch);
            return Ok(Json(SyncResponse {
                success: true,
                disposition: SyncDisposition::Paused,
                before: None,
                after: None,
                error: None,
            }));
        }
    }

    // Flip the chip to `pending` for the duration. This runs in its OWN brief
    // write-lock scope that drops before the long pull/push lock below, so the
    // overlay write + subscription re-poll reach the chip *before* the sync
    // begins (a mid-sync publish wouldn't — the chip's re-poll needs the read
    // lock the long write lock holds). The settled status is published by the
    // status check the controller runs after `/sync` returns.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk_state = state.write().await;
        publish_sync_status_attr(
            &tonk_state,
            &params.repo,
            &params.branch,
            tonk_schema::Replica::pending_status(),
        )
        .await;
    }

    // Everything above this point is local — an offline or paused branch
    // returns without touching the network — so renewal waits until a
    // remote operation is actually going to happen. It has to happen
    // before the read lock below, which is held across both directions.
    ensure_session_authority(&state).await?;

    // A READ lock, not a write lock. Pull/push reach the reactor and operator
    // by shared reference (the reactor owns its own interior locks for the
    // branch cache), so they don't need exclusive `TonkState`. Holding only a
    // read lock lets status checks and the pause command interleave during the
    // network-bound sync — the whole reason sync used to feel unresponsive and
    // pause couldn't land mid-sync was the write lock serializing everything.
    //
    // Dropping to a read lock means a local commit can now race the pull on the
    // same branch. That's safe since `tonk-2026-06-21`: `pull` publishes its
    // merged head CAS'd against the version it merged from, so a racing commit
    // makes the pull fail with a version mismatch instead of silently dropping
    // the commit. We recover here by refreshing the branch head and retrying —
    // the retry reuses the blocks the failed attempt already fetched.
    let tonk_state = state.read().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    // Pull with bounded refresh-and-retry on a head that moved under us.
    let mut after_pull = None;
    let mut pull_error: Option<TonkWorkerError> = None;
    // See the pull handler: authorization-bearing branches hydrate.
    let hydrate = super::account_state::is_account_key(&tonk_state, &params.repo).await;
    for attempt in 0..SYNC_RETRY_LIMIT {
        let pulled = if hydrate {
            tonk_state
                .reactor
                .repository(&params.repo)
                .branch(&params.branch)
                .pull()
                .download()
                .perform(&tonk_state.operator)
                .await
        } else {
            tonk_state
                .reactor
                .repository(&params.repo)
                .branch(&params.branch)
                .pull()
                .perform(&tonk_state.operator)
                .await
        };
        match pulled {
            Ok(after) => {
                log!("Pull succeeded: {}@{}", params.branch, params.repo);
                if hydrate
                    && let Err(error) =
                        super::account_state::converge_account_state(&tonk_state).await
                {
                    log!("account-state convergence after sync pull failed: {error}");
                }
                after_pull = Some(after);
                pull_error = None;
                break;
            }
            Err(e) if is_head_moved(&e) && attempt + 1 < SYNC_RETRY_LIMIT => {
                // A commit advanced the head while we merged. Refresh the
                // handle's view of it and retry from the now-current snapshot.
                log!(
                    "Pull raced a commit on {}@{} (attempt {}); refreshing",
                    params.branch,
                    params.repo,
                    attempt + 1
                );
                if let Err(refresh_err) = session.handle().refresh(&tonk_state.operator).await {
                    log!("refresh after raced pull failed: {refresh_err:?}");
                    pull_error = Some(TonkWorkerError::Internal(
                        "failed to refresh local branch after a concurrent update".to_string(),
                    ));
                    break;
                }
            }
            Err(e) => {
                log!("Pull failed: {}@{}: {e:?}", params.branch, params.repo);
                pull_error = Some(sync_failure(&e));
                break;
            }
        }
    }
    if let Some(error) = pull_error {
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        publish_failure_status(&tonk_state, &params.repo, &params.branch, &error).await;
        return Err(error);
    }
    let after_pull = after_pull.flatten();
    // A pull that moved the head changed what every live view over this
    // branch shows; deliver it now rather than after the push settles —
    // a refused or empty push left views waiting on a repaint that only
    // a manual refresh provided.
    if after_pull.is_some() && after_pull != before {
        tonk_state
            .reactor
            .schedule_poll(std::sync::Arc::clone(&session.state));
        tonk_state
            .reactor
            .run_scheduled_polls(&tonk_state.operator)
            .await;
    }

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .push()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(_) => {
            log!("Push succeeded: {}@{}", params.branch, params.repo);
            let after = session.handle().revision();
            announce_head(&params.repo, &params.branch, after.clone());
            // Settle the chip: re-classify against the upstream and publish the
            // resolved status (e.g. `synced`), or it stays stuck on `pending`/
            // `syncing…` — the `sync` op only flipped it to `pending` at the
            // start. Done per-branch as this one finishes, so a slow branch
            // never pins another's chip.
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_settled_status(&tonk_state, &params.repo, &params.branch).await;
            Ok(Json(SyncResponse {
                success: true,
                disposition: SyncDisposition::Completed,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {}@{}: {e:?}", params.branch, params.repo);
            // Still settle the chip — a failed push leaves us `ahead`, not
            // `pending`; classify so the chip reflects reality.
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_settled_status(&tonk_state, &params.repo, &params.branch).await;
            let _ = (before, after_pull);
            let error = sync_failure(&e);
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_failure_status(&tonk_state, &params.repo, &params.branch, &error).await;
            Err(error)
        }
    }
}

/// The repositories owed a sync sweep, in two sets.
///
/// The service worker owns *what* needs syncing; the page only polls *when*
/// (`POST /api/sync`). A commit enqueues its repo in `dirty` (from the transact
/// handler, where the route is known); a successful drain clears it. A failed
/// sweep moves it to `retrying`. The pull side is not tracked — [`drain`] pulls
/// every currently-open repository so a read-only viewer receives upstream
/// edits without ever committing, so both sets are purely push-priority hints.
///
/// The split exists because the drain gate reads `dirty` (and only `dirty`) to
/// decide whether to bypass its quiet interval, and "this failed, try again"
/// must never be mistaken for "the user has un-pushed work". See
/// [`requeue`](Self::requeue).
///
/// Interior-mutable behind its own lock so enqueuing on commit doesn't contend
/// with the outer `TonkState` lock.
#[derive(Default)]
pub struct SyncQueue {
    /// Repo name → most recent commit instant (for activity priority).
    dirty: std::sync::Mutex<HashMap<String, f64>>,
    /// Repo name → instant its last sweep failed. Held apart from `dirty` on
    /// purpose: see [`requeue`](Self::requeue).
    retrying: std::sync::Mutex<HashMap<String, f64>>,
}

impl SyncQueue {
    /// Record that `repo` has un-pushed local commits. `now` is a caller-
    /// supplied monotonic-ish timestamp (the SW stamps `Date.now()` at the
    /// event boundary — the reactor's deterministic paths can't read a clock).
    pub fn mark_dirty(&self, repo: &str, now: f64) {
        if let Ok(mut dirty) = self.dirty.lock() {
            dirty.insert(repo.to_owned(), now);
        }
    }

    /// How many repos have un-pushed local commits waiting.
    ///
    /// Read by the drain gate: pending local work bypasses the quiet
    /// interval, so a tab the user just switched away from still pushes
    /// its last edit at the active cadence instead of sitting on it for a
    /// minute. The count (rather than a bare flag) is what the bypass log
    /// reports. Cheap on the idle path: an empty dirty set is a lock and a
    /// `len`, and a drain with nothing to push short-circuits before the
    /// network.
    ///
    /// Counts `dirty` only — a repo awaiting retry is NOT pending local work
    /// for this purpose. See [`requeue`](Self::requeue).
    pub fn dirty_count(&self) -> usize {
        self.dirty.lock().map(|dirty| dirty.len()).unwrap_or(0)
    }

    /// Every repo owed a sweep — dirty first (most-recently-active first),
    /// then the retry set. Both are cleared: a repo that fails again is put
    /// back by [`requeue`](Self::requeue).
    fn drain_pending(&self) -> Vec<String> {
        fn take(slot: &std::sync::Mutex<HashMap<String, f64>>) -> Vec<String> {
            let Ok(mut map) = slot.lock() else {
                return Vec::new();
            };
            let mut repos: Vec<(String, f64)> = map.drain().collect();
            // Descending by timestamp: an active editor's repo syncs before
            // idle background repos.
            repos.sort_by(|a, b| b.1.total_cmp(&a.1));
            repos.into_iter().map(|(repo, _)| repo).collect()
        }
        let mut repos = take(&self.dirty);
        repos.extend(take(&self.retrying));
        repos
    }

    /// Queue `repo` for a retry after a failed sweep.
    ///
    /// Deliberately NOT `mark_dirty`. `sync_repository` returns `Err` for any
    /// branch whose pull or push reported failure — an unreachable relay, an
    /// expired permit, a 5xx, a non-fast-forward push — none of which mean the
    /// repo has un-pushed local work. Folding those back into `dirty` latched
    /// the quiet-interval bypass on permanently: one repo failing against an
    /// erroring remote re-marked itself on every pass, so a hidden tab polled
    /// at the active cadence forever, burning request quota on calls that were
    /// already failing. The retry set keeps the repo in the sweep order
    /// without feeding [`dirty_count`](Self::dirty_count), so a repo that only
    /// ever fails backs off to the hidden interval, while a genuinely new
    /// local commit (which calls `mark_dirty`) re-earns the bypass.
    fn requeue(&self, repo: &str, now: f64) {
        if let Ok(mut retrying) = self.retrying.lock() {
            retrying.insert(repo.to_owned(), now);
        }
    }

    /// Drop `repo` from the queue without touching any other entry.
    /// Called when a space is removed: a stamp left behind would
    /// survive the reactor's [`evict`](crate::Reactor::evict) and, on the
    /// next [`drain_sync`], get folded into the union that `sync_repository`
    /// reconciles — re-acquiring (resurrecting) the just-removed repo.
    ///
    /// Wasm-gated: its only caller, `remove_space_inner`, is service-worker
    /// scoped, so a native build never reaches it (native clippy flags it
    /// dead code otherwise).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn forget(&self, repo: &str) {
        if let Ok(mut dirty) = self.dirty.lock() {
            dirty.remove(repo);
        }
        if let Ok(mut retrying) = self.retrying.lock() {
            retrying.remove(repo);
        }
    }
}

/// Drain the sync work-queue: reconcile every repository that has un-pushed
/// commits (the dirty set) plus every currently-open repository (so a viewer
/// pulls upstream edits without committing). The union is synced through the
/// per-repo [`sync_repository`] sweep, which pulls+pushes each upstream branch
/// and honors the durable pause preference.
///
/// Five callers reach this: the per-fetch `schedule_sync_drain` (debounced,
/// generation-ticketed — the path the `<tonk-host>` idle poll to `POST
/// /api/sync` rides on, since `on_fetch` schedules it), the SW's
/// self-scheduled loop tick, `onconnectivity` (immediate reconcile on
/// regaining connectivity), `onvisibility` (immediate reconcile on a page
/// becoming visible), and the SW's own Background-Sync `onsync` (a discrete
/// OS event with no fetch to hook, so it drains directly). Every one of these
/// passes through the same `may_drain` gate first, so they never overlap.
/// Branches are synced per-repo; repos run sequentially here (the reactor
/// serializes branch state anyway), priority-ordered by activity.
pub async fn drain_sync(state: &AppState) {
    // One drain at a time, and concurrent triggers coalesce instead of
    // queueing: a keepalive beat arriving while a transact-triggered
    // drain runs would only repeat the same sweep, and letting them
    // interleave is how branch commits tear (session rotation and the
    // account ensure both write without a per-branch lock in dialog).
    // Whatever this beat would have pushed, the next one covers.
    static DRAIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Ok(_serialized) = DRAIN.try_lock() else {
        return;
    };

    // Before anything presigns: the operator's delegation expires, and a
    // drain is the regular beat this worker has to notice that on.
    // Best-effort here — a failed rotation must not take the drain down.
    // The current session keeps working until it lapses, the remote
    // boundaries below renew synchronously anyway, and the next drain
    // tries again.
    if let Err(error) = ensure_session_authority(state).await {
        log!("session authority renewal failed, keeping the current one: {error}");
    }

    // Dirty repos first (push priority), then repos owed a retry, then every
    // other open repo (pull).
    let now = current_millis();
    let pending = {
        let tonk = state.read().await;
        tonk.sync_queue.drain_pending()
    };

    // Every currently-open repository — the pull population. Read the reactor's
    // cached repo map; a repo only appears once acquired, which every rendered
    // space has done.
    let open: Vec<String> = {
        let tonk = state.read().await;
        tonk.reactor.repos().read().keys().cloned().collect()
    };

    // Union, pending-first, de-duplicated while preserving order.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let order: Vec<String> = pending
        .into_iter()
        .chain(open)
        .filter(|repo| seen.insert(repo.clone()))
        .collect();

    for repo in &order {
        if let Err(e) = sync_repository(state, repo).await {
            // Push didn't fully land — re-mark so the next heartbeat retries.
            log!("drain_sync: {repo} did not fully reconcile: {e}");
            let tonk = state.read().await;
            tonk.sync_queue.requeue(repo, now);
        }
    }

    // The account rides profile main, which no repository — and so no
    // reactor entry — represents in the pull population above. Sweep it
    // explicitly each drain, unless a dirty mark already routed it
    // through `sync_repository` — sweeping twice per heartbeat is the
    // exact duplication the dedicated path exists to avoid.
    let account_already_swept = {
        let tonk = state.read().await;
        let mut swept = false;
        for repo in &order {
            if super::account_state::is_account_key(&tonk, repo).await {
                swept = true;
                break;
            }
        }
        swept
    };
    if !account_already_swept {
        let tonk = state.read().await;
        let (status, swept) = super::account_state::ensure_account_state_swept(&tonk).await;
        if status != tonk_account::AccountStateStatus::Unconfigured
            && let Err(error) = swept
        {
            log!("drain_sync: account state did not fully reconcile: {error}");
        }
    }
}

/// Make sure every credential this worker is about to sign with is still
/// good, rotating the operator and replaying every guest invite onto it
/// when anything is due.
///
/// Rotation replaces the operator key, not just its delegation: the
/// certificate store is content-addressed with no delete, and its chain
/// walk never consults the clock, so a re-minted delegation filed under
/// the same audience would sit beside the lapsed one and be chosen about
/// half the time. A new key means a new audience and no ambiguity.
///
/// That is also why one guest coming due rotates for all of them. The
/// operator is shared by every mounted repository, so a new audience
/// orphans every guest chain at once, and the only safe rotation is the
/// one that re-mints the whole set. Durable spaces need no replay: they
/// reach the operator through `space -> root -> device -> operator`,
/// whose last hop `session::open` re-mints anyway.
///
/// The replacement is built over the state's existing storage pool, so
/// every repository and branch handle the reactor has cached stays
/// valid — the operator changes, the spaces underneath do not.
///
/// Order matters at the end: every replacement chain is retained first,
/// every record is written second, and only then does the state adopt
/// the new operator. A failure anywhere leaves the current operator and
/// the current records exactly as they were — the chains retained for an
/// audience nothing points at are inert, and a record still naming the
/// previous operator reads as due on the next attempt.
pub(crate) async fn ensure_session_authority(state: &AppState) -> Result<(), TonkWorkerError> {
    let now = crate::session::now();
    let (profile, storage, expires_at) = {
        let tonk = state.read().await;
        (
            tonk.profile.clone(),
            tonk.storage.clone(),
            tonk.session_expires_at,
        )
    };

    if !crate::session::needs_renewal(expires_at, now) {
        return Ok(());
    }

    // Mint outside the lock — nothing else may proceed while a write
    // lock is held, and this signs. The operator KEY is stable, so this
    // replaces the delegation authorizing it, not the audience.
    let session = crate::session::rotate(&profile, &storage).await?;

    let mut tonk = state.write().await;
    // A concurrent drain may have rotated while this one was minting.
    // Theirs is no worse than ours, and adopting both would retire a
    // session that requests are already presenting.
    if tonk.session_expires_at != expires_at {
        return Ok(());
    }

    // No guest replay: a guest's chain is addressed to the operator, and
    // the operator's DID no longer moves, so a renewed delegation leaves
    // every guest chain exactly as valid as it was. Replaying invites
    // here was the only consumer of a guest's retained invite URL.
    tonk.operator = session.operator;
    tonk.session_expires_at = session.expires_at;
    Ok(())
}

/// `POST /api/sync` — an external sync poke.
///
/// Deliberately does NO work of its own: the drain is scheduled by the SW's
/// `on_fetch`, which runs `schedule_sync_drain` for EVERY request (debounced,
/// generation-ticketed) before routing. So merely *reaching* this route already
/// enqueued a coalesced drain on the event's `wait_until`. Draining here too
/// would fire a second, un-debounced drain and stack it on the scheduled one —
/// so the poke participates in the same scheduling machinery precisely by
/// leaving the drain to `on_fetch`.
///
/// The steady cadence is SW-owned (the self-scheduled sync loop in
/// `worker.rs`); this route remains for debug tooling and for the page's
/// keepalive, which rides it to keep the worker alive.
///
/// It is NOT a reliable way to force an immediate reconcile. The drain it
/// schedules goes through `may_drain` like every other, so on a hidden page
/// with nothing to push it can be refused for up to the hidden interval —
/// a poke asks; it does not compel. A caller that genuinely needs a prompt
/// pull should make the page visible (`onvisibility` drains directly) or
/// land a local commit, which bypasses the quiet interval. Always `200`
/// either way: the poke is fire-and-forget and the response says nothing
/// about whether a drain ran.
#[wasm_compat]
pub async fn drain() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

/// A millisecond wall-clock stamp for activity priority. `Date.now()` in the SW
/// event context (not the reactor's deterministic paths). Native builds (tests)
/// have no clock dependency here, so they return 0 — priority ordering is moot
/// off-wasm.
fn current_millis() -> f64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        js_sys::Date::now()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::UpstreamConfiguration;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn with_upstream() -> BranchConfiguration {
        BranchConfiguration {
            upstream: Some(UpstreamConfiguration {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
            revision: None,
        }
    }

    fn without_upstream() -> BranchConfiguration {
        BranchConfiguration {
            upstream: None,
            revision: None,
        }
    }

    #[dialog_common::test]
    fn it_selects_only_branches_with_an_upstream() {
        let branches = HashMap::from([
            ("main".to_string(), with_upstream()),
            ("scratch".to_string(), without_upstream()),
        ]);
        assert_eq!(branches_to_sync(&branches), vec!["main".to_string()]);
    }

    #[dialog_common::test]
    fn it_returns_branches_sorted_for_a_stable_sweep_order() {
        let branches = HashMap::from([
            ("zeta".to_string(), with_upstream()),
            ("alpha".to_string(), with_upstream()),
        ]);
        assert_eq!(
            branches_to_sync(&branches),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[dialog_common::test]
    fn it_returns_empty_when_no_branch_has_an_upstream() {
        let branches = HashMap::from([("main".to_string(), without_upstream())]);
        assert!(branches_to_sync(&branches).is_empty());
    }

    fn authorization_failure(error: AuthorizeError) -> crate::reactor::ReactorError {
        crate::reactor::ReactorError::Pull(PullError::Publish(PublishError::Authorization(error)))
    }

    #[dialog_common::test]
    fn it_classifies_revocation_from_the_typed_reason() {
        let error = sync_failure(&authorization_failure(AuthorizeError::Revoked {
            subject: dialog_capability::did!("key:zSubject"),
        }));
        assert!(matches!(
            error,
            TonkWorkerError::Upstream {
                status: 403,
                code: Some(ref code),
                ..
            } if code == "CREDENTIAL_REVOKED"
        ));
    }

    #[dialog_common::test]
    fn it_maps_revocation_registry_outages_to_sync_unavailable() {
        let error = sync_failure(&authorization_failure(AuthorizeError::Unavailable {
            detail: "revocation registry offline".to_string(),
        }));
        assert!(matches!(
            error,
            TonkWorkerError::Upstream {
                status: 503,
                code: Some(ref code),
                ..
            } if code == "SYNC_UNAVAILABLE"
        ));
    }

    #[dialog_common::test]
    fn it_counts_a_repo_as_dirty_until_the_drain_takes_it() {
        let queue = SyncQueue::default();
        assert_eq!(queue.dirty_count(), 0, "a fresh queue holds nothing");

        queue.mark_dirty("notes", 1_000.0);
        assert_eq!(queue.dirty_count(), 1, "a commit marks its repo dirty");

        assert_eq!(queue.drain_pending(), vec!["notes".to_string()]);
        assert_eq!(
            queue.dirty_count(),
            0,
            "the drain takes the repo, so nothing is pending after it",
        );
    }

    #[dialog_common::test]
    fn it_orders_dirty_repos_by_most_recent_commit() {
        let queue = SyncQueue::default();
        queue.mark_dirty("stale", 1_000.0);
        queue.mark_dirty("active", 9_000.0);
        assert_eq!(
            queue.drain_pending(),
            vec!["active".to_string(), "stale".to_string()],
            "the repo the user is editing syncs first",
        );
    }

    /// The latch this split exists to prevent: `sync_repository` returns `Err`
    /// for any relay failure — unreachable host, expired permit, 5xx,
    /// non-fast-forward push — none of which mean un-pushed local work. When
    /// a failure re-marked the repo dirty, one repo failing against a broken
    /// remote held the quiet-interval bypass open forever, so a hidden tab
    /// polled at the active cadence indefinitely against a remote that was
    /// already erroring.
    #[dialog_common::test]
    fn it_stops_counting_a_repo_that_only_ever_fails() {
        let queue = SyncQueue::default();
        queue.mark_dirty("notes", 1_000.0);

        // Ten drains, each one failing and requeueing the repo.
        for pass in 0..10 {
            let now = 2_000.0 + f64::from(pass) * 2_000.0;
            assert_eq!(
                queue.drain_pending(),
                vec!["notes".to_string()],
                "a failing repo must keep being retried",
            );
            queue.requeue("notes", now);
            assert_eq!(
                queue.dirty_count(),
                0,
                "a repo awaiting retry is not pending local work",
            );
        }
    }

    #[dialog_common::test]
    fn it_lets_a_new_commit_re_earn_the_bypass_after_a_failure() {
        let queue = SyncQueue::default();
        queue.mark_dirty("notes", 1_000.0);
        queue.drain_pending();
        queue.requeue("notes", 2_000.0);
        assert_eq!(queue.dirty_count(), 0);

        // The user edits again: genuinely un-pushed new work, bypass earned.
        queue.mark_dirty("notes", 3_000.0);
        assert_eq!(
            queue.dirty_count(),
            1,
            "a new local commit still bypasses the quiet interval",
        );
    }

    #[dialog_common::test]
    fn it_sweeps_dirty_repos_before_retries() {
        let queue = SyncQueue::default();
        queue.requeue("failing", 9_000.0);
        queue.mark_dirty("edited", 1_000.0);
        assert_eq!(
            queue.drain_pending(),
            vec!["edited".to_string(), "failing".to_string()],
            "un-pushed local work has push priority over a retry",
        );
    }

    // `forget` is service-worker scoped (wasm-gated), so this one is too.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    fn it_forgets_a_removed_repo_from_both_sets() {
        let queue = SyncQueue::default();
        queue.mark_dirty("gone", 1_000.0);
        queue.requeue("gone", 1_000.0);
        queue.forget("gone");
        assert_eq!(queue.dirty_count(), 0);
        assert!(
            queue.drain_pending().is_empty(),
            "a removed space must not be resurrected by a leftover stamp",
        );
    }
}

/// Overlay tests — wasm-only (needs IndexedDB / branch IO via the service-worker
/// host).
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod overlay_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::{petname, prelude::DidExt as _};

    use crate::router::{api_router_with_state, tests::put_repo, tests::test_state};

    #[dialog_common::test]
    async fn it_stamps_the_self_identity_overlay_on_load() {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let key = put_repo(&app, "chip-space").await;
        let tonk = state.read().await;
        super::publish_self_identity(&tonk, &key, "main").await;

        let session = tonk
            .reactor
            .repository(&key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();

        let entity: dialog_artifacts::Entity =
            tonk_schema::Replica::SELF_STATE_HERE.parse().unwrap();
        let rows: Vec<tonk_schema::ProfileIdentity> = session
            .handle()
            .query()
            .select(Query::<tonk_schema::ProfileIdentity> {
                this: Term::from(entity),
                did: Term::var("did"),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();

        assert_eq!(rows.len(), 1, "expected one state:self row");
        assert_eq!(
            rows[0].name.0,
            petname(&tonk.profile.did()),
            "name must be the petname when no override is set",
        );
        assert_eq!(
            rows[0].did.0,
            tonk.profile.did().this(),
            "did must match the profile DID for the sigil",
        );
    }
}

/// Session renewal tests — wasm-only, because they need a real
/// certificate store to mint against.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod renewal_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use super::ensure_session_authority;
    use crate::router::tests::test_state;
    use crate::router::{AppState, api_router_with_state};

    async fn operator_did(state: &AppState) -> String {
        state.read().await.operator.did().to_string()
    }

    /// The operator key derives from a CONSTANT context, so renewal
    /// replaces the delegation authorizing it and never the key itself.
    ///
    /// This is what lets a chain addressed to the operator, such as a
    /// guest's invite hop, survive renewal. Deriving a fresh key each
    /// time invalidated those chains twice a day, which made a retained
    /// bearer secret the only way to mint replacements.
    #[dialog_common::test]
    async fn it_keeps_the_operator_did_across_renewal() {
        let (_app, state, _lsp) = api_router_with_state(test_state().await);
        let before = operator_did(&state).await;

        ensure_session_authority(&state).await.unwrap();
        ensure_session_authority(&state).await.unwrap();

        assert_eq!(
            before,
            operator_did(&state).await,
            "renewal re-mints the delegation, not the operator key",
        );
    }
}
