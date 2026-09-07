//! `/transact` route — accepts a structured
//! [`TransactRequest`] and drives the reactor's
//! [`TransactionBuilder`] directly, preserving per-mutation
//! durable/transient classification through to the effects
//! evaluator.
//!
//! See `plan/transact-endpoint.md`. Unlike `/evaluate`, this
//! route bypasses tonk-notation: callers send a typed wire
//! shape so the reactor's commit pipeline knows what's
//! transient without re-querying the schema.

use ::axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_wasm_macros::wasm_compat;
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_evaluator::evaluate::CommitSummary;
use tonk_schema::claim::{Claim, TransactRequest};

use super::AppState;
use crate::TonkWorkerError;
use crate::broadcast::{LOCAL_COMMIT_CHANNEL, Notification, broadcast};
use crate::reactor::{BranchReference, ReactorError};

/// Path parameters for the repository-scoped route.
#[derive(Debug, Deserialize)]
pub struct TransactPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Path parameters for the profile-scoped route. The profile is
/// a singleton, so no `repo` segment.
#[derive(Debug, Deserialize)]
pub struct ProfileTransactPath {
    /// The branch name.
    pub branch: String,
}

/// Response body for `/transact`. Mirrors the commit-side
/// surface of [`tonk_evaluator::evaluate::EvaluateResponse`] minus
/// the query blocks: revision before/after and a claim count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactResponse {
    /// Revision of the branch before the commit, if any.
    pub revision_before: Option<Revision>,
    /// Revision of the branch after the commit.
    pub revision_after: Option<Revision>,
    /// Commit summary — currently just the number of claims
    /// (asserts + retracts) submitted to the builder. Effect
    /// evaluation may grow this once it lands.
    pub commits: CommitSummary,
}

/// `POST /api/repository/{repo}/branch/{branch}/transact`
#[wasm_compat]
pub async fn transact(
    State(state): State<AppState>,
    Path(path): Path<TransactPath>,
    client: Option<Extension<super::ClientId>>,
    lifetime: Option<Extension<crate::worker::FetchLifetime>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    log!("transact repo={}, branch={}", path.repo, path.branch);
    // A read lock on `TonkState` — concurrent transactions and syncs don't
    // serialize on the outer lock. Transactions instead serialize on the
    // per-branch transactor lock (taken inside `transact_on_branch`); sync
    // coordinates via the head CAS. So a tab's commit never blocks behind an
    // in-flight sync (the bug that made pause-mid-sync feel dead).
    let (response, transients) = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state
            .reactor
            .repository(&path.repo)
            .branch(&path.branch);
        transact_on_branch(&tonk_state, tonk_branch, body).await?
    };
    // Mark the repo dirty so the next sync drain pushes its new commits — but
    // ONLY if the commit actually moved the tree. The SW owns the sync
    // work-queue (the page only pokes `POST /api/sync` on a heartbeat); a
    // commit here is the authoritative "this repo has un-pushed changes"
    // signal, and a transact whose data didn't change has none.
    //
    // This is not hypothetical: every page load fires the transient
    // `tonk:load` site stamp through this route. Transients are retracted
    // before the durable write, so the commit lands with an IDENTICAL tree
    // hash (only the revision's `moment` ticks) — yet it used to enqueue the
    // repo, so every single reload scheduled a push of nothing.
    if response.0.revision_before.as_ref().map(|r| &r.tree)
        != response.0.revision_after.as_ref().map(|r| &r.tree)
    {
        let tonk_state = state.read().await;
        tonk_state.sync_queue.mark_dirty(&path.repo, now_millis());
    }
    announce_local_commit(&path.branch, &response.0);
    // Dispatch any transient commands (now that the state lock is
    // released, so each command's `execute` can re-acquire it) and then
    // drain the polls this request scheduled. `dispatch` always drains —
    // even with no commands — so the durable commit's scheduled poll fans
    // out. The origin is the repo + branch this commit landed in, plus the
    // client that asked, so a handler can post a page-capability effect
    // (e.g. navigation) back to it.
    let origin = super::CommandOrigin {
        repo: path.repo,
        branch: path.branch,
        client: client.map(|Extension(id)| id),
    };
    // Run the transient command providers (route stamping, invite, join, …) and
    // the post-commit poll drain WITHOUT blocking this response. A command's
    // `execute` can be slow — `tonk:load`'s route match + site stamp is ~1s on a
    // cold content branch — and the client (`<tonk-site>`) doesn't read the
    // command's result from this response: it observes the stamp through its live
    // subscription, which the command's scheduled poll broadcasts when it lands.
    // Awaiting the dispatch here serialized the iframe boot behind the command;
    // detaching it returns the commit (~40ms) immediately and lets the command
    // finish in the background under the fetch event's `waitUntil` lifetime.
    spawn_dispatch(state, origin, transients.unwrap_or_default(), lifetime).await;
    Ok(response)
}

