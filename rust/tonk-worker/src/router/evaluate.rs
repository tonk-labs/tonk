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
///
/// Note: after the `tonk_schema::evaluate` extraction, the
/// shared pipeline does not yet accept a `transact` toggle.
/// The query parameter is still parsed (so callers keep their
/// existing URL surface) but is currently ignored — every
/// invocation commits. Threading `transact` back through
/// `evaluate::run` is a follow-up in `tonk-schema`.
#[derive(Debug, Deserialize)]
pub struct EvaluateQuery {
    /// When `false`, the caller wants analysis + queries only,
    /// without applying mutations. Currently ignored — see the
    /// note on the type.
    ///
    /// Defaults to `true` so existing callers keep today's
    /// behavior. Accepts `true`/`false`, `1`/`0`, `yes`/`no`.
    #[serde(default = "default_true", deserialize_with = "deserialize_bool")]
    #[allow(dead_code)]
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
    _query: EvaluateQuery,
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

    let outcome = evaluate::run(&syntax, session.handle(), &tonk_state.operator)
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
/// carrying the diagnostic messages.
fn surface_parse_diagnostics(parsed: Parsed) -> Result<Syntax, TonkWorkerError> {
    if !parsed.diagnostics.is_empty() {
        let messages = parsed
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(TonkWorkerError::Router(messages));
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
        EvaluateError::Analyze(message) => {
            log!("Analyzer rejected document: {message}");
            TonkWorkerError::Router(message)
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
