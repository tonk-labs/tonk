//! Error types for the Tonk worker.

use axum::http::StatusCode;
use axum::response::IntoResponse;
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
}

impl IntoResponse for TonkWorkerError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            TonkWorkerError::NotFound(_) => StatusCode::NOT_FOUND,
            TonkWorkerError::Router(_) => StatusCode::BAD_REQUEST,
            TonkWorkerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
