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
use dialog_repository::Revision;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_evaluator::evaluate::{EvaluateError, SyntaxEvaluateExt};
use tonk_notation::{Parsed, Syntax, parse};

use super::AppState;
use crate::TonkWorkerError;

// Re-export the response and match types so router consumers
// (router.rs, browser clients via wasm-bindgen) name them
// through this module rather than reaching into tonk-schema.
pub use tonk_evaluator::evaluate::{CommitSummary, QueryMatchBlock, QueryResult};

/// Wire-shape returned by `/evaluate`. Local to the worker so
/// the JSON contract is owned at the HTTP boundary, not in the
/// shared evaluator. Slide owns its own copy of this shape
/// (`slide::output`) so its `-f json` output stays byte-compatible
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
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);
    evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
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
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
    evaluate_on_branch(&tonk_state, tonk_branch, body, query).await
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

    let parsed = parse(text);
    let syntax = surface_parse_diagnostics(parsed)?;

    log!("Evaluating {} expression(s)", syntax.expressions.len());

    let session = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;
    let branch = session.handle();
    let revision_before = branch.revision();

    let evaluated = syntax
        .evaluate(branch)
        .perform(&tonk_state.operator)
        .await
        .map_err(map_evaluate_error)?;

    // A document commits when it writes anything. `rule!:` is a
    // mutation (the `!` says so) and the analyzer lifts it into a
    // `Statement::InstallEffect`, so a document with any planned
    // statement is the single commit signal.
    let response = if query.transact && evaluated.analysis.analysis.has_statements() {
        let result = evaluated
            .commit()
            .perform(&tonk_state.operator)
            .await
            .map_err(map_evaluate_error)?;
        // Re-poll subscriptions so SSE clients see the new state.
        // The chain commits via dialog directly; the reactor's
        // subscription registry is the worker's responsibility.
        session.poll(&tonk_state.operator).await;
        EvaluateResponse {
            revision_before,
            revision_after: Some(result.revision),
            matches_before: result.matches_before,
            matches_after: result.matches_after,
            commits: result.commits,
        }
    } else {
        // Pure-query or dry-run: drop the transaction without
        // committing. The pre-mutation matches double as "after"
        // because no mutations actually landed. Zero out
        // `claims` so the response reflects what *did* commit
        // (nothing), not what *would* have committed — auto-
        // evaluate from the editor relies on this to know the
        // branch is untouched.
        let mut commits = evaluated.commits;
        commits.claims = 0;
        EvaluateResponse {
            revision_before: revision_before.clone(),
            revision_after: revision_before,
            matches_before: evaluated.matches.clone(),
            matches_after: evaluated.matches,
            commits,
        }
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
    use crate::router::{AppState, api_router_with_state, tests::test_state};

    /// Create the test repository via `PUT /api/repository/{name}`,
    /// then hand back the wrapped [`AppState`] so tests can call
    /// [`evaluate_body`] against the same `TonkState` the route
    /// would. The reactor only *loads* repositories — it never
    /// creates them — so the repo must exist before the first
    /// `evaluate_body` call acquires a branch on it.
    ///
    /// Tolerates `412 Precondition Failed`: IndexedDB survives the
    /// single-process wasm test run, so a name reused by a prior
    /// run is already present.
    async fn state_with_repo(repo: &str) -> AppState {
        let (app, state, _lsp) = api_router_with_state(test_state().await);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}"))
                    .method("PUT")
                    .header("content-type", "application/json")
                    .header("if-none-match", "*")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert!(
            status == StatusCode::CREATED || status == StatusCode::PRECONDITION_FAILED,
            "expected 201 or 412 from PUT /api/repository/{repo}, got {status}",
        );
        state
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
    const CONCEPTS: &str = "\
concept!: &person-entered
  transient:
  with:
    name:
      the: xyz.tonk.env/name
      as: text
      cardinality: one
      description: \"name\"
    age:
      the: xyz.tonk.env/age
      as: unsigned-integer
      cardinality: one
      description: \"age\"

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
  description: \"A person\"
  with:
    name: person-name
    age: person-age
";

    /// The rule person <- person-entered, as its own document.
    const RULE: &str = "\
rule!:
  assert!: person
  when:
    - assert: person-entered
      where: { this: ?this, name: ?name, age: ?age }
";

    /// Regression: a document that is *only* a `rule!:` is a
    /// mutation document — the `!` says so. It must commit. Before
    /// the fix the commit guard checked the wrong condition and
    /// rule-only documents were silently dropped: the rule never
    /// reached the branch.
    #[dialog_common::test]
    async fn it_commits_a_rule_only_document() {
        let repo = "test-evaluate-rule-only";
        let state = state_with_repo(repo).await;

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
        let repo = "test-evaluate-rule-fires";
        let state = state_with_repo(repo).await;

        evaluate(&state, repo, CONCEPTS, true).await;
        evaluate(&state, repo, RULE, true).await;

        // Assert a transient `person-entered` instance. The write
        // seeds the effects fixpoint that drives the rule.
        let instance = "\
person-entered!:
  this: did:key:zPersonEnteredSubject
  name: \"Tester Joe\"
  age: 42
";
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
        let repo = "test-evaluate-query-only";
        let state = state_with_repo(repo).await;

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
