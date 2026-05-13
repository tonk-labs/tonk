//! Evaluate route — accepts an asserted-notation document and
//! drives the unified analyze → query → plan → commit pipeline.
//!
//! Implementation lives in [`tonk_schema::evaluate`]; this module
//! is the axum adapter: parse the body, surface parse diagnostics
//! as 400s, acquire the cached branch via the reactor (so the
//! handle is reused across requests), call
//! [`tonk_schema::evaluate::run`], then re-poll subscriptions when
//! the document committed.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_wasm_macros::wasm_compat;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_notation::{Parsed, Syntax, parse};
use tonk_schema::evaluate::{self, EvaluateError};

use super::AppState;
use crate::TonkWorkerError;

pub use tonk_schema::evaluate::{CommitSummary, EvaluateResponse, QueryMatchBlock, QueryResult};

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

    // Acquire the cached branch via the reactor so the same
    // handle is reused across requests and subscription polling
    // sees commits emitted by the shared evaluator below.
    let session = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(|e| TonkWorkerError::NotFound(e.to_string()))?;

    let outcome = evaluate::run(
        &syntax,
        session.handle(),
        &tonk_state.operator,
        query.transact,
    )
    .await
    .map_err(map_evaluate_error)?;

    // Subscriptions on this branch only need re-polling when the
    // document committed — pure-query docs leave branch state
    // unchanged, so existing subscribers' results couldn't have
    // changed.
    if outcome.committed {
        session.poll(&tonk_state.operator).await;
    }

    Ok(Json(outcome.response))
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
/// Analyze-time failures are the user's fault (400); query, plan,
/// and commit failures are internal (500).
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
        EvaluateError::Commit(message) => {
            log!("Transaction commit failed: {message}");
            TonkWorkerError::Internal(message)
        }
    }
}
