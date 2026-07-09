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

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_wasm_macros::wasm_compat;
use dialog_reactor::{COMMIT_RETRY_LIMIT, is_head_moved};
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
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate repo={}, branch={}", path.repo, path.branch);
    // A read lock, like `transact`. `evaluate_on_branch` reaches the reactor and
    // operator by shared reference, and serializes its commit on the per-branch
    // transactor lock rather than on the whole of `TonkState`. Holding the write
    // lock here instead serialized every evaluate against every other request —
    // including the language server, whose analysis holds a read guard for the
    // length of a `didChange`. Since the editor auto-fires a dry-run evaluate on
    // each fresh diagnostics frame, that made typing contend with itself.
    let result = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state
            .reactor
            .repository(&path.repo)
            .branch(&path.branch);
        evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
    };

    // A commit moves the branch head. Announce it on the branch's
    // channel so subscribed UIs refresh their revision/sync-state
    // badges without a full refetch (which would tear down the
    // editor). Pure queries and dry runs leave `revision_after ==
    // revision_before`, so they announce nothing.
    if let Ok(Json(response)) = &result
        && let Some(revision) = &response.revision_after
        && response.revision_before.as_ref() != Some(revision)
    {
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
    result
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
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
    log!("evaluate profile branch={}", path.branch);
    // Read lock, for the reasons spelled out in [`evaluate`].
    let result = {
        let tonk_state = state.read().await;
        let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
        evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
    };

    // Same durable-commit gate as [`evaluate`]: pure queries and dry
    // runs leave the head unchanged and announce nothing. The profile
    // routes have no per-endpoint announcement to mirror, so only the
    // cross-cutting local-commit channel is posted.
    if let Ok(Json(response)) = &result
        && let Some(revision) = &response.revision_after
        && response.revision_before.as_ref() != Some(revision)
    {
        broadcast(
            crate::broadcast::LOCAL_COMMIT_CHANNEL,
            &Notification {
                branch: path.branch.clone(),
                revision: revision.clone(),
            },
        );
    }
    result
}

