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
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::{SyncState, classify};

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
    // A replica paused mid-sync keeps `paused` — don't classify or hit the
    // network for it.
    if !is_sync_enabled(tonk, repo, branch).await {
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

/// Whether `error` is the "branch head moved under us" version mismatch a pull
/// raises (since `tonk-2026-06-21`) when a commit advanced the head while it
/// merged — the signal to refresh and retry rather than fail. Matched on the
/// rendered error: the mismatch is nested several error types deep
/// (`ReactorError::Pull(PullError::Publish(PublishError::VersionMismatch))`),
/// and the rendered chain carries the distinctive "Version mismatch" text from
/// the leaf without the worker importing the whole dialog error hierarchy.
fn is_head_moved(error: &crate::reactor::ReactorError) -> bool {
    error.to_string().contains("Version mismatch")
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
    // Honor the durable pause preference: a paused replica skips the whole
    // sweep (no pull, no push) until resumed. This is the gate localStorage
    // couldn't provide — the SW can read this branch fact. Keyed on the
    // content branch, where the chip writes the preference.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk = state.read().await;
        if !is_sync_enabled(&tonk, repo, "main").await {
            log!("background sync of '{repo}' skipped: paused");
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
        // The `/sync` route reports pull/push failures as a 200 with
        // `success: false` (a non-fast-forward push after divergence,
        // a fetch failure), so a returned `Ok` is not proof the sync
        // landed — inspect `success` too.
        match sync(State(state.clone()), Path(params)).await {
            Ok(Json(response)) if !response.success => {
                let detail = response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string());
                log!("background sync of {repo}/{branch} did not complete: {detail}");
                failed = true;
            }
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

/// Response for sync operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether the sync operation succeeded.
    pub success: bool,
    /// Local branch revision *before* the sync ran. `None` when
    /// the branch had no commits at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Revision>,
    /// Local branch revision *after* the sync. `None` when the
    /// branch still has no commits, or when the operation failed
    /// before producing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Revision>,
    /// Error message if sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

    // A write lock, unlike `sync`. Dropping to a read lock would let a commit
    // race the pull, and this route — unlike `sync` — has no refresh-and-retry
    // loop to recover from the resulting head CAS failure. It is not on a hot
    // path (`/sync` is what the app calls), so excluding writers costs nothing.
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
        .pull()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(after) => {
            log!("Pull succeeded: {}@{}", params.branch, params.repo);
            announce_head(&params.repo, &params.branch, after.clone());
            Ok(Json(SyncResponse {
                success: true,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {}@{}: {e:?}", params.branch, params.repo);
            Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(e.to_string()),
            }))
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

    // Write lock — see [`pull`]. Same no-retry caveat, same cold path.
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
                before: before.clone(),
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {}@{}: {e:?}", params.branch, params.repo);
            Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Sync-state of a branch relative to its upstream.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    /// How the local head relates to the upstream head.
    pub state: SyncState,
    /// Local branch revision, or `null` if it has no commits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<Revision>,
    /// Upstream branch revision as last fetched, or `null` if the
    /// upstream has no commits (or none is configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<Revision>,
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
            // A remote is configured but unreachable — publish `offline` so
            // the chip reflects it (distinct from `local`, no remote at all).
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            publish_sync_status_attr(
                &tonk_state,
                &params.repo,
                &params.branch,
                tonk_schema::Replica::offline_status(),
            )
            .await;
            return Err(TonkWorkerError::Internal(e.to_string()));
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

    // Honor the durable pause preference at the single chokepoint every sync
    // path flows through (the in-page interval coordinator, the background
    // sweep, a manual sync all call `/sync`). A paused replica neither pulls
    // nor pushes; we re-stamp `paused` so a status check that raced doesn't
    // leave a stale `pending` on the chip, and report success (nothing to do).
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk_state = state.read().await;
        if !is_sync_enabled(&tonk_state, &params.repo, &params.branch).await {
            publish_paused_status(&tonk_state, &params.repo, &params.branch).await;
            log!("sync of {}/{} skipped: paused", params.repo, params.branch);
            return Ok(Json(SyncResponse {
                success: true,
                before: None,
                after: None,
                error: None,
            }));
        }
    }

    // Flip the chip to `pending` before the sync begins, so the overlay write +
    // subscription re-poll reach the chip up front rather than mid-pull. The
    // settled status is published by the status check the controller runs after
    // `/sync` returns. A read lock: the stamp lands in the branch's session
    // overlay, which has its own interior lock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let tonk_state = state.read().await;
        publish_sync_status_attr(
            &tonk_state,
            &params.repo,
            &params.branch,
            tonk_schema::Replica::pending_status(),
        )
        .await;
    }

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
    let mut pull_error = None;
    for attempt in 0..SYNC_RETRY_LIMIT {
        match tonk_state
            .reactor
            .repository(&params.repo)
            .branch(&params.branch)
            .pull()
            .perform(&tonk_state.operator)
            .await
        {
            Ok(after) => {
                log!("Pull succeeded: {}@{}", params.branch, params.repo);
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
                    pull_error = Some(refresh_err.to_string());
                    break;
                }
            }
            Err(e) => {
                log!("Pull failed: {}@{}: {e:?}", params.branch, params.repo);
                pull_error = Some(format!("Pull failed: {e}"));
                break;
            }
        }
    }
    if let Some(error) = pull_error {
        return Ok(Json(SyncResponse {
            success: false,
            before: before.clone(),
            after: before,
            error: Some(error),
        }));
    }
    let after_pull = after_pull.flatten();

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
            Ok(Json(SyncResponse {
                success: false,
                before,
                after: after_pull,
                error: Some(format!("Push failed: {e}")),
            }))
        }
    }
}

