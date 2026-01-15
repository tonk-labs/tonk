//! Error types for the service.
//!
//! This module wraps `dialog_ucan` error types with worker-specific
//! response conversion functionality.

use serde::Serialize;
use worker::Response;

// Re-export ErrorCode from dialog_ucan
pub use dialog_ucan::ErrorCode;

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

/// Service error type with worker-specific response conversion.
///
/// This wraps `dialog_ucan::ServiceError` and adds the ability to convert
/// to a Cloudflare Worker Response.
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

    /// Convert to a worker error response.
    pub fn to_response(&self) -> worker::Result<Response> {
        ErrorResponse::new(self.code, &self.message).to_response()
    }

    // Convenience constructors for common errors

    pub fn invalid_base64(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidBase64, message)
    }

    pub fn invalid_cbor(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidCbor, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn signature_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::SignatureInvalid, message)
    }

    pub fn audience_mismatch(expected: &str, got: &str) -> Self {
        Self::new(
            ErrorCode::AudienceMismatch,
            format!(
                "Audience mismatch: audience ({}) must equal subject ({})",
                got, expected
            ),
        )
    }

    pub fn invocation_expired() -> Self {
        Self::new(ErrorCode::InvocationExpired, "Invocation has expired")
    }

    pub fn proof_not_found(cid: &str) -> Self {
        Self::new(
            ErrorCode::ProofNotFound,
            format!("Proof not found: {}", cid),
        )
    }

    pub fn chain_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ChainInvalid, message)
    }

    pub fn subject_not_allowed() -> Self {
        Self::new(ErrorCode::SubjectNotAllowed, "Subject not allowed by proof")
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}

/// Convert from `dialog_ucan::ServiceError` to our worker-aware `ServiceError`.
impl From<dialog_ucan::ServiceError> for ServiceError {
    fn from(err: dialog_ucan::ServiceError) -> Self {
        Self {
            code: err.code,
            message: err.message,
        }
    }
}
