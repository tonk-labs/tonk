//! Error types for the Tonk worker.

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use thiserror::Error;

/// Errors that can occur in the Tonk worker.
#[derive(Error, Debug)]
pub enum TonkWorkerError {
    /// An error occurred in the router.
    #[error("Router error: {0}")]
    Router(String),

    /// An internal error occurred.
    #[error("Internal error: {0}")]
    Internal(String),

    /// A requested resource was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// A resource already exists where a create was requested.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// An HTTP precondition (e.g. `If-None-Match: *`) was not met.
    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),
}

impl TonkWorkerError {
    fn kind(&self) -> &'static str {
        match self {
            TonkWorkerError::Router(_) => "router",
            TonkWorkerError::Internal(_) => "internal",
            TonkWorkerError::NotFound(_) => "not_found",
            TonkWorkerError::Conflict(_) => "conflict",
            TonkWorkerError::PreconditionFailed(_) => "precondition_failed",
        }
    }

    fn message(&self) -> String {
        match self {
            TonkWorkerError::Router(m)
            | TonkWorkerError::Internal(m)
            | TonkWorkerError::NotFound(m)
            | TonkWorkerError::Conflict(m)
            | TonkWorkerError::PreconditionFailed(m) => m.clone(),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    kind: &'static str,
    message: String,
}

impl IntoResponse for TonkWorkerError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            TonkWorkerError::NotFound(_) => StatusCode::NOT_FOUND,
            TonkWorkerError::Router(_) => StatusCode::BAD_REQUEST,
            TonkWorkerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            TonkWorkerError::Conflict(_) => StatusCode::CONFLICT,
            TonkWorkerError::PreconditionFailed(_) => StatusCode::PRECONDITION_FAILED,
        };
        let body = ErrorBody {
            error: ErrorDetail {
                kind: self.kind(),
                message: self.message(),
            },
        };
        (status, Json(body)).into_response()
    }
}
