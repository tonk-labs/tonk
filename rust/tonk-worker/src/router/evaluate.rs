//! Evaluate route — accepts an asserted-notation document and
//! drives the analyze → query → mutation pipeline against the
//! branch.
//!
//! The actual analyze + plan logic lives in
//! [`tonk_schema::evaluate`] behind the
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
use tonk_notation::{Parsed, Syntax, parse};
use tonk_schema::evaluate::{EvaluateError, SyntaxEvaluateExt};

use super::AppState;
use crate::TonkWorkerError;

// Re-export the response and match types so router consumers
// (router.rs, browser clients via wasm-bindgen) name them
// through this module rather than reaching into tonk-schema.
pub use tonk_schema::evaluate::{CommitSummary, QueryMatchBlock, QueryResult};

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
        .evaluate(branch.transaction())
        .perform(branch, &tonk_state.operator)
        .await
        .map_err(map_evaluate_error)?;

    // A document commits when it writes anything. `rule!:` is a
    // mutation (the `!` says so), but the analyzer lifts it into
    // `analysis.effects` rather than `mutate.statements`, so a
    // pure-`rule!:` document has empty `statements`. Both buckets
    // must be consulted, otherwise the rule is silently dropped.
    // (The deeper fix — folding effects into the mutation analysis
    // so `rule!:` is just another statement — rides with the
    // analyzer IR rework.)
    let has_work =
        !evaluated.analysis.mutate.statements.is_empty() || !evaluated.analysis.effects.is_empty();
    let response = if query.transact && has_work {
        let result = evaluated
            .commit()
            .perform(branch, &tonk_state.operator)
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
