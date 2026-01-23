//! Error types for the service.

use serde::Serialize;
use worker::Response;

/// Error codes returned by the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

    // 500 Internal Server Error
    InternalError,
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
            ErrorCode::ChainInvalid | ErrorCode::CommandMismatch | ErrorCode::SubjectNotAllowed => {
                403
            }

            // 500 Internal Server Error
            ErrorCode::InternalError => 500,
        }
    }
}

/// Structured error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
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
