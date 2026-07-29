//! Error types for the service.

use serde::{Deserialize, Serialize};
use worker::Response;

/// Error codes returned by the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // 400 Bad Request - Input validation errors
    InvalidArgument,

    // 401 Unauthorized - Authentication errors
    SignatureInvalid,
    AudienceMismatch,
    InvocationExpired,

    // 403 Forbidden - Authorization errors
    ChainInvalid,
    CommandMismatch,
    SubjectNotAllowed,
    // Only constructed from the wasm-gated revocation screen
    // (`handlers::ucan::screen_revoked`); a native build never
    // constructs it even though `status_code` matches it exhaustively.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    CredentialRevoked,

    // 500 Internal Server Error
    InternalError,

    // 503 Service Unavailable - the revocation registry could not be
    // consulted and no cached verdict covers the request. Retryable,
    // unlike the 403s above. Same wasm-only construction as
    // `CredentialRevoked`.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    RevocationUnavailable,
}

impl ErrorCode {
    /// Get the HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            // 400 Bad Request
            ErrorCode::InvalidArgument => 400,

            // 401 Unauthorized
            ErrorCode::SignatureInvalid
            | ErrorCode::AudienceMismatch
            | ErrorCode::InvocationExpired => 401,

            // 403 Forbidden
            ErrorCode::ChainInvalid
            | ErrorCode::CommandMismatch
            | ErrorCode::SubjectNotAllowed
            | ErrorCode::CredentialRevoked => 403,

            // 500 Internal Server Error
            ErrorCode::InternalError => 500,

            // 503 Service Unavailable
            ErrorCode::RevocationUnavailable => 503,
        }
    }
}

/// Structured error response.
#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub message: String,
}

impl ErrorResponse {
    /// Create a new error response.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }
    }

    /// Convert to a worker Response.
    pub fn to_response(&self) -> worker::Result<Response> {
        let status = self.error.code.status_code();
        Response::from_json(self).map(|r| r.with_status(status))
    }
}

/// Service error type for internal use.
#[derive(Debug)]
pub struct ServiceError {
    pub code: ErrorCode,
    pub message: String,
}

impl ServiceError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Convert to an error response.
    pub fn to_response(&self) -> worker::Result<Response> {
        ErrorResponse::new(self.code, &self.message).to_response()
    }
}
