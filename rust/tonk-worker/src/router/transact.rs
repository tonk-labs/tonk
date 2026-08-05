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
use tonk_core::command::{
    CommandBatch, CommandOccurrence, CommandValidationError, InvocationMetadata,
};
use tonk_evaluator::effect_query::effects_by_command;
use tonk_evaluator::evaluate::CommitSummary;
use tonk_schema::claim::{Claim, SourceClaim, TransactRequest};
use tonk_schema::command_definition::CommandDefinition;
use tonk_schema::query_source::Source;

use super::AppState;
use crate::TonkWorkerError;
use crate::broadcast::{LOCAL_COMMIT_CHANNEL, Notification, broadcast};
use crate::reactor::{BranchReference, ReactorError};

/// Request-level disposition of a successfully committed invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationStatus {
    /// At least one declarative rule or native handler was registered.
    Handled,
}

/// Per-claim nominal command evidence returned by `/transact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationOutcome {
    /// Zero-based source claim index.
    pub claim: usize,
    /// Stable nominal command kind.
    pub command: dialog_artifacts::Entity,
    /// Successful request-level status.
    pub status: InvocationStatus,
    /// Declarative rules registered for this kind at preflight.
    pub registered_rules: usize,
    /// Declarative rules whose durable premises matched this occurrence.
    pub fired_rules: usize,
    /// Native handlers registered for this exact kind at preflight.
    pub registered_handlers: usize,
    /// Native handlers decoded and scheduled after commit.
    pub scheduled_handlers: usize,
    /// Opaque correlation identifier for diagnostic status lookup.
    pub correlation: String,
}

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
    /// Nominal invocation results in source claim order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invocations: Vec<InvocationOutcome>,
}

struct PreparedInvocation {
    claim: usize,
    occurrence: CommandOccurrence,
    registered_rules: usize,
    registered_handlers: usize,
}

#[derive(Default)]
struct DispatchWork {
    legacy: dialog_artifacts::Changes,
    nominal: Vec<super::PendingInvocation>,
}

/// `POST /api/repository/{repo}/branch/{branch}/transact`
#[wasm_compat]
pub async fn transact(
    State(state): State<AppState>,
    Path(path): Path<TransactPath>,
    client: Option<Extension<super::ClientId>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    log!("transact repo={}, branch={}", path.repo, path.branch);
    // A read lock on `TonkState` — concurrent transactions and syncs don't
    // serialize on the outer lock. Transactions instead serialize on the
    // per-branch transactor lock (taken inside `transact_on_branch`); sync
    // coordinates via the head CAS. So a tab's commit never blocks behind an
    // in-flight sync (the bug that made pause-mid-sync feel dead).
    let origin = super::CommandOrigin {
        repo: path.repo.clone(),
        branch: path.branch.clone(),
        client: client.map(|Extension(id)| id),
    };
    let command_env = super::CommandEnv::new(state.clone(), origin.clone());
    let (response, dispatch) = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state
            .reactor
            .repository(&path.repo)
            .branch(&path.branch);
        transact_on_branch(&tonk_state, tonk_branch, body, &command_env).await?
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
    // Run the transient command providers (route stamping, invite, join, …) and
    // the post-commit poll drain WITHOUT blocking this response. A command's
    // `execute` can be slow — `tonk:load`'s route match + site stamp is ~1s on a
    // cold content branch — and the client (`<tonk-site>`) doesn't read the
    // command's result from this response: it observes the stamp through its live
    // subscription, which the command's scheduled poll broadcasts when it lands.
    // Awaiting the dispatch here serialized the iframe boot behind the command;
    // detaching it returns the commit (~40ms) immediately and lets the command
    // finish in the background, like an `event.waitUntil`.
    spawn_dispatch(state, origin, dispatch).await;
    Ok(response)
}

