//! Evaluate route — accepts an asserted-notation document and
//! drives the analyze → query → mutation pipeline against the
//! branch.
//!
//! The actual analyze + plan logic lives in
//! [`tonk_evaluator::evaluate`] behind the
//! [`SyntaxEvaluateExt::evaluate`] chain. This module is
//! the axum adapter: parse the body, surface parse diagnostics
//! as 400s, acquire the cached branch via the reactor, drive
//! the chain, and assemble the JSON response. Subscription
//! polling fires after a successful commit so SSE subscribers
//! see the new state.
//!
//! Post-commit behavior mirrors `/transact`: the document's
//! transient facts (commands) are dispatched to their registered
//! providers after the commit, and a commit that moved the tree
//! marks the repo dirty so the next sync drain pushes it.

use ::axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::Changes;
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_evaluator::evaluate::{EvaluateError, SyntaxEvaluateExt};
use tonk_notation::{Parsed, Syntax, parse};

use super::AppState;
use crate::TonkWorkerError;
use crate::broadcast::{Notification, broadcast};

// Re-export the response and match types so router consumers
// (router.rs, browser clients via wasm-bindgen) name them
// through this module rather than reaching into tonk-schema.
pub use tonk_evaluator::evaluate::{CommitSummary, QueryMatchBlock, QueryResult};

/// Wire-shape returned by `/evaluate`. Local to the worker so
/// the JSON contract is owned at the HTTP boundary, not in the
/// shared evaluator. Tonk owns its own copy of this shape
/// (`tonk_cli::output`) so its `-f json` output stays byte-compatible
/// with the HTTP body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluateResponse {
    /// Revision of the branch before the commit, if any.
    pub revision_before: Option<Revision>,
    /// Revision of the branch after the commit. Equal to
    /// `revision_before` when the document didn't commit.
    pub revision_after: Option<Revision>,
    /// Per-source-expression query matches as they looked
    /// *before* the commit.
    pub matches_before: Vec<QueryMatchBlock>,
    /// Per-source-expression query matches as they look *after*
    /// the commit. For pure-query / dry-run docs this equals
    /// `matches_before`.
    pub matches_after: Vec<QueryMatchBlock>,
    /// Commit summary — number of EAV claims plus entities the
    /// document touched.
    pub commits: CommitSummary,
}

/// Path parameters for the evaluate route.
#[derive(Debug, Deserialize)]
pub struct EvaluatePath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Path parameters for the profile-side evaluate route. The
/// profile is a singleton — no `repo` segment.
#[derive(Debug, Deserialize)]
pub struct ProfileEvaluatePath {
    /// The branch name.
    pub branch: String,
}

/// Query-string parameters for the evaluate route. Today the only
/// option is `transact`, which lets the caller suppress the
/// commit step so auto-fire evaluates (e.g. when the editor
/// settles after an edit) can project what *would* happen
/// without applying mutations the user hasn't confirmed.
#[derive(Debug, Deserialize)]
pub struct EvaluateQuery {
    /// When `false`, run analysis + queries + planning but
    /// drop the dialog transaction instead of committing.
    /// `commits.claims` will be `0` and `revision_after ==
    /// revision_before`. The editor's auto-evaluate uses this
    /// so an in-progress edit can project results without
    /// applying mutations the user hasn't confirmed.
    ///
    /// Defaults to `true` so existing callers keep today's
    /// behavior. Accepts `true`/`false`, `1`/`0`, `yes`/`no`.
    #[serde(default = "default_true", deserialize_with = "deserialize_bool")]
    pub transact: bool,
}

impl Default for EvaluateQuery {
    fn default() -> Self {
        Self { transact: true }
    }
}

fn default_true() -> bool {
    true
}

/// Deserialize a query-string boolean from the loose forms a
/// browser query string might carry — `true`, `false`, `1`,
/// `0`, `yes`, `no`. Anything else falls back to `true` so a
/// stray value can't accidentally suppress the commit.
fn deserialize_bool<'de, D: serde::Deserializer<'de>>(de: D) -> Result<bool, D::Error> {
    let raw = String::deserialize(de)?;
    Ok(!matches!(
        raw.to_ascii_lowercase().as_str(),
        "false" | "0" | "no"
    ))
}

