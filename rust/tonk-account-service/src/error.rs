//! Error types for the service.

use serde::Serialize;
use worker::Response;

/// Error codes returned by the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    InternalError,
}

impl ErrorCode {
    /// The HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            ErrorCode::InvalidArgument => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::Forbidden => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::Conflict => 409,
            ErrorCode::RateLimited => 429,
            ErrorCode::InternalError => 500,
        }
    }
}

/// Structured error carried through handlers and serialized to the body.
#[derive(Debug, Serialize)]
pub struct ServiceError {
    pub code: ErrorCode,
    pub message: String,
}

impl ServiceError {
    /// Create a new service error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Convert to a worker Response with the matching status.
    pub fn to_response(&self) -> worker::Result<Response> {
        Response::from_json(&serde_json::json!({ "error": self }))
            .map(|r| r.with_status(self.code.status_code()))
    }
}