/// `POST /api/profile/branch/{branch}/transact`
#[wasm_compat]
pub async fn transact_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileTransactPath>,
    client: Option<Extension<super::ClientId>>,
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
    let origin = super::CommandOrigin {
        repo: String::new(),
        branch: path.branch.clone(),
        client: client.map(|Extension(id)| id),
    };
    let command_env = super::CommandEnv::new(state.clone(), origin.clone());
    let (response, dispatch) = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
        transact_on_branch(&tonk_state, tonk_branch, body, &command_env).await?
    };
    announce_local_commit(&path.branch, &response.0);
    // Profile-branch commits carry an empty `repo` origin: the profile
    // repository is not in the named-repo namespace, and no command
    // dispatched here loads an origin repository by name. The originating
    // client is carried so the join command can post its navigate message
    // back to the exact tab that asked. `dispatch` always drains the
    // scheduled polls, even with no transients, so the durable commit fans
    // out.
    // Detached like the repository-scoped `transact` above: the commit returns
    // now, the command providers + poll drain run in the background and broadcast
    // their writes to subscribers when they land.
    spawn_dispatch(state, origin, dispatch).await;
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

/// Run [`super::dispatch`] as detached background work so it never blocks the
/// transact response. On wasm the SW keeps the spawned task alive on its event
/// loop and the returned future is immediately ready; native builds (tests) run
/// the dispatch inline so commands complete deterministically before assertions.
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

async fn spawn_dispatch(state: AppState, origin: super::CommandOrigin, dispatch: DispatchWork) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_futures::spawn_local(async move {
        super::dispatch_with_nominal(&state, origin, dispatch.legacy, dispatch.nominal).await;
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    super::dispatch_with_nominal(&state, origin, dispatch.legacy, dispatch.nominal).await;
}

async fn transact_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: BranchReference<'a>,
    body: Bytes,
    command_env: &super::CommandEnv,
) -> Result<(Json<TransactResponse>, DispatchWork), TonkWorkerError> {
    let request: TransactRequest = serde_json::from_slice(&body)
        .map_err(|e| TonkWorkerError::Router(format!("invalid TransactRequest body: {e}")))?;

    let session = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(reactor_to_error)?;

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
                invocations: Vec::new(),
            }),
            DispatchWork::default(),
        ));
    }

    // Preflight every source claim before the transaction builder emits any
    // facts. Nominal commands resolve against this branch's authoritative
    // current schema and must have at least one exact-kind consumer.
    let mut structural = Vec::new();
    let mut prepared = Vec::new();
    for (claim_index, source) in request.claims.into_iter().enumerate() {
        match source {
            SourceClaim::Invoke(source) => {
                let command = CommandDefinition::by_entity(source.command.clone())
                    .resolve(&Source::from(session.handle()), &tonk_state.operator)
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!("command resolution failed: {error}"))
                    })?
                    .ok_or_else(|| command_error("command_unknown", "unknown command kind"))?;
                let invocation = command
                    .schema()
                    .validate(source)
                    .map_err(validation_error)?;
                let correlation = format!("invoke:{}", hex::encode(rand::random::<[u8; 16]>()));
                let occurrence_entity = dialog_artifacts::Entity::new().map_err(|error| {
                    TonkWorkerError::Internal(format!(
                        "could not assign command occurrence: {error}"
                    ))
                })?;
                let occurrence = CommandOccurrence::new(
                    invocation,
                    InvocationMetadata::new(occurrence_entity, correlation),
                );
                let registered_rules = effects_by_command(occurrence.command().clone())
                    .resolve(session.handle(), &tonk_state.operator)
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!(
                            "command consumer lookup failed: {error}"
                        ))
                    })?
                    .len();
                let registered_handlers = tonk_state.commands.registrations(occurrence.command());
                if registered_rules == 0 && registered_handlers == 0 {
                    return Err(command_error(
                        "command_unhandled",
                        "command has no registered rule or native handler",
                    ));
                }
                prepared.push(PreparedInvocation {
                    claim: claim_index,
                    occurrence,
                    registered_rules,
                    registered_handlers,
                });
            }
            structural_source => {
                // Validate and type-coerce structural claims before any commit.
                structural.push(
                    Claim::try_from(structural_source).map_err(|error| {
                        TonkWorkerError::Router(format!("invalid claim: {error}"))
                    })?,
                );
            }
        }
    }

    let commands = CommandBatch::new(
        prepared
            .iter()
            .map(|invocation| invocation.occurrence.clone())
            .collect(),
    );
    let mut builder = tonk_branch.transaction().command_batch(commands);
    for claim in structural {
        builder = builder.apply(claim);
    }

    // Capture the transient bucket before commit consumes it: the
    // commit sweeps transients from durable storage, so this snapshot
    // is the only post-commit view of which commands arrived. Empty →
    // no command dispatch.
    let transients = builder.transients.clone();
    let legacy = transients;

    // The per-branch transactor lock that serializes commits is taken INSIDE
    // the reactor's `commit().perform()` (so no commit path — route or direct
    // handler — can sidestep it). Taking it here too would deadlock: the
    // mutex is not re-entrant.
    let report = builder
        .commit()
        .perform_report(&tonk_state.operator)
        .await
        .map_err(reactor_to_error)?;

    // Native work is decoded and scheduled only after the declarative commit
    // succeeds. Create the diagnostic record before any future is polled.
    let mut outcomes = Vec::with_capacity(prepared.len());
    let mut nominal = Vec::with_capacity(prepared.len());
    for invocation in prepared {
        let correlation = invocation.occurrence.correlation().to_string();
        let command = invocation.occurrence.command().clone();
        let handlers = tonk_state
            .commands
            .schedule(&invocation.occurrence, command_env);
        let scheduled_handlers = handlers.len();
        let fired_rules = report
            .induction
            .fired_rules_by_occurrence
            .get(invocation.occurrence.occurrence())
            .copied()
            .unwrap_or_default();
        tonk_state
            .invocations
            .insert(super::InvocationRecord {
                correlation: correlation.clone(),
                command: command.clone(),
                handlers: handlers
                    .iter()
                    .map(|handler| super::HandlerOutcome {
                        handler: handler.name.to_string(),
                        state: super::HandlerState::Scheduled,
                        message: None,
                    })
                    .collect(),
            })
            .await;
        outcomes.push(InvocationOutcome {
            claim: invocation.claim,
            command: command.clone(),
            status: InvocationStatus::Handled,
            registered_rules: invocation.registered_rules,
            fired_rules,
            registered_handlers: invocation.registered_handlers,
            scheduled_handlers,
            correlation: correlation.clone(),
        });
        nominal.push(super::PendingInvocation {
            correlation,
            command,
            handlers,
        });
    }

    Ok((
        Json(TransactResponse {
            revision_before,
            revision_after: Some(report.revision),
            commits: CommitSummary {
                claims: claim_count,
                entities: Default::default(),
            },
            invocations: outcomes,
        }),
        DispatchWork { legacy, nominal },
    ))
}

