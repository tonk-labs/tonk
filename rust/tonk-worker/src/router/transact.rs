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
use tonk_analyzer::evaluate::CommitSummary;
use tonk_common::log;
use tonk_schema::mutation::TransactRequest;

use super::AppState;
use crate::TonkWorkerError;
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
/// surface of [`tonk_analyzer::evaluate::EvaluateResponse`] minus
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
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    log!("transact repo={}, branch={}", path.repo, path.branch);
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state
        .reactor
        .repository(&path.repo)
        .branch(&path.branch);
    transact_on_branch(&tonk_state, tonk_branch, body).await
}

/// `POST /api/profile/branch/{branch}/transact`
#[wasm_compat]
pub async fn transact_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileTransactPath>,
    _headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    log!("transact profile branch={}", path.branch);
    let tonk_state = state.write().await;
    let tonk_branch = tonk_state.reactor.profile_repository().branch(&path.branch);
    transact_on_branch(&tonk_state, tonk_branch, body).await
}

async fn transact_on_branch<'a>(
    tonk_state: &'a crate::worker::TonkState,
    tonk_branch: BranchReference<'a>,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    let request: TransactRequest = serde_json::from_slice(&body)
        .map_err(|e| TonkWorkerError::Router(format!("invalid TransactRequest body: {e}")))?;

    let session = tonk_branch
        .acquire(&tonk_state.operator)
        .await
        .map_err(reactor_to_error)?;

    let revision_before = session.handle().revision();
    let claim_count = request.mutations.len();

    // No mutations: short-circuit to a no-op response so callers
    // can submit empty batches without paying for a commit.
    if request.mutations.is_empty() {
        return Ok(Json(TransactResponse {
            revision_before: revision_before.clone(),
            revision_after: revision_before,
            commits: CommitSummary::default(),
        }));
    }

    let mut builder = tonk_branch.transaction();
    for mutation in request.mutations {
        builder = builder.apply(mutation);
    }
    let revision_after = builder
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(reactor_to_error)?;

    Ok(Json(TransactResponse {
        revision_before,
        revision_after: Some(revision_after),
        commits: CommitSummary {
            claims: claim_count,
            entities: Default::default(),
        },
    }))
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