/// `POST /api/repository/{repo}/branch/{branch}/evaluate`
///
/// Body: an asserted-notation document — any mix of queries and
/// mutations. Returns query matches and a commit summary in one
/// response.
#[wasm_compat]
pub async fn evaluate(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    axum::extract::Query(query): axum::extract::Query<EvaluateQuery>,
    client: Option<Extension<super::ClientId>>,
    lifetime: Option<Extension<crate::worker::FetchLifetime>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate repo={}, branch={}", path.repo, path.branch);
    let (response, transients) = {
        // A READ lock, not a write lock. `tokio`'s `RwLock` is write-preferring, so
        // a single write-lock holder stalls every new reader — a boot-time evaluate
        // (or the editor's auto-evaluate) blocked every concurrent `query` for its
        // whole duration. `evaluate_on_branch` reaches the branch through the reactor
        // (its own per-branch locks) and never mutates `TonkState`, so shared access
        // suffices — same as `query`/`transact`/`sync`. The committing path serializes
        // on the branch transactor itself (see `evaluate_on_branch`).
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state
            .reactor
            .repository(&path.repo)
            .branch(&path.branch);
        let (response, transients) =
            evaluate_on_branch(&tonk_state, tonk_branch, body, query).await?;

        // A commit moves the branch head. Announce it on the branch's
        // channel so subscribed UIs refresh their revision/sync-state
        // badges without a full refetch (which would tear down the
        // editor). Pure queries and dry runs leave `revision_after ==
        // revision_before`, so they announce nothing.
        if let Some(revision) = &response.0.revision_after
            && response.0.revision_before.as_ref() != Some(revision)
        {
            // The commit scheduled a subscription poll; drain it so
            // subscribers see the change as an incremental delta. Without
            // this, a committing `/evaluate` leaves the delta uncomputed —
            // the branch broadcast below still fires, so UIs reconnect and
            // re-render a STALE snapshot instead of applying the new value
            // (mirrors the drain `/transact` does after its commit).
            tonk_state
                .reactor
                .run_scheduled_polls(&tonk_state.operator)
                .await;

            broadcast(
                &format!("/api/repository/{}/branch/{}", path.repo, path.branch),
                &Notification {
                    branch: path.branch.clone(),
                    revision: revision.clone(),
                },
            );
            broadcast(
                crate::broadcast::LOCAL_COMMIT_CHANNEL,
                &Notification {
                    branch: path.branch.clone(),
                    revision: revision.clone(),
                },
            );
        }
        (response, transients)
    };

    // Mark the repo dirty so the next sync drain pushes the new commits —
    // only when the commit actually moved the tree, mirroring `/transact`.
    if response.0.revision_before.as_ref().map(|r| &r.tree)
        != response.0.revision_after.as_ref().map(|r| &r.tree)
    {
        let tonk_state = state.read().await;
        tonk_state
            .sync_queue
            .mark_dirty(&path.repo, super::sync::now_millis());
    }

    // Dispatch the document's transient commands, mirroring `/transact`:
    // detached so a slow handler never delays the response, with the
    // origin carrying the branch this commit landed in and the client
    // that asked.
    if let Some(transients) = transients {
        let origin = super::CommandOrigin {
            repo: path.repo,
            branch: path.branch,
            client: client.map(|Extension(id)| id),
        };
        super::transact::spawn_dispatch(state, origin, transients, lifetime).await;
    }
    Ok(response)
}

/// `POST /api/profile/branch/{branch}/evaluate`
///
/// Profile-side counterpart to [`evaluate`]. The profile is its
/// own repository but lives outside the named-repo namespace, so
/// the route surface is parallel to the repository routes rather
/// than nested under one. Same body / query-string / response
/// contract.
#[wasm_compat]
pub async fn evaluate_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileEvaluatePath>,
    axum::extract::Query(query): axum::extract::Query<EvaluateQuery>,
    client: Option<Extension<super::ClientId>>,
    lifetime: Option<Extension<crate::worker::FetchLifetime>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate profile branch={}", path.branch);
    // Same write boundary as `transact_profile`: a sealed guest is bound
    // to its view's {repo, branch} and must not write the profile. A
    // dry run (`transact=false`) commits nothing, so only the committing
    // form is refused.
    if query.transact
        && let Some(Extension(client_id)) = &client
    {
        let bindings = state.read().await.view_bindings.clone();
        if bindings.read().await.contains_key(client_id) {
            return Err(TonkWorkerError::Forbidden(
                "sealed guests may not write the profile branch".into(),
            ));
        }
    }
    let (response, transients) = {
        // Read lock — see [`evaluate`] for why a write lock here serialized every
        // concurrent request behind a commit.
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
        let (response, transients) =
            evaluate_on_branch(&tonk_state, tonk_branch, body, query).await?;

        // Same durable-commit gate as [`evaluate`]: pure queries and dry
        // runs leave the head unchanged and announce nothing. The profile
        // routes have no per-endpoint announcement to mirror, so only the
        // cross-cutting local-commit channel is posted.
        if let Some(revision) = &response.0.revision_after
            && response.0.revision_before.as_ref() != Some(revision)
        {
            // Drain the poll the commit scheduled so subscribers get the
            // incremental delta (see the note in [`evaluate`]).
            tonk_state
                .reactor
                .run_scheduled_polls(&tonk_state.operator)
                .await;

            broadcast(
                crate::broadcast::LOCAL_COMMIT_CHANNEL,
                &Notification {
                    branch: path.branch.clone(),
                    revision: revision.clone(),
                },
            );
        }
        (response, transients)
    };

    // Dispatch transient commands with the empty-repo origin profile
    // commits carry — mirroring `transact_profile`.
    if let Some(transients) = transients {
        let origin = super::CommandOrigin {
            repo: String::new(),
            branch: path.branch,
            client: client.map(|Extension(id)| id),
        };
        super::transact::spawn_dispatch(state, origin, transients, lifetime).await;
    }
    Ok(response)
}

/// Builds the facts recording a seed install, given the version its
/// library commit minted.
///
/// Called once the library has STAGED, so the version it receives is
/// authoritative rather than predicted; the facts it returns commit as
/// the next link of the same batch.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
pub type SeedRecord<'a> = &'a (
        dyn Fn(&dialog_artifacts::history::Version) -> Vec<dialog_artifacts::Instruction>
            + Send
            + Sync
    );

/// Commit the evaluated transaction, optionally chaining a second commit
/// that names the first's version, then publish the whole chain.
///
/// The two-commit shape is what lets a fact name its own commit. A
/// branch transaction's commit STAGES: the revision is minted, so
/// `batch.version()` is authoritative rather than predicted, but the
/// branch head has not moved and nothing is visible yet. The record
/// commits as the next link, and the single `publish` moves the head to
/// the chain tip — so either both land or neither does, and no reader
/// ever observes a library without the record describing it.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
async fn stage_and_publish(
    tonk_state: &crate::worker::TonkState,
    txn: dialog_repository::Transaction<&dialog_repository::Branch>,
    record: Option<SeedRecord<'_>>,
) -> Result<dialog_artifacts::Revision, dialog_repository::CommitError> {
    let batch = txn.commit().perform(&tonk_state.operator).await?;
    let Some(record) = record else {
        return batch.publish().perform(&tonk_state.operator).await;
    };

    // Authoritative, not predicted: the commit that minted this version
    // has already happened. It just is not visible yet.
    let instructions = record(&batch.version());
    let mut next = batch.transaction();
    for instruction in instructions {
        next = match instruction {
            dialog_artifacts::Instruction::Assert(artifact)
            | dialog_artifacts::Instruction::Replace(artifact) => {
                next.assert(crate::router::claim::RawClaim {
                    the: artifact.the,
                    of: artifact.of,
                    is: artifact.is,
                    unique: false,
                })
            }
            dialog_artifacts::Instruction::Retract(artifact) => {
                next.retract(crate::router::claim::RawClaim {
                    the: artifact.the,
                    of: artifact.of,
                    is: artifact.is,
                    unique: false,
                })
            }
        };
    }
    next.commit()
        .perform(&tonk_state.operator)
        .await?
        .publish()
        .perform(&tonk_state.operator)
        .await
}

