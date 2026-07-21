//! Error types for the service.

use serde::Serialize;
use worker::Response;

/// Error codes returned by the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Invalid or malformed request argument (HTTP 400).
    InvalidArgument,
    /// Request lacks valid authentication credentials (HTTP 401).
    Unauthorized,
    /// Authentication succeeded but insufficient permissions (HTTP 403).
    Forbidden,
    /// Requested resource not found (HTTP 404).
    NotFound,
    /// Request conflicts with existing state (HTTP 409).
    Conflict,
    /// Request rate limit exceeded (HTTP 429).
    RateLimited,
    /// Internal server error (HTTP 500).
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
    /// The error code classifying the failure.
    pub code: ErrorCode,
    /// Human-readable error message.
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