/// `POST /api/profile/branch/{branch}/transact`
#[wasm_compat]
pub async fn transact_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileTransactPath>,
    client: Option<Extension<super::ClientId>>,
    lifetime: Option<Extension<crate::worker::FetchLifetime>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    log!("transact profile branch={}", path.branch);
    if let Some(Extension(client_id)) = &client {
        let bindings = state.read().await.view_bindings.clone();
        if bindings.read().await.contains_key(client_id) {
            return Err(TonkWorkerError::Forbidden(
                "sealed guests may not write the profile branch".into(),
            ));
        }
    }
    let (response, transients) = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
        transact_on_branch(&tonk_state, tonk_branch, body).await?
    };
    announce_local_commit(&path.branch, &response.0);
    // Profile-branch commits carry an empty `repo` origin: the profile
    // repository is not in the named-repo namespace, and no command
    // dispatched here loads an origin repository by name. The originating
    // client is carried so the join command can post its navigate message
    // back to the exact tab that asked. `dispatch` always drains the
    // scheduled polls, even with no transients, so the durable commit fans
    // out.
    let origin = super::CommandOrigin {
        repo: String::new(),
        branch: path.branch,
        client: client.map(|Extension(id)| id),
    };
    // Detached like the repository-scoped `transact` above: the commit returns
    // now, the command providers + poll drain run in the background and broadcast
    // their writes to subscribers when they land.
    spawn_dispatch(state, origin, transients.unwrap_or_default(), lifetime).await;
    Ok(response)
}

/// Announce a durable commit on [`LOCAL_COMMIT_CHANNEL`] so
/// cross-cutting listeners (the page's analytics) hear about writes
/// to any repo without subscribing to per-endpoint channels. A no-op
/// transact leaves the head unchanged and announces nothing.
fn announce_local_commit(branch: &str, response: &TransactResponse) {
    if let Some(revision) = &response.revision_after
        && response.revision_before.as_ref() != Some(revision)
    {
        broadcast(
            LOCAL_COMMIT_CHANNEL,
            &Notification {
                branch: branch.to_owned(),
                revision: revision.clone(),
            },
        );
    }
}

/// A millisecond wall-clock stamp for sync-queue activity priority. `Date.now()`
/// in the SW event context; native (tests) has no clock dependency, so 0.
fn now_millis() -> f64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        js_sys::Date::now()
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        0.0
    }
}

/// Run [`super::dispatch`] as background work so it never blocks the transact
/// response. The service worker attaches the dispatch promise to the originating
/// fetch event; native builds run it inline so tests remain deterministic.
async fn spawn_dispatch(
    state: AppState,
    origin: super::CommandOrigin,
    transients: dialog_artifacts::Changes,
    lifetime: Option<Extension<crate::worker::FetchLifetime>>,
) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    match lifetime {
        Some(Extension(lifetime)) => {
            let promise = wasm_bindgen_futures::future_to_promise(async move {
                super::dispatch(&state, origin, transients).await;
                Ok(wasm_bindgen::JsValue::UNDEFINED)
            });
            if let Err(error) = lifetime.extend(&promise) {
                log!("transact: failed to extend fetch lifetime for command dispatch: {error:?}");
            }
        }
        None => wasm_bindgen_futures::spawn_local(async move {
            super::dispatch(&state, origin, transients).await;
        }),
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = lifetime;
        super::dispatch(&state, origin, transients).await;
    }
}