/// Shared body for [`evaluate`] and [`evaluate_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is
/// the only difference between the two routes.
///
/// Alongside the response, returns the transient facts (commands) the
/// committed document dispatched — `None` for dry runs, pure queries,
/// and documents that carried no transients. The caller hands them to
/// the post-commit command dispatcher, exactly as `/transact` does.
async fn evaluate_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: crate::reactor::BranchReference<'a>,
    body: Bytes,
    query: EvaluateQuery,
) -> Result<(Json<EvaluateResponse>, Option<Changes>), TonkWorkerError> {
    evaluate_on_branch_with(
        tonk_state,
        tonk_branch,
        body,
        query,
        Vec::new(),
        None,
        EvaluationMode::Interactive,
    )
    .await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EvaluationMode {
    Interactive,
    LibrarySeed,
    #[cfg(test)]
    LibrarySeedWithRace,
}

/// Libraries are known mutation documents. Take the writer lock before their
/// first evaluation, sharing the interactive path's commit, refresh and retry.
pub(super) async fn seed_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: crate::reactor::BranchReference<'a>,
    body: String,
) -> Result<EvaluateResponse, TonkWorkerError> {
    evaluate_on_branch_with(
        tonk_state,
        tonk_branch,
        Bytes::from(body.into_bytes()),
        EvaluateQuery { transact: true },
        Vec::new(),
        None,
        EvaluationMode::LibrarySeed,
    )
    .await
    .map(|(Json(response), _)| response)
}

