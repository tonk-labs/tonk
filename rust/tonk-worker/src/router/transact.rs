//! Transaction route — accepts a tonk-schema transaction document
//! (JSON or YAML) and commits all derived facts in a single
//! transaction.
//!
//! See [`tonk_schema::transact`] for the notation specification.

use ::axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{Attribute, Entity, Statement, Update, Value};
use dialog_repository::RepositoryExt as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_schema::transact::{self, Claim, ParseError};

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for the transact route.
#[derive(Debug, Deserialize)]
pub struct TransactPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response from a successful transaction.
#[derive(Debug, Serialize, Deserialize)]
pub struct TransactResponse {
    /// Number of EAV claims committed.
    pub claims: usize,
    /// Resolved entity URI for every named subject in the document.
    pub bookmarks: BTreeMap<String, String>,
}

/// A claim ready for the transaction API. Implements [`Statement`]
/// by forwarding to [`Update::associate`] so we can stage runtime-
/// determined `(the, of, is)` triples.
struct RawClaim {
    the: Attribute,
    of: Entity,
    is: Value,
}

impl From<Claim> for RawClaim {
    fn from(claim: Claim) -> Self {
        Self {
            the: claim.the,
            of: claim.of,
            is: claim.is,
        }
    }
}

impl Statement for RawClaim {
    fn assert(self, update: &mut impl Update) {
        update.associate(self.the, self.of, self.is);
    }

    fn retract(self, update: &mut impl Update) {
        update.dissociate(self.the, self.of, self.is);
    }
}

/// `POST /api/repository/{repo}/branch/{branch}/transact`
///
/// Body: a tonk-schema transaction document, encoded as JSON
/// (`Content-Type: application/json`) or YAML (`Content-Type:
/// application/yaml` or `text/yaml`). Empty documents are valid
/// and produce a no-op (zero claims).
#[wasm_compat]
pub async fn transact(
    State(state): State<AppState>,
    Path(path): Path<TransactPath>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TransactResponse>, TonkWorkerError> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let parsed = parse_body(content_type, &body).map_err(parse_error_to_router)?;

    log!(
        "Transacting {} claim(s) on repo={}, branch={}",
        parsed.claims.len(),
        path.repo,
        path.branch,
    );

    let bookmarks: BTreeMap<String, String> = parsed
        .bookmarks
        .iter()
        .map(|(name, entity)| (name.clone(), entity.to_string()))
        .collect();
    let claim_count = parsed.claims.len();

    if parsed.claims.is_empty() {
        return Ok(Json(TransactResponse {
            claims: 0,
            bookmarks,
        }));
    }

    let tonk_state = state.write().await;

    let repo = tonk_state
        .profile
        .repository(&path.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", path.repo, e))
        })?;

    let branch = repo
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", path.branch, e))
        })?;

    let mut transaction = branch.transaction();
    for claim in parsed.claims {
        transaction = transaction.assert(RawClaim::from(claim));
    }

    transaction
        .commit()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            log!("Transaction commit failed: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to commit transaction: {}", e))
        })?;

    Ok(Json(TransactResponse {
        claims: claim_count,
        bookmarks,
    }))
}

fn parse_body(content_type: &str, body: &[u8]) -> Result<transact::Transaction, ParseError> {
    let text = std::str::from_utf8(body).map_err(|e| ParseError::InvalidDescriptor {
        kind: "document",
        subject: String::new(),
        reason: format!("body is not valid UTF-8: {e}"),
    })?;

    if content_type.starts_with("application/yaml") || content_type.starts_with("text/yaml") {
        transact::parse_yaml(text)
    } else {
        transact::parse_json(text)
    }
}

fn parse_error_to_router(err: ParseError) -> TonkWorkerError {
    TonkWorkerError::Router(err.to_string())
}
