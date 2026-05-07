//! Error types for the Tonk worker.

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use thiserror::Error;
use tonk_schema::analyzer::AnalyzeError;

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

    /// An analyzer rejection — preserved structurally so the
    /// editor can attach the diagnostic to the offending source
    /// span instead of rendering the message as a banner.
    /// Always returned with HTTP 400.
    #[error("Analyzer error: {message}")]
    Analyze {
        /// Stable error code (`E_INCOMPLETE_ASSERTION`, etc).
        code: &'static str,
        /// Human-readable message.
        message: String,
        /// Source range in the submitted document, when known.
        range: Option<lsp_types::Range>,
    },
}

impl From<AnalyzeError> for TonkWorkerError {
    fn from(error: AnalyzeError) -> Self {
        Self::Analyze {
            code: error.code(),
            message: error.kind.to_string(),
            range: error.range,
        }
    }
}

/// Failure mode when assembling a repository from a
/// [`RepositoryConfiguration`][crate::router::RepositoryConfiguration].
///
/// Separate from [`TonkWorkerError`] so the core "create this
/// repository" helper doesn't have to know about HTTP. The
/// `From` impl below maps each variant to the HTTP status the
/// handler wants: invalid configuration → 400, internal → 500.
#[derive(Error, Debug)]
pub enum RepositoryError {
    /// The request body refers to something that doesn't exist
    /// — currently only "branch upstream references a remote
    /// that wasn't in the `remote` map." User-supplied and thus
    /// a 4xx upstream.
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// Any other failure during construction: a dialog-db
    /// operation failed (create / open / commit), the meta
    /// branch couldn't be written, delegation couldn't be
    /// saved, etc.
    #[error("Internal repository error: {0}")]
    Internal(String),
}

impl From<RepositoryError> for TonkWorkerError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::InvalidConfiguration(m) => TonkWorkerError::Router(m),
            RepositoryError::Internal(m) => TonkWorkerError::Internal(m),
        }
    }
}

impl TonkWorkerError {
    fn kind(&self) -> &'static str {
        match self {
            TonkWorkerError::Router(_) => "router",
            TonkWorkerError::Internal(_) => "internal",
            TonkWorkerError::NotFound(_) => "not_found",
            TonkWorkerError::Conflict(_) => "conflict",
            TonkWorkerError::PreconditionFailed(_) => "precondition_failed",
            TonkWorkerError::Analyze { .. } => "analyze",
        }
    }

    fn message(&self) -> String {
        match self {
            TonkWorkerError::Router(m)
            | TonkWorkerError::Internal(m)
            | TonkWorkerError::NotFound(m)
            | TonkWorkerError::Conflict(m)
            | TonkWorkerError::PreconditionFailed(m) => m.clone(),
            TonkWorkerError::Analyze { message, .. } => message.clone(),
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
    /// Stable error code, when the kind carries one. Today only
    /// the `analyze` kind populates this with codes like
    /// `E_INCOMPLETE_ASSERTION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    /// Source range in the submitted document, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<lsp_types::Range>,
}

impl IntoResponse for TonkWorkerError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            TonkWorkerError::NotFound(_) => StatusCode::NOT_FOUND,
            TonkWorkerError::Router(_) | TonkWorkerError::Analyze { .. } => StatusCode::BAD_REQUEST,
            TonkWorkerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            TonkWorkerError::Conflict(_) => StatusCode::CONFLICT,
            TonkWorkerError::PreconditionFailed(_) => StatusCode::PRECONDITION_FAILED,
        };
        let (code, range) = match &self {
            TonkWorkerError::Analyze { code, range, .. } => (Some(*code), *range),
            _ => (None, None),
        };
        let body = ErrorBody {
            error: ErrorDetail {
                kind: self.kind(),
                message: self.message(),
                code,
                range,
            },
        };
        (status, Json(body)).into_response()
    }
}