/// [`evaluate_on_branch`], with `retract` folded into the same batch the
/// document commits in, and an optional second commit that can name the
/// first's version.
///
/// A seed upgrade is the caller: it withdraws the previous seed's claims
/// and installs the new library atomically. Order matters and is fixed
/// here — the retractions seed the transaction, the document follows —
/// because a retract followed by an assert of the same fact KEEPS it,
/// citing what it overrode, while the reverse order cancels. So the
/// overlap between two seeds survives an upgrade untouched.
///
/// `record` is how a seed record names the very commit that installed
/// the library. The document's commit STAGES rather than publishes, so
/// its version is already minted and authoritative when `record` is
/// handed it; the facts it returns are committed as the next link of the
/// same batch, and one publish makes both visible at once. Nothing
/// predicts a version, and no reader ever sees a library without its
/// record.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
async fn evaluate_on_branch_with<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: crate::reactor::BranchReference<'a>,
    body: Bytes,
    query: EvaluateQuery,
    retract: Vec<crate::router::claim::RawClaim>,
    record: Option<SeedRecord<'_>>,
    mode: EvaluationMode,
) -> Result<(Json<EvaluateResponse>, Option<Changes>), TonkWorkerError> {
    let total_start = web_time::Instant::now();
    let evaluation_passes = std::sync::atomic::AtomicUsize::new(0);
    let text = std::str::from_utf8(&body)
        .map_err(|e| TonkWorkerError::Router(format!("body is not valid UTF-8: {e}")))?;

    let t_parse = web_time::Instant::now();
    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;
    let parse_ms = t_parse.elapsed().as_millis();

    let exprs = syntax.expressions.len();
    log!("Evaluating {exprs} expression(s)");

    let session = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    // A document commits when it writes anything. `rule!:` is a mutation (the
    // `!` says so) and the analyzer lifts it into a `Statement::InstallEffect`,
    // so a planned statement is the single commit signal. We can only know this
    // after evaluating, but the WILL-commit decision also governs locking, so a
    // committing document runs its whole evaluate+commit under the branch
    // transactor while a dry run takes no lock — hence the two arms below share
    // the evaluation via a closure rather than a pre-check. Library seeds are
    // known writers and bypass the speculative evaluation.

    // Evaluate against the current head. Kept as a closure so the committing
    // path can replay it after a refresh (the evaluated transaction borrows the
    // pre-refresh tree, so a moved head means re-evaluating, not just
    // re-committing). Evaluation is pure over (document, head) and the
    // document's statements are idempotent asserts/retracts, so replay is safe.
    let evaluate_once = || async {
        evaluation_passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let branch = session.handle();
        let revision_before = branch.revision();
        // The branch folds its session overlay into every read — the
        // transaction's as-if-committed view included — so the dry-run preview
        // sees the same ephemeral facts a `query`/`subscribe` does with no
        // explicit integrate here, and they never reach the durable write (the
        // overlay is session-only). Match queries resolve stored `db.rule/*`
        // rules automatically via the branch query's layer stack.
        let mut txn = branch.transaction();
        for claim in retract.iter().cloned() {
            txn = txn.retract(claim);
        }
        let t_eval = web_time::Instant::now();
        let evaluated = syntax
            .evaluate(txn)
            .perform(&tonk_state.operator)
            .await
            .map_err(map_evaluate_error)?;
        let eval_ms = t_eval.elapsed().as_millis();
        // Post-evaluation matches: the txn's overlay already reflects every
        // mutation and induce-pass derivation, so this is the same answer a
        // post-commit branch query would give.
        let t_matches = web_time::Instant::now();
        let matches_after = evaluated
            .matches_after(&tonk_state.operator)
            .await
            .map_err(map_evaluate_error)?;
        let matches_ms = t_matches.elapsed().as_millis();
        Ok::<_, TonkWorkerError>((
            evaluated,
            revision_before,
            matches_after,
            eval_ms,
            matches_ms,
        ))
    };

    // Dry-run fast path: evaluate once, take NO lock, drop the transaction.
    // This is the editor's per-keystroke auto-evaluate — it must never contend
    // with a committing writer, which is the whole point of dropping the write
    // lock. We can't know it's a dry run until after evaluating, so a peek: if
    // the first (lock-free) evaluation shows no commit, return it directly; only
    // a committing document re-enters under the transactor lock.
    if mode == EvaluationMode::Interactive {
        let (evaluated, revision_before, matches_after, ..) = evaluate_once().await?;
        if !(query.transact && evaluated.analysis.analysis.has_statements()) {
            // Pure-query or dry-run: drop the transaction without committing. The
            // pre-mutation matches double as "after". Zero out `claims` so the
            // response reflects what *did* commit (nothing) — the editor's
            // auto-evaluate relies on this to know the branch is untouched.
            let _ = matches_after;
            let mut commits = evaluated.commits;
            commits.claims = 0;
            return Ok((
                Json(EvaluateResponse {
                    revision_before: revision_before.clone(),
                    revision_after: revision_before,
                    matches_before: evaluated.matches.clone(),
                    matches_after: evaluated.matches,
                    commits,
                }),
                None,
            ));
        }
        drop(evaluated);
    }

    // Committing path. Serialize on the branch transactor — the same lock the
    // reactor's `Commit::perform` takes for `/transact`. This document's commit
    // is a *dialog* `Transaction::commit()` (it threads the raw transaction
    // through the evaluator), which CASes against the head snapshot but never
    // retries. The lock excludes the common racer — another committer — so those
    // line up here instead of one losing the CAS. A sync is the exception the
    // lock can't cover: it advances the head while holding this lock only for
    // its microsecond cell write, releasing it across its network fetch, so it
    // can still land in our snapshot→publish window. The retry loop below
    // handles that residual case by refreshing and re-evaluating, exactly as
    // `Commit::perform` does for `/transact`.
    let _committing = session.transactor().lock().await;

    // Evaluate under the lock and commit, refreshing and re-evaluating on a
    // `Version mismatch` (a sync landed between our head snapshot and the
    // publish). Each iteration re-evaluates because `Transaction` isn't `Clone`
    // and the commit consumes it — and after a refresh the evaluation must run
    // against the new head anyway. Bounded so a flapping head can't spin.
    const EVALUATE_RETRY_LIMIT: usize = 4;
    let (
        revision_before,
        revision_after,
        matches_after,
        matches_before,
        commits,
        transients,
        eval_ms,
        matches_ms,
        commit_ms,
    ) = {
        let mut attempt = 0;
        loop {
            let (evaluated, revision_before, matches_after, eval_ms, matches_ms) =
                evaluate_once().await?;
            if mode != EvaluationMode::Interactive && !evaluated.analysis.analysis.has_statements()
            {
                return Err(TonkWorkerError::Internal(
                    "library seed must contain mutation statements".to_owned(),
                ));
            }
            // Model an external head advance after the snapshot, bypassing the
            // transactor just as an in-flight sync can. Only compiled in tests.
            #[cfg(test)]
            if mode == EvaluationMode::LibrarySeedWithRace && attempt == 0 {
                syntax
                    .evaluate(session.handle().transaction())
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(map_evaluate_error)?
                    .txn
                    .commit()
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(|e| TonkWorkerError::Internal(format!("test race: {e}")))?
                    .publish()
                    .perform(&tonk_state.operator)
                    .await
                    .map_err(|e| TonkWorkerError::Internal(format!("test race: {e}")))?;
            }
            // Extract what the response needs before the commit consumes the
            // transaction (`Transaction` isn't `Clone`, and `commit()` takes it
            // by value). The transients mirror is what post-commit command
            // dispatch runs on — the commit sweeps them from the transaction.
            let matches_before = evaluated.matches;
            let commits = evaluated.commits;
            let transients = evaluated.transients;
            let t_commit = web_time::Instant::now();
            match stage_and_publish(tonk_state, evaluated.txn, record).await {
                Ok(revision_after) => {
                    break (
                        revision_before,
                        revision_after,
                        matches_after,
                        matches_before,
                        commits,
                        transients,
                        eval_ms,
                        matches_ms,
                        t_commit.elapsed().as_millis(),
                    );
                }
                Err(e)
                    if e.to_string().contains("Version mismatch")
                        && attempt + 1 < EVALUATE_RETRY_LIMIT =>
                {
                    attempt += 1;
                    log!(
                        "evaluate commit raced a sync (attempt {attempt}); refreshing and retrying"
                    );
                    session
                        .handle()
                        .refresh(&tonk_state.operator)
                        .await
                        .map_err(|e| {
                            map_evaluate_error(EvaluateError::Query(format!("refresh: {e}")))
                        })?;
                }
                Err(e) => {
                    return Err(map_evaluate_error(EvaluateError::Query(format!(
                        "commit: {e}"
                    ))));
                }
            }
        }
    };

    // Re-poll subscriptions so SSE clients see the new state. The chain commits
    // via dialog directly; the reactor's subscription registry is the worker's
    // responsibility.
    let t_poll = web_time::Instant::now();
    session.poll(&tonk_state.operator).await;
    let poll_ms = t_poll.elapsed().as_millis();
    let total_ms = total_start.elapsed().as_millis();
    let passes = evaluation_passes.load(std::sync::atomic::Ordering::Relaxed);
    let timing = format!(
        "evaluate timing: {exprs} exprs | parse {parse_ms}ms | analyze+eval {eval_ms}ms | matches {matches_ms}ms | commit {commit_ms}ms | poll {poll_ms}ms | total {total_ms}ms | passes {passes}"
    );
    log!("{timing}");
    // Tee timing onto a BroadcastChannel so a page (or DevTools listener) can
    // read seed numbers without the SW console — the seed runs background in the
    // SW, so its logs never reach the page console.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if let Ok(channel) = web_sys::BroadcastChannel::new("tonk-timing") {
        let _ = channel.post_message(&wasm_bindgen::JsValue::from_str(&timing));
    }

    Ok((
        Json(EvaluateResponse {
            revision_before,
            revision_after: Some(revision_after),
            matches_before,
            matches_after,
            commits,
        }),
        (!transients.is_empty()).then_some(transients),
    ))
}