/// The set of repositories that have local commits not yet pushed.
///
/// The service worker owns *what* needs syncing; the page only polls *when*
/// (`POST /api/sync`). A commit enqueues its repo here (from the transact
/// handler, where the route is known); a successful drain clears it. The pull
/// side is not tracked — [`drain`] pulls every currently-open repository so a
/// read-only viewer receives upstream edits without ever committing, so the
/// dirty set is purely a push-priority hint.
///
/// Interior-mutable behind its own lock so enqueuing on commit doesn't contend
/// with the outer `TonkState` lock.
#[derive(Default)]
pub struct SyncQueue {
    /// Repo name → most recent commit instant (for activity priority).
    dirty: std::sync::Mutex<HashMap<String, f64>>,
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

    /// The dirty repos, most-recently-active first.
    fn drain_dirty(&self) -> Vec<String> {
        let Ok(mut dirty) = self.dirty.lock() else {
            return Vec::new();
        };
        let mut repos: Vec<(String, f64)> = dirty.drain().collect();
        // Descending by timestamp: an active editor's repo syncs before idle
        // background repos.
        repos.sort_by(|a, b| b.1.total_cmp(&a.1));
        repos.into_iter().map(|(repo, _)| repo).collect()
    }

    /// Re-mark `repo` dirty after a failed drain so the next pass retries it.
    fn requeue(&self, repo: &str, now: f64) {
        self.mark_dirty(repo, now);
    }

    /// Drop `repo` from the dirty set without touching any other entry.
    /// Called when a space is removed: a dirty stamp left behind would
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
    }
}

/// Drain the sync work-queue: reconcile every repository that has un-pushed
/// commits (the dirty set) plus every currently-open repository (so a viewer
/// pulls upstream edits without committing). The union is synced through the
/// per-repo [`sync_repository`] sweep, which pulls+pushes each upstream branch
/// and honors the durable pause preference.
///
/// Two callers reach this: the per-fetch `schedule_sync_drain` (debounced,
/// generation-ticketed — the path the `<tonk-host>` idle poll to `POST
/// /api/sync` rides on, since `on_fetch` schedules it) and the SW's own
/// Background-Sync `onsync` (a discrete OS event with no fetch to hook, so it
/// drains directly). Branches are synced per-repo; repos run sequentially here
/// (the reactor serializes branch state anyway), priority-ordered by activity.
pub async fn drain_sync(state: &AppState) {
    // Dirty repos first (push priority), then every other open repo (pull).
    let now = current_millis();
    let dirty = {
        let tonk = state.read().await;
        tonk.sync_queue.drain_dirty()
    };

    // Every currently-open repository — the pull population. Read the reactor's
    // cached repo map; a repo only appears once acquired, which every rendered
    // space has done.
    let open: Vec<String> = {
        let tonk = state.read().await;
        tonk.reactor.repos().read().keys().cloned().collect()
    };

    // Union, dirty-first, de-duplicated while preserving order.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let order: Vec<String> = dirty
        .into_iter()
        .chain(open)
        .filter(|repo| seen.insert(repo.clone()))
        .collect();

    for repo in order {
        if let Err(e) = sync_repository(state, &repo).await {
            // Push didn't fully land — re-mark so the next heartbeat retries.
            log!("drain_sync: {repo} did not fully reconcile: {e}");
            let tonk = state.read().await;
            tonk.sync_queue.requeue(&repo, now);
        }
    }
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
/// `worker.rs`); this route remains for explicit pokes (debug tooling, a
/// page transition that wants an immediate reconcile). Always `200`.
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
            .with(session.overlay())
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
