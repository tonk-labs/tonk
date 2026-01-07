//! Error types for the service.

use serde::Serialize;
use worker::Response;

/// Error codes returned by the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // 400 Bad Request - Input validation errors
    InvalidRequestBody,
    InvalidBase64,
    InvalidCbor,
    MissingArgument,
    InvalidArgument,
    UnknownCapability,

    // 401 Unauthorized - Authentication errors
    SignatureInvalid,
    AudienceMismatch,
    InvocationExpired,
    ProofNotFound,
    ProofExpired,
    ProofNotYetValid,

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
            ErrorCode::InvalidRequestBody
            | ErrorCode::InvalidBase64
            | ErrorCode::InvalidCbor
            | ErrorCode::MissingArgument
            | ErrorCode::InvalidArgument
            | ErrorCode::UnknownCapability => 400,

            // 401 Unauthorized
            ErrorCode::SignatureInvalid
            | ErrorCode::AudienceMismatch
            | ErrorCode::InvocationExpired
            | ErrorCode::ProofNotFound
            | ErrorCode::ProofExpired
            | ErrorCode::ProofNotYetValid => 401,

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

    // Convenience constructors for common errors

    pub fn invalid_request_body(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequestBody, message)
    }

    pub fn invalid_base64(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidBase64, message)
    }

    pub fn invalid_cbor(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidCbor, message)
    }

    pub fn missing_argument(arg: &str) -> Self {
        Self::new(
            ErrorCode::MissingArgument,
            format!("Missing required argument: {}", arg),
        )
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn unknown_capability(cmd: &str) -> Self {
        Self::new(
            ErrorCode::UnknownCapability,
            format!("Unknown capability: {}", cmd),
        )
    }

    pub fn signature_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::SignatureInvalid, message)
    }

    pub fn audience_mismatch(expected: &str, got: &str) -> Self {
        Self::new(
            ErrorCode::AudienceMismatch,
            format!("Audience mismatch: expected {}, got {}", expected, got),
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

    pub fn proof_expired(index: usize) -> Self {
        Self::new(
            ErrorCode::ProofExpired,
            format!("Proof[{}] has expired", index),
        )
    }

    pub fn proof_not_yet_valid(index: usize) -> Self {
        Self::new(
            ErrorCode::ProofNotYetValid,
            format!("Proof[{}] is not yet valid", index),
        )
    }

    pub fn chain_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ChainInvalid, message)
    }

    pub fn command_mismatch(expected: &str, found: &str) -> Self {
        Self::new(
            ErrorCode::CommandMismatch,
            format!(
                "Command mismatch: invoked {}, but proof authorizes {}",
                expected, found
            ),
        )
    }

    pub fn subject_not_allowed() -> Self {
        Self::new(ErrorCode::SubjectNotAllowed, "Subject not allowed by proof")
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}