/// Bridge-callable wrapper around the evaluate pipeline. Runs
/// the same logic as [`evaluate_on_branch`] but accepts plain
/// `String` arguments instead of HTTP-level types so the bridge
/// handler can call it without constructing an axum request.
/// Gated to match its callers: every seeding path that needs its record
/// to name the installing commit now goes through
/// [`evaluate_body_recording`], leaving this reachable only from tests
/// and the service worker.
#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
pub async fn evaluate_body(
    tonk_state: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    body: String,
    transact: bool,
) -> Result<EvaluateResponse, TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.repository(repo).branch(branch);
    let query = EvaluateQuery { transact };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch(tonk_state, tonk_branch, bytes, query)
        .await
        .map(|(Json(r), _)| r)
}

/// [`evaluate_body`], additionally returning the transient facts
/// (commands) the committed document dispatched. The bridge's evaluate
/// handler uses this so a sealed guest's document triggers command
/// dispatch the same way the HTTP `/evaluate` route does; callers that
/// evaluate authored documents with no commands (seeding, joins) keep
/// using [`evaluate_body`].
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
pub async fn evaluate_body_with_transients(
    tonk_state: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    body: String,
    transact: bool,
) -> Result<(EvaluateResponse, Option<Changes>), TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.repository(repo).branch(branch);
    let query = EvaluateQuery { transact };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch(tonk_state, tonk_branch, bytes, query)
        .await
        .map(|(Json(r), transients)| (r, transients))
}

/// [`evaluate_body`], with a second commit that names the first's
/// version.
///
/// The seed install's entry point. The document stages, its minted
/// version is handed to `record`, and the facts that come back commit as
/// the next link of the same batch — one publish for both. Nothing
/// predicts a version, and no reader ever sees a library without the
/// record describing it.
pub async fn evaluate_body_recording(
    tonk_state: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    body: String,
    record: SeedRecord<'_>,
) -> Result<EvaluateResponse, TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.repository(repo).branch(branch);
    let query = EvaluateQuery { transact: true };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch_with(
        tonk_state,
        tonk_branch,
        bytes,
        query,
        Vec::new(),
        Some(record),
        EvaluationMode::LibrarySeed,
    )
    .await
    .map(|(Json(r), _)| r)
}

/// [`evaluate_body`], with `retract` folded into the same commit and a
/// `record` naming that commit's version.
///
/// The seed upgrade's entry point: withdrawing the previous seed and
/// installing its replacement is one staged commit, so a subscriber never
/// sees a space with no definitions, and the record naming it chains on
/// before the single publish.
pub async fn evaluate_with_retractions(
    tonk_state: &crate::worker::TonkState,
    repo: &str,
    branch: &str,
    body: String,
    retract: Vec<crate::router::claim::RawClaim>,
    record: SeedRecord<'_>,
) -> Result<EvaluateResponse, TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.repository(repo).branch(branch);
    let query = EvaluateQuery { transact: true };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch_with(
        tonk_state,
        tonk_branch,
        bytes,
        query,
        retract,
        Some(record),
        EvaluationMode::LibrarySeed,
    )
    .await
    .map(|(Json(r), _)| r)
}

/// [`evaluate_profile_body`], with a second commit naming the first's
/// version — the profile branch's counterpart to
/// [`evaluate_body_recording`].
pub async fn evaluate_profile_body_recording(
    tonk_state: &crate::worker::TonkState,
    branch: &str,
    body: String,
    record: SeedRecord<'_>,
) -> Result<EvaluateResponse, TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.profile_repository().branch(branch);
    let query = EvaluateQuery { transact: true };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch_with(
        tonk_state,
        tonk_branch,
        bytes,
        query,
        Vec::new(),
        Some(record),
        EvaluationMode::LibrarySeed,
    )
    .await
    .map(|(Json(r), _)| r)
}

/// Like [`evaluate_body`], but against the **profile** repository's
/// branch rather than a named repo. Used to seed the standard library
/// onto the profile meta branch at profile creation, so a
/// `<tonk-display>` reading the profile (e.g. the Hub) can resolve the
/// library's concepts and views there. SW-only — its sole caller
/// (`seed_profile_library`) is gated to the service-worker scope.
/// Gated to match its callers: the profile-seeding path now goes through
/// [`evaluate_profile_body_recording`] so the seed record can name the
/// commit that installed it, leaving this reachable only from the
/// service-worker tests.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn evaluate_profile_body(
    tonk_state: &crate::worker::TonkState,
    branch: &str,
    body: String,
    transact: bool,
) -> Result<EvaluateResponse, TonkWorkerError> {
    let tonk_branch = tonk_state.reactor.profile_repository().branch(branch);
    let query = EvaluateQuery { transact };
    let bytes = Bytes::from(body.into_bytes());
    evaluate_on_branch(tonk_state, tonk_branch, bytes, query)
        .await
        .map(|(Json(r), _)| r)
}

