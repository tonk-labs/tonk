//! Core ceremony logic for the account service.
//!
//! Ceremonies are written once, generically over [`Store`](crate::store::Store)
//! and [`EmailSender`](crate::email::EmailSender), so the same code runs
//! against the production D1/real-mail backends and the in-memory/captured
//! test doubles.

use crate::error::ErrorCode;
use crate::store::StoreError;

pub mod accounts;
pub mod codes;
pub mod delegation;
pub mod devices;

/// Errors shared by every ceremony in this crate.
#[derive(Debug)]
pub enum CeremonyError {
    /// A request was made too soon after a previous one.
    RateLimited,
    /// A supplied verification code did not check out.
    CodeInvalid,
    /// The requested operation would violate a uniqueness constraint.
    Conflict(String),
    /// Malformed input, or a failed delegation check.
    Invalid(String),
    /// The request lacks valid authentication credentials.
    Unauthorized(String),
    /// Authentication succeeded but the caller lacks permission.
    Forbidden(String),
    /// An unexpected internal failure.
    Internal(String),
}

impl From<StoreError> for CeremonyError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::Conflict(msg) => CeremonyError::Conflict(msg),
            StoreError::Internal(msg) => CeremonyError::Internal(msg),
        }
    }
}

impl CeremonyError {
    /// The API error code this ceremony error maps to.
    pub fn code(&self) -> ErrorCode {
        match self {
            CeremonyError::RateLimited => ErrorCode::RateLimited,
            CeremonyError::CodeInvalid => ErrorCode::Unauthorized,
            CeremonyError::Conflict(_) => ErrorCode::Conflict,
            CeremonyError::Invalid(_) => ErrorCode::InvalidArgument,
            CeremonyError::Unauthorized(_) => ErrorCode::Unauthorized,
            CeremonyError::Forbidden(_) => ErrorCode::Forbidden,
            CeremonyError::Internal(_) => ErrorCode::InternalError,
        }
    }
}