/// Shared body for [`evaluate`] and [`evaluate_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is
/// the only difference between the two routes.
async fn evaluate_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: crate::reactor::BranchReference<'a>,
    body: Bytes,
    query: EvaluateQuery,
) -> Result<Json<EvaluateResponse>, TonkWorkerError> {
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

    // Serialize a committing evaluate on the branch's transactor lock — the same
    // lock the reactor's `Commit::perform` takes for `/transact`. This document's
    // commit is a *dialog* `Transaction::commit()`, which CASes against the head
    // snapshot taken when `branch.transaction()` is built below. The lock excludes
    // the common racer — another committer (a transact, an invite/join/pause) —
    // so those line up here instead of one losing the CAS and having to retry.
    // A sync is the exception the lock can't cover: it advances the head while
    // only briefly holding this lock and releases it between its network fetch and
    // its cell advance, so it can still land in our snapshot→publish window. The
    // commit loop below handles that residual case by refreshing and retrying,
    // exactly as `Commit::perform` does.
    //
    // A dry run (`transact=false`) writes nothing, so it never takes the lock.
    // That's the auto-evaluate the editor fires on every fresh diagnostics frame;
    // keeping it lock-free is the point of holding only a read guard on
    // `TonkState` here.
    let _transacting = if query.transact {
        Some(session.transactor().lock().await)
    } else {
        None
    };

    let branch = session.handle();

    // Evaluate, then commit if the document writes, refreshing and retrying on a
    // head that moved under us. The transactor lock above excludes other
    // *committers*, but a pull advances the head while only briefly holding that
    // lock (and releases it between its network `prepare` and its `commit`), so a
    // sync can still land between the transaction's head snapshot below and our
    // publish. `/transact` handles this the same way (`Commit::perform`'s retry
    // loop); this is the evaluate-side equivalent.
    //
    // The retry re-enters at `branch.transaction()` — not just `.commit()` —
    // because the evaluated transaction borrows the pre-refresh tree: after a
    // refresh we must re-run the evaluation against the new head. Evaluation is
    // pure over (document, head), so replaying it is safe; the document's
    // statements are idempotent asserts/retracts.
    let mut attempt = 0;
    let response = loop {
        let revision_before = branch.revision();

        // Fold the branch's session overlay (ephemeral facts kept out of
        // storage, e.g. an invite's private seed) into the evaluation
        // transaction so every evaluation — the inspector running
        // `invitation:`, preview or explicit — sees the same facts a
        // `query`/`subscribe` does. Folding it here (not gated on
        // `transact`) is what keeps the read paths consistent; the overlay
        // is read-only and gets `induce`-swept on commit (below) so it never
        // lands durably.
        let overlay = session.overlay();
        // The evaluation's match queries (`txn.query()` inside the
        // evaluator) resolve stored `db.rule/*` rules automatically — rule
        // resolution is built into the branch query's layer stack, so the
        // inspector's dry-run preview shows the same deductions a committed
        // `query` / `subscribe` returns.
        let txn = branch.transaction().integrate(overlay.clone());

        let t_eval = web_time::Instant::now();
        let evaluated = syntax
            .evaluate(txn)
            .perform(&tonk_state.operator)
            .await
            .map_err(map_evaluate_error)?;
        let eval_ms = t_eval.elapsed().as_millis();

        // Compute the post-evaluation matches before any commit
        // decision: the txn's overlay already reflects every mutation
        // and induce-pass derivation, so this is the same answer a
        // post-commit branch query would give.
        let t_matches = web_time::Instant::now();
        let matches_after = evaluated
            .matches_after(&tonk_state.operator)
            .await
            .map_err(map_evaluate_error)?;
        let matches_ms = t_matches.elapsed().as_millis();

        // A document commits when it writes anything. `rule!:` is a
        // mutation (the `!` says so) and the analyzer lifts it into a
        // `Statement::InstallEffect`, so a document with any planned
        // statement is the single commit signal.
        if query.transact && evaluated.analysis.analysis.has_statements() {
            let t_commit = web_time::Instant::now();
            // Sweep the overlay before the durable write: the transaction
            // folded the session overlay in for reads, but those ephemeral
            // facts (e.g. an invite seed) must never persist. Retracting them
            // here cancels the integrated asserts so only the document's own
            // statements land.
            let commit = evaluated
                .txn
                .integrate(session.overlay_retraction())
                .commit()
                .perform(&tonk_state.operator)
                .await;
            let revision_after = match commit {
                Ok(revision) => revision,
                Err(e) if is_head_moved(&e) && attempt < COMMIT_RETRY_LIMIT => {
                    attempt += 1;
                    log!("evaluate commit raced (attempt {attempt}); refreshing head and retrying");
                    // Refresh the handle's view of the head, then loop to re-run
                    // the evaluation against it. If even the refresh fails, give
                    // up with the original mismatch.
                    branch
                        .refresh(&tonk_state.operator)
                        .await
                        .map_err(|refresh_err| {
                            map_evaluate_error(EvaluateError::Query(format!(
                                "commit: {e}; refresh after race failed: {refresh_err}"
                            )))
                        })?;
                    continue;
                }
                Err(e) => {
                    return Err(map_evaluate_error(EvaluateError::Query(format!(
                        "commit: {e}"
                    ))));
                }
            };
            let commit_ms = t_commit.elapsed().as_millis();
            // Re-poll subscriptions so SSE clients see the new state.
            // The chain commits via dialog directly; the reactor's
            // subscription registry is the worker's responsibility.
            let t_poll = web_time::Instant::now();
            session.poll(&tonk_state.operator).await;
            let poll_ms = t_poll.elapsed().as_millis();
            let timing = format!(
                "evaluate timing: {exprs} exprs | parse {parse_ms}ms | analyze+eval {eval_ms}ms | matches {matches_ms}ms | commit {commit_ms}ms | poll {poll_ms}ms"
            );
            log!("{timing}");
            // Tee timing onto a BroadcastChannel so a page (or DevTools
            // listener) can read seed numbers without the SW console — the
            // seed runs background in the SW, so its logs never reach the
            // page console.
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            if let Ok(channel) = web_sys::BroadcastChannel::new("tonk-timing") {
                let _ = channel.post_message(&wasm_bindgen::JsValue::from_str(&timing));
            }
            break EvaluateResponse {
                revision_before,
                revision_after: Some(revision_after),
                matches_before: evaluated.matches,
                matches_after,
                commits: evaluated.commits,
            };
        }

        // Pure-query or dry-run: drop the transaction without
        // committing. The pre-mutation matches double as "after"
        // because no mutations actually landed. Zero out
        // `claims` so the response reflects what *did* commit
        // (nothing), not what *would* have committed — auto-
        // evaluate from the editor relies on this to know the
        // branch is untouched.
        let mut commits = evaluated.commits;
        commits.claims = 0;
        break EvaluateResponse {
            revision_before: revision_before.clone(),
            revision_after: revision_before,
            matches_before: evaluated.matches.clone(),
            matches_after: evaluated.matches,
            commits,
        };
    };

    Ok(Json(response))
}

/// Bridge-callable wrapper around the evaluate pipeline. Runs
/// the same logic as [`evaluate_on_branch`] but accepts plain
/// `String` arguments instead of HTTP-level types so the bridge
/// handler can call it without constructing an axum request.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
        .map(|Json(r)| r)
}

/// Like [`evaluate_body`], but against the **profile** repository's
/// branch rather than a named repo. Used to seed the standard library
/// onto the profile meta branch at profile creation, so a
/// `<tonk-display>` reading the profile (e.g. the Hub) can resolve the
/// library's concepts and views there. SW-only — its sole caller
/// (`seed_profile_library`) is gated to the service-worker scope.
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
        .map(|Json(r)| r)
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
/// These drive [`evaluate_body`] — the cfg-`wasm32` seam that runs
/// the *same* `evaluate_on_branch` logic the HTTP handler runs,
/// including the commit guard. They guard the two bug classes that
/// escaped to manual browser testing this session: a `rule!:`-only
/// document silently not committing, and a rule never firing on a
/// transient instance.
///
/// wasm32-only — `evaluate_body` is cfg-`wasm32` and the worker's
/// test `TonkState` is built from the service-worker harness.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::{EvaluateResponse, evaluate_body};
    use crate::router::{AppState, RepositoryInfo, api_router_with_state, tests::test_state};

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