/// Project [`Parsed`] onto a successful syntax or a 400 error
/// carrying the first diagnostic's structure (code + range +
/// message) so the editor can route it to a positioned
/// squiggle. Subsequent diagnostics are dropped: the parser
/// can produce a cascade from a single root cause and surfacing
/// them all confuses more than helps. The first one is
/// generally the proximate cause.
fn surface_parse_diagnostics(parsed: Parsed) -> Result<Syntax, TonkWorkerError> {
    if let Some(first) = parsed.diagnostics.first() {
        let code = first
            .code
            .as_ref()
            .and_then(|c| match c {
                lsp_types::NumberOrString::String(s) => Some(s.clone()),
                lsp_types::NumberOrString::Number(_) => None,
            })
            // Stable fallback so the client always has something
            // to switch on. Parser diagnostics from
            // `tonk-notation` carry codes today; this default
            // keeps the contract honest if a future emitter
            // forgets to set one.
            .unwrap_or_else(|| "E_PARSE".to_owned());
        return Err(TonkWorkerError::Analyze {
            code,
            message: first.message.clone(),
            range: Some(first.range),
        });
    }
    parsed
        .syntax
        .ok_or_else(|| TonkWorkerError::Router("empty document".to_owned()))
}

/// Map shared-evaluator errors onto worker-level HTTP failures.
/// Analyze-time failures are the user's fault (400); query and
/// plan failures are internal (500).
fn map_evaluate_error(error: EvaluateError) -> TonkWorkerError {
    match error {
        EvaluateError::Analyze(analyze_error) => {
            log!("Analyzer rejected document: {analyze_error}");
            // `From<AnalyzeError>` carries `code` and `range`
            // through to the structured response body the
            // editor decodes into a `TonkUiError::Analyze`
            // diagnostic — that's what positions the squiggle.
            TonkWorkerError::from(analyze_error)
        }
        EvaluateError::Query(message) => TonkWorkerError::Internal(message),
        EvaluateError::Plan(message) => {
            TonkWorkerError::Internal(format!("plan failed: {message}"))
        }
    }
}

