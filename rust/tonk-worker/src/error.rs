//! Error types for the Tonk worker.

use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use thiserror::Error;
use tonk_analyzer::analyzer::AnalyzeError;

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

    /// The caller is not permitted to perform the operation.
    #[error("Forbidden: {0}")]
    Forbidden(String),

    /// A typed non-success response from an upstream service.
    #[error("Upstream service returned HTTP {status}: {message}")]
    Upstream {
        /// Original upstream HTTP status.
        status: u16,
        /// Stable upstream classification, when supplied.
        code: Option<String>,
        /// Bounded caller-safe message.
        message: String,
    },

    /// Durable identity must be provisioned before this operation.
    #[error("A local passkey root is required")]
    RootRequired,
    /// Linked account state is not ready for authoritative writes.
    #[error("Account state unavailable: {0}")]
    AccountStateUnavailable(String),

    /// An analyzer rejection — preserved structurally so the
    /// editor can attach the diagnostic to the offending source
    /// span instead of rendering the message as a banner.
    /// Always returned with HTTP 400.
    #[error("Analyzer error: {message}")]
    Analyze {
        /// Stable error code (`E_INCOMPLETE_ASSERTION`,
        /// `E_PARSE`, etc). Parser diagnostics carry owned
        /// strings; analyzer diagnostics carry `&'static str`s
        /// that get copied into the owned shape at conversion
        /// time. Either way the editor reads it as the routing
        /// key for the squiggle.
        code: String,
        /// Human-readable message.
        message: String,
        /// Source range in the submitted document, when known.
        range: Option<lsp_types::Range>,
    },
}

impl From<AnalyzeError> for TonkWorkerError {
    fn from(error: AnalyzeError) -> Self {
        Self::Analyze {
            code: error.code().to_owned(),
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

    /// Durable creation was attempted before provisioning a local root.
    #[error("A local passkey root is required")]
    RootRequired,

    /// Any other failure during construction: a dialog-db operation failed.
    #[error("Internal repository error: {0}")]
    Internal(String),
}

impl From<RepositoryError> for TonkWorkerError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::InvalidConfiguration(m) => TonkWorkerError::Router(m),
            RepositoryError::RootRequired => TonkWorkerError::RootRequired,
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
            TonkWorkerError::Forbidden(_) => "forbidden",
            TonkWorkerError::Upstream { .. } => "upstream",
            TonkWorkerError::RootRequired => "conflict",
            TonkWorkerError::AccountStateUnavailable(_) => "account_state_unavailable",
            TonkWorkerError::Analyze { .. } => "analyze",
        }
    }

    fn message(&self) -> String {
        match self {
            TonkWorkerError::Router(m)
            | TonkWorkerError::Internal(m)
            | TonkWorkerError::NotFound(m)
            | TonkWorkerError::Conflict(m)
            | TonkWorkerError::PreconditionFailed(m)
            | TonkWorkerError::Forbidden(m)
            | TonkWorkerError::AccountStateUnavailable(m) => m.clone(),
            TonkWorkerError::RootRequired => "a local passkey root is required".to_string(),
            TonkWorkerError::Upstream { message, .. } => message.clone(),
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
    /// Stable error code, when the kind carries one. Today the
    /// `analyze` kind populates this with codes like
    /// `E_INCOMPLETE_ASSERTION` (analyzer-emitted) and
    /// `E_PARSE` (parser-emitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
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
            TonkWorkerError::Conflict(_) | TonkWorkerError::RootRequired => StatusCode::CONFLICT,
            TonkWorkerError::PreconditionFailed(_) => StatusCode::PRECONDITION_FAILED,
            TonkWorkerError::Forbidden(_) => StatusCode::FORBIDDEN,
            TonkWorkerError::Upstream { status, .. } => StatusCode::from_u16(*status)
                .ok()
                .filter(|status| status.is_client_error() || status.is_server_error())
                .unwrap_or(StatusCode::BAD_GATEWAY),
            TonkWorkerError::AccountStateUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        };
        let (code, range) = match &self {
            TonkWorkerError::Analyze { code, range, .. } => (Some(code.clone()), *range),
            TonkWorkerError::RootRequired => (Some("ROOT_REQUIRED".to_string()), None),
            TonkWorkerError::Upstream { code, .. } => (code.clone(), None),
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