fn command_error(code: &'static str, message: impl Into<String>) -> TonkWorkerError {
    TonkWorkerError::Command {
        code,
        message: message.into(),
    }
}

fn validation_error(error: CommandValidationError) -> TonkWorkerError {
    match error {
        CommandValidationError::UnknownArgument { field } => command_error(
            "command_argument_unknown",
            format!("command argument {field:?} is not declared"),
        ),
        CommandValidationError::MissingRequiredArgument { field } => command_error(
            "command_argument_missing",
            format!("required command argument {field:?} is missing"),
        ),
        CommandValidationError::ReservedArgument { field } => command_error(
            "command_argument_reserved",
            format!("command argument {field:?} is reserved"),
        ),
        CommandValidationError::TypeMismatch {
            field,
            expected,
            found,
        } => command_error(
            "command_argument_type",
            format!("command argument {field:?} expects {expected} but received {found}"),
        ),
    }
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

fn reactor_to_error(err: ReactorError) -> TonkWorkerError {
    match err {
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
            TonkWorkerError::NotFound(err.to_string())
        }
        ReactorError::QueryFailed(_)
        | ReactorError::Commit(_)
        | ReactorError::Induce(_)
        | ReactorError::Pull(_)
        | ReactorError::Push(_) => TonkWorkerError::Internal(err.to_string()),
    }
}
