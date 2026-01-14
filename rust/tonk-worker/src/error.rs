//! Error types for the Tonk worker.

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
}

impl IntoResponse for TonkWorkerError {
    fn into_response(self) -> axum::response::Response {
        self.to_string().into_response()
    }
}