async fn transact_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: BranchReference<'a>,
    body: Bytes,
) -> Result<(Json<TransactResponse>, Option<dialog_artifacts::Changes>), TonkWorkerError> {
    let request: TransactRequest = serde_json::from_slice(&body)
        .map_err(|e| TonkWorkerError::Router(format!("invalid TransactRequest body: {e}")))?;

    // A transact carrying ONLY transients writes nothing durable — the
    // canonical case is `tonk:load`, the site stamp, which lands in the
    // session overlay (device-local, dies with the worker) and never
    // touches the branch. Failing it because the repository has not been
    // replicated yet is wrong twice over: there is no write to reject,
    // and the page that needs the stamp is exactly the page sitting on a
    // space it has just asked to join. Its claim was lost, so the site
    // was never stamped and the view sat on a spinner even after the
    // repo arrived.
    //
    // Durable claims still require the branch: a write to a repository
    // this device does not hold is a genuine error, not an absence.
    let all_transient = request
        .claims
        .iter()
        .all(|claim| claim.application().is_transient());

    let session = match tonk_branch.acquire(&tonk_state.operator).await {
        Ok(session) => session,
        Err(error) if all_transient && !request.claims.is_empty() && is_absence(&error) => {
            // Emit the transients WITHOUT a branch: they are the payload
            // the dispatcher needs (this is how `tonk:load` reaches its
            // handler), and none of them writes to storage. Returning an
            // empty `Changes` here instead would answer 200 and still
            // drop the command, which is the same silent loss with a
            // friendlier status code.
            let transients = transient_changes(request.claims)?;
            return Ok((
                Json(TransactResponse {
                    revision_before: None,
                    revision_after: None,
                    commits: CommitSummary::default(),
                }),
                (!transients.is_empty()).then_some(transients),
            ));
        }
        Err(error) => return Err(reactor_to_error(error)),
    };

    let revision_before = session.handle().revision();
    let claim_count = request.claims.len();

    // No claims: short-circuit to a no-op response so callers
    // can submit empty batches without paying for a commit.
    if request.claims.is_empty() {
        return Ok((
            Json(TransactResponse {
                revision_before: revision_before.clone(),
                revision_after: revision_before,
                commits: CommitSummary::default(),
            }),
            None,
        ));
    }

    let mut builder = tonk_branch.transaction();
    for source in request.claims {
        // Validate and type-coerce the wire claim against its
        // predicate's declared types before it can emit any facts. A
        // mismatch (e.g. a string into an `as: signed-integer` field
        // that isn't an integral float) is the caller's error.
        let claim = Claim::try_from(source)
            .map_err(|e| TonkWorkerError::Router(format!("invalid claim: {e}")))?;
        builder = builder.apply(claim);
    }

    // Capture the transient bucket before commit consumes it: the
    // commit sweeps transients from durable storage, so this snapshot
    // is the only post-commit view of which commands arrived. Empty →
    // no command dispatch.
    let transients = builder.transients.clone();
    let to_dispatch = (!transients.is_empty()).then_some(transients);

    // The per-branch transactor lock that serializes commits is taken INSIDE
    // the reactor's `commit().perform()` (so no commit path — route or direct
    // handler — can sidestep it). Taking it here too would deadlock: the
    // mutex is not re-entrant.
    let revision_after = builder
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(reactor_to_error)?;

    Ok((
        Json(TransactResponse {
            revision_before,
            revision_after: Some(revision_after),
            commits: CommitSummary {
                claims: claim_count,
                entities: Default::default(),
            },
        }),
        to_dispatch,
    ))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod profile_write_boundary {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::{Extension, body::Bytes, http::HeaderMap};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tonk_schema::claim::TransactRequest;

    use super::transact_profile;
    use crate::{
        TonkWorkerError,
        router::{AppState, ClientId, ViewBinding},
    };

    /// Build a test `AppState` containing a single view-bound client.
    /// Returns the state and the `ClientId` of that client.
    async fn test_state_with_view_bound_client() -> (AppState, ClientId) {
        let state = crate::router::tests::test_state().await;
        let app_state: AppState = Arc::new(RwLock::new(state));
        let client_id = ClientId("sealed-guest-test".to_owned());
        let bindings = app_state.read().await.view_bindings.clone();
        bindings.write().await.insert(
            client_id.clone(),
            ViewBinding {
                repo: "some-repo".to_owned(),
                branch: "main".to_owned(),
            },
        );
        (app_state, client_id)
    }

    #[dialog_common::test]
    async fn it_rejects_profile_writes_from_sealed_guest_clients() {
        // Arrange: a TonkState whose view_bindings registry contains
        // `client_id` (i.e. this client is a sealed guest bound to a
        // {repo, branch}).
        let (state, client_id) = test_state_with_view_bound_client().await;
        // Serialize an empty TransactRequest as the body — empty claims
        // trigger the short-circuit path, so without the auth guard this
        // call would succeed (proving the guard is the gating change).
        let body_bytes = Bytes::from(
            serde_json::to_vec(&TransactRequest::default()).expect("TransactRequest serializes"),
        );

        // Act
        let result = transact_profile(
            axum::extract::State(state),
            axum::extract::Path(super::ProfileTransactPath {
                branch: "main".to_owned(),
            }),
            Some(Extension(client_id)),
            None,
            HeaderMap::new(),
            body_bytes,
        )
        .await;

        // Assert
        assert!(
            matches!(result, Err(TonkWorkerError::Forbidden(_))),
            "sealed-guest clients must not write profile/meta, got: {:?}",
            result,
        );
    }
}

/// Bucket an all-transient claim list into a [`dialog_artifacts::Changes`]
/// without a branch.
///
/// Mirrors what `TransactionBuilder::apply` does for the transient case,
/// minus the branch it has no need for: transients are dispatched to
/// command handlers and swept, never written to storage. This is what
/// lets a `tonk:load` site stamp reach its handler while the repository
/// is still absent.
fn transient_changes(
    claims: Vec<tonk_schema::claim::SourceClaim>,
) -> Result<dialog_artifacts::Changes, TonkWorkerError> {
    // `Statement` carries the `assert`/`retract` methods on the plan.
    use dialog_artifacts::Statement as _;
    use tonk_schema::transact::application_plan_from_predicate;

    let mut changes = dialog_artifacts::Changes::new();
    for source in claims {
        let claim = Claim::try_from(source)
            .map_err(|e| TonkWorkerError::Router(format!("invalid claim: {e}")))?;
        match claim {
            Claim::Assert(application) => {
                application_plan_from_predicate(application).assert(&mut changes);
            }
            Claim::Retract(application) => {
                application_plan_from_predicate(application).retract(&mut changes);
            }
        }
    }
    Ok(changes)
}

/// Whether this failure means "there is nothing here" rather than
/// "something went wrong". Mirrors the query router's classifier: an
/// unreplicated repository or an uncreated branch is an absence.
fn is_absence(err: &ReactorError) -> bool {
    matches!(
        err,
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. }
    )
}

fn reactor_to_error(err: ReactorError) -> TonkWorkerError {
    match err {
        ReactorError::Shutdown => TonkWorkerError::Internal(err.to_string()),
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
            TonkWorkerError::NotFound(err.to_string())
        }
        ReactorError::QueryFailed(_)
        | ReactorError::Commit(_)
        | ReactorError::Pull(_)
        | ReactorError::Download(_)
        | ReactorError::Push(_) => TonkWorkerError::Internal(err.to_string()),
    }
}