/// Route-level regression tests for `/evaluate`.
///
/// These drive [`evaluate_body`] — the test wrapper that runs
/// the *same* `evaluate_on_branch` logic the HTTP handler runs,
/// including the commit guard. They guard the two bug classes that
/// escaped to manual browser testing this session: a `rule!:`-only
/// document silently not committing, and a rule never firing on a
/// transient instance.
///
/// Runs natively and in the service-worker harness.
#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    #[cfg(target_arch = "wasm32")]
    use axum::body::Body;
    #[cfg(target_arch = "wasm32")]
    use axum::http::{Request, StatusCode};
    #[cfg(target_arch = "wasm32")]
    use tower::ServiceExt;

    use super::{EvaluateResponse, evaluate_body};
    use crate::router::AppState;
    #[cfg(target_arch = "wasm32")]
    use crate::router::{RepositoryInfo, api_router_with_state, tests::test_state};

    /// Create the test repository via `PUT /api/repository/{name}`,
    /// then hand back the wrapped [`AppState`] so tests can call
    /// [`evaluate_body`] against the same `TonkState` the route
    /// would. The reactor only *loads* repositories — it never
    /// creates them — so the repo must exist before the first
    /// `evaluate_body` call acquires a branch on it.
    ///
    /// `label` is only a display name; the repository is created with a
    /// freshly minted identity and mounted at its routing key. Returns
    /// the state plus that key so callers address the repo by identity.
    #[cfg(target_arch = "wasm32")]
    async fn state_with_repo(label: &str) -> (AppState, String) {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{label}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "expected 201 from PUT /api/repository/{label}, got {status}",
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let info: RepositoryInfo = serde_json::from_slice(&body).unwrap();
        (state, info.name)
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn state_with_repo(label: &str) -> (AppState, String) {
        use crate::router::repository::{
            BranchConfiguration, RepositoryConfiguration, create_repository,
        };
        use tonk_schema::prelude::DidExt;
        let state = crate::router::command::tests::native::test_state().await;
        let repo = create_repository(
            &*state.read().await,
            label,
            &RepositoryConfiguration::default().branch("main", BranchConfiguration::default()),
        )
        .await
        .unwrap()
        .did()
        .repo_key()
        .to_owned();
        (state, repo)
    }

    async fn seed(state: &AppState, repo: &str, body: &str) -> EvaluateResponse {
        let tonk = state.read().await;
        super::seed_on_branch(
            &tonk,
            tonk.reactor.repository(repo).branch("main"),
            body.to_owned(),
        )
        .await
        .unwrap()
    }

    async fn facts(state: &AppState, repo: &str) -> std::collections::BTreeSet<String> {
        use futures_util::StreamExt;
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(repo)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let stream = session
            .handle()
            .claims()
            .select(dialog_artifacts::ArtifactSelector::new().of_starting_with(""))
            .perform(&tonk.operator)
            .await
            .unwrap();
        tokio::pin!(stream);
        let mut artifacts = Vec::new();
        while let Some(artifact) = stream.next().await {
            let artifact = artifact.unwrap().to_owned().unwrap();
            // Revision records encode the repository identity and signatures.
            if artifact.the.to_string() != "dialog.db/revision" {
                artifacts.push(artifact);
            }
        }
        let mut rules = std::collections::BTreeMap::new();
        for artifact in &artifacts {
            if artifact.the.to_string() == "dialog.rule/source" {
                let dialog_artifacts::Value::Bytes(bytes) = &artifact.is else {
                    panic!("rule source must be bytes")
                };
                let mut rule = match dialog_query::rule::InductiveRule::decode(bytes) {
                    Ok(rule) => serde_json::to_value(rule).unwrap(),
                    Err(_) => serde_json::to_value(
                        dialog_query::rule::DeductiveRule::decode(bytes).unwrap(),
                    )
                    .unwrap(),
                };
                normalize_generated_variables(&mut rule, &mut Default::default());
                let source = serde_json::to_string(&rule).unwrap();
                let identity = blake3::hash(source.as_bytes()).to_string();
                rules.insert(artifact.of.to_string(), (identity, source));
            }
        }
        let mut facts = std::collections::BTreeSet::new();
        for artifact in artifacts {
            let subject = artifact.of.to_string();
            let subject = rules
                .get(&subject)
                .map(|r| r.0.as_str())
                .unwrap_or(&subject);
            let value = if artifact.the.to_string() == "dialog.rule/source" {
                rules.get(&artifact.of.to_string()).unwrap().1.clone()
            } else {
                format!("{:?}", artifact.is)
            };
            facts.insert(format!("{} {subject} {value}", artifact.the));
        }
        facts
    }

    // The analyzer's fresh variable counter is process-global. Alpha-rename
    // generated variables in first-use order, preserving repeated references,
    // user variable names, constants and all rule structure.
    fn normalize_generated_variables(
        value: &mut serde_json::Value,
        names: &mut std::collections::BTreeMap<String, String>,
    ) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(serde_json::Value::Object(variable)) = object.get_mut("?")
                    && let Some(serde_json::Value::String(name)) = variable.get_mut("name")
                    && name.strip_prefix("__").is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
                    })
                {
                    let next = format!("__{}", names.len());
                    *name = names.entry(name.clone()).or_insert(next).clone();
                }
                for child in object.values_mut() {
                    normalize_generated_variables(child, names);
                }
            }
            serde_json::Value::Array(array) => {
                for child in array {
                    normalize_generated_variables(child, names);
                }
            }
            _ => {}
        }
    }

    #[dialog_common::test]
    async fn library_seeds_match_interactive_facts_and_rules() {
        let (single, single_repo) = state_with_repo("single-pass").await;
        let (interactive, interactive_repo) = state_with_repo("interactive").await;
        // Compare each real first-boot library, including the durable db.rule/*
        // artifacts, without comparing identity-specific commit revisions.
        for library in [
            include_str!("../../../tonk-core/assets/library/core.yaml"),
            include_str!("../../../tonk-core/assets/library/profile.yaml"),
            include_str!("../../../tonk-core/assets/library/onboarding-agent.yaml"),
        ] {
            let single_before = facts(&single, &single_repo).await;
            let interactive_before = facts(&interactive, &interactive_repo).await;
            let seeded = seed(&single, &single_repo, library).await;
            let evaluated = evaluate(&interactive, &interactive_repo, library, true).await;
            assert_eq!(seeded.commits.claims, evaluated.commits.claims);
            let single_after = facts(&single, &single_repo).await;
            let interactive_after = facts(&interactive, &interactive_repo).await;
            let single_added = single_after
                .difference(&single_before)
                .collect::<std::collections::BTreeSet<_>>();
            let interactive_added = interactive_after
                .difference(&interactive_before)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(single_added.len(), interactive_added.len());
            assert!(
                single_added == interactive_added,
                "single-only: {:?}; interactive-only: {:?}",
                single_added
                    .difference(&interactive_added)
                    .collect::<Vec<_>>(),
                interactive_added
                    .difference(&single_added)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[dialog_common::test]
    async fn seeded_rules_fire_and_concurrent_seeds_preserve_both_writes() {
        let (state, repo) = state_with_repo("seed-rules").await;
        seed(&state, &repo, CONCEPTS).await;
        seed(&state, &repo, RULE).await;
        let first = "person-entered!:\n  this: did:key:zFirst\n  name: First\n  age: 1\n";
        let second = "person-entered!:\n  this: did:key:zSecond\n  name: Second\n  age: 2\n";
        futures_util::future::join(seed(&state, &repo, first), seed(&state, &repo, second)).await;
        let query = evaluate(&state, &repo, "person:\n", false).await;
        assert_eq!(query.matches_after[0].results.len(), 2);
    }

    #[dialog_common::test]
    async fn library_seed_retries_after_a_head_race() {
        let (state, repo) = state_with_repo("seed-race").await;
        let tonk = state.read().await;
        let response = super::evaluate_on_branch_with(
            &tonk,
            tonk.reactor.repository(&repo).branch("main"),
            CONCEPTS.to_owned().into(),
            super::EvaluateQuery { transact: true },
            Vec::new(),
            None,
            super::EvaluationMode::LibrarySeedWithRace,
        )
        .await
        .unwrap()
        .0
        .0;
        assert!(response.revision_after.is_some());
        drop(tonk);
        seed(&state, &repo, RULE).await;
        seed(
            &state,
            &repo,
            "person-entered!:\n  this: did:key:zRaced\n  name: Raced\n  age: 3\n",
        )
        .await;
        assert_eq!(
            evaluate(&state, &repo, "person:\n", false)
                .await
                .matches_after[0]
                .results
                .len(),
            1
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn interactive_queries_and_dry_runs_do_not_wait_for_the_writer() {
        let (state, repo) = state_with_repo("lock-free-preview").await;
        seed(&state, &repo, CONCEPTS).await;
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(&repo)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .unwrap();
        let _writer = session.transactor().lock().await;
        for (body, transact) in [("person:\n", true), (RULE, false)] {
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                evaluate_body(&tonk, &repo, "main", body.to_owned(), transact),
            )
            .await
            .expect("interactive preview must not wait for the held writer lock")
            .unwrap();
            assert_eq!(response.revision_before, response.revision_after);
            assert_eq!(response.commits.claims, 0);
        }
    }

    /// Run a document through [`evaluate_body`] against the test
    /// state's `main` branch.
    async fn evaluate(
        state: &AppState,
        repo: &str,
        body: &str,
        transact: bool,
    ) -> EvaluateResponse {
        let guard = state.read().await;
        evaluate_body(&guard, repo, "main", body.to_owned(), transact)
            .await
            .unwrap_or_else(|e| panic!("evaluate_body failed: {e}"))
    }

    /// A document declaring the transient `person-entered` concept,
    /// the durable `person` concept, and the attributes the rule's
    /// premise and head bind. Committing this first lets a separate
    /// `rule!:` document resolve both concepts by bookmark name.
    const CONCEPTS: &str = r#"concept!: &person-entered
  transient:
  with:
    name:
      the: xyz.tonk.env/name
      as: text
      cardinality: one
      description: "name"
    age:
      the: xyz.tonk.env/age
      as: unsigned-integer
      cardinality: one
      description: "age"

attribute!: &person-name
  description: The person's name
  the: xyz.tonk.person/name
  as: text
  cardinality: one

attribute!: &person-age
  description: The person's age
  the: xyz.tonk.person/age
  as: unsigned-integer
  cardinality: one

concept!: &person
  description: "A person"
  with:
    name: person-name
    age: person-age
"#;

    /// The rule person <- person-entered, as its own document.
    const RULE: &str = r#"rule!:
  assert!: person
  when:
    - assert: person-entered
      where: { this: ?this, name: ?name, age: ?age }
"#;

    /// Regression: a document that is *only* a `rule!:` is a
    /// mutation document — the `!` says so. It must commit. Before
    /// the fix the commit guard checked the wrong condition and
    /// rule-only documents were silently dropped: the rule never
    /// reached the branch.
    #[dialog_common::test]
    async fn it_commits_a_rule_only_document() {
        let (state, repo) = state_with_repo("test-evaluate-rule-only").await;
        let repo = repo.as_str();

        // First document: install the concepts the rule references.
        let concepts = evaluate(&state, repo, CONCEPTS, true).await;
        assert!(
            concepts.commits.claims > 0,
            "concepts document should commit claims",
        );

        // Second document: only the rule.
        let rule = evaluate(&state, repo, RULE, true).await;
        assert!(
            rule.commits.claims > 0,
            "rule-only document must commit; saw {} claims",
            rule.commits.claims,
        );
        assert_ne!(
            rule.revision_after, rule.revision_before,
            "rule-only document must advance the branch revision",
        );
    }

    /// Regression: a rule must fire on a transient concept instance
    /// asserted through notation. Install concepts + rule, assert a
    /// transient `person-entered`, then query the durable `person`
    /// and confirm the rule produced a row. Mirrors the real
    /// `person-entered → person` browser scenario.
    #[dialog_common::test]
    async fn it_fires_a_rule_on_a_transient_instance() {
        let (state, repo) = state_with_repo("test-evaluate-rule-fires").await;
        let repo = repo.as_str();

        evaluate(&state, repo, CONCEPTS, true).await;
        evaluate(&state, repo, RULE, true).await;

        // Assert a transient `person-entered` instance. The write
        // seeds the effects fixpoint that drives the rule.
        let instance = r#"person-entered!:
  this: did:key:zPersonEnteredSubject
  name: "Tester Joe"
  age: 42
"#;
        let asserted = evaluate(&state, repo, instance, true).await;
        assert!(
            asserted.commits.claims > 0,
            "transient instance assertion must commit claims; saw {}",
            asserted.commits.claims,
        );

        // Query the durable `person` — the rule should have landed
        // a row driven by the transient.
        let query = evaluate(&state, repo, "person:\n", false).await;
        assert_eq!(
            query.matches_after.len(),
            1,
            "expected one query match block for `person:`",
        );
        assert_eq!(
            query.matches_after[0].results.len(),
            1,
            "rule should have produced one durable person row; got {:?}",
            query.matches_after[0].results,
        );
    }

    /// The evaluate pipeline must surface a committed document's
    /// transient facts for post-commit command dispatch — the seam
    /// the route and the bridge hand to `router::command::dispatch`,
    /// mirroring `/transact`. A durable document and a dry run
    /// surface none.
    #[dialog_common::test]
    async fn it_returns_transients_for_command_dispatch() {
        let (state, repo) = state_with_repo("test-evaluate-transients").await;
        let repo = repo.as_str();

        evaluate(&state, repo, CONCEPTS, true).await;

        let guard = state.read().await;
        let instance = r#"person-entered!:
  this: did:key:zPersonEnteredDispatch
  name: "Dispatch Joe"
  age: 9
"#;
        let (_, transients) =
            super::evaluate_body_with_transients(&guard, repo, "main", instance.to_owned(), true)
                .await
                .expect("transient instance evaluates");
        assert!(
            transients.is_some(),
            "a committed transient-concept assertion must surface for dispatch",
        );

        let durable = r#"person!:
  this: did:key:zPersonDurableDispatch
  name: "Durable Joe"
  age: 9
"#;
        let (_, transients) =
            super::evaluate_body_with_transients(&guard, repo, "main", durable.to_owned(), true)
                .await
                .expect("durable instance evaluates");
        assert!(
            transients.is_none(),
            "a durable-only document dispatches nothing",
        );

        let (_, transients) =
            super::evaluate_body_with_transients(&guard, repo, "main", instance.to_owned(), false)
                .await
                .expect("dry run evaluates");
        assert!(transients.is_none(), "a dry run dispatches nothing");
    }

    /// A pure-query document must not advance the branch even with
    /// `transact=true`: nothing is written, so `commits.claims` is
    /// zero and the revision is unchanged.
    #[dialog_common::test]
    async fn it_does_not_commit_a_query_only_document() {
        let (state, repo) = state_with_repo("test-evaluate-query-only").await;
        let repo = repo.as_str();

        // Install a concept so the query resolves.
        evaluate(&state, repo, CONCEPTS, true).await;

        let query = evaluate(&state, repo, "person:\n", true).await;
        assert_eq!(
            query.commits.claims, 0,
            "query-only document must not commit any claims",
        );
        assert_eq!(
            query.revision_after, query.revision_before,
            "query-only document must not advance the branch revision",
        );
    }
}
