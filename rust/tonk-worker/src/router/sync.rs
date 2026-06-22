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
/// The status is a transient observation (idle / push / pull / offline), so
/// it goes to the overlay — never persisted, never replicated — and folds
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
    use std::sync::Arc;
    use tonk_schema::{Replica, ReplicaSyncStatus};

    let Ok(entity) = Replica::SYNC_STATE_HERE.parse() else {
        return;
    };
    let stamp = ReplicaSyncStatus::new(entity, Replica::sync_status_attr(state));

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
    session.state.assert_overlay(stamp);
    tonk.reactor.schedule_poll(Arc::clone(&session.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Tag prefix every durable background sync carries; the repository
/// name follows the colon. A `sync` event delivers only this string
/// and the worker has no notion of an "active repository", so the
/// repo identity has to ride along in the tag.
const SYNC_TAG_PREFIX: &str = "tonk-sync:";

/// Parse the repository name out of a background-sync tag.
///
/// Tags are `tonk-sync:{repo}`. Anything not matching that shape — a
/// bare or differently-prefixed tag, an empty repo — yields `None`,
/// so a malformed tag becomes a no-op rather than an error.
pub fn repo_from_sync_tag(tag: &str) -> Option<&str> {
    tag.strip_prefix(SYNC_TAG_PREFIX)
        .filter(|repo| !repo.is_empty())
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
            log!("Pull succeeded");
            announce_head(&params.repo, &params.branch, after.clone());
            Ok(Json(SyncResponse {
                success: true,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {e:?}");
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
            log!("Push succeeded");
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
            log!("Push failed: {e:?}");
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

    let remote = handle
        .fetch()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(e.to_string()))?;

    let sync_state = SyncState::from(classify(local.as_ref(), remote.as_ref()));
    // Publish the live status to the replica's `tonk/sync` overlay so the
    // chip's subscription reflects it — the same path the HTTP response
    // carries, now also a fact the UI can subscribe to.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    publish_sync_status(&tonk_state, &params.repo, &params.branch, sync_state).await;
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

    let tonk_state = state.write().await;

    let session = tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let before = session.handle().revision();

    let after_pull = match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .pull()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(after) => {
            log!("Pull succeeded");
            after
        }
        Err(e) => {
            log!("Pull failed: {e:?}");
            return Ok(Json(SyncResponse {
                success: false,
                before: before.clone(),
                after: before,
                error: Some(format!("Pull failed: {e}")),
            }));
        }
    };

    match tonk_state
        .reactor
        .repository(&params.repo)
        .branch(&params.branch)
        .push()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(_) => {
            log!("Push succeeded");
            let after = session.handle().revision();
            announce_head(&params.repo, &params.branch, after.clone());
            Ok(Json(SyncResponse {
                success: true,
                before,
                after,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {e:?}");
            Ok(Json(SyncResponse {
                success: false,
                before,
                after: after_pull,
                error: Some(format!("Push failed: {e}")),
            }))
        }
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
    fn it_parses_the_repo_from_a_well_formed_tag() {
        assert_eq!(repo_from_sync_tag("tonk-sync:home"), Some("home"));
    }

    #[dialog_common::test]
    fn it_ignores_a_tag_without_the_sync_prefix() {
        assert_eq!(repo_from_sync_tag("home"), None);
        assert_eq!(repo_from_sync_tag("other-tag"), None);
    }

    #[dialog_common::test]
    fn it_ignores_a_prefix_only_tag_with_an_empty_repo() {
        assert_eq!(repo_from_sync_tag("tonk-sync:"), None);
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
