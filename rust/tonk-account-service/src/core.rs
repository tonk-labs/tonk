//! Core ceremony logic for the account service.
//!
//! Ceremonies are written once, generically over [`Store`](crate::store::Store)
//! and [`EmailSender`](crate::email::EmailSender), so the same code runs
//! against the production D1/real-mail backends and the in-memory/captured
//! test doubles.

use crate::chains::ChainError;
use crate::error::ErrorCode;
use crate::store::StoreError;

pub mod accounts;
pub mod backup;
pub mod codes;
pub mod delegation;
pub mod descriptor;
pub mod devices;
pub mod enrollment;
pub mod links;

/// Errors shared by every ceremony in this crate.
#[derive(Debug)]
pub enum CeremonyError {
    /// A request was made too soon after a previous one.
    RateLimited,
    /// A supplied verification code did not check out.
    CodeInvalid,
    /// The requested operation would violate a uniqueness constraint.
    Conflict(String),
    /// A generation-bound resource does not exist.
    NotFound(String),
    /// Malformed input, or a failed delegation check.
    Invalid(String),
    /// The request lacks valid authentication credentials.
    Unauthorized(String),
    /// Authentication succeeded but the caller lacks permission.
    Forbidden(String),
    /// An unexpected internal failure.
    Internal(String),
}

/// The message returned for a uniqueness conflict a ceremony has not
/// explained in its own terms.
pub const GENERIC_CONFLICT: &str = "conflicts with existing state";

/// Log a detail that must not be returned to the caller.
pub fn log_detail(detail: &str) {
    #[cfg(target_arch = "wasm32")]
    worker::console_error!("{detail}");
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{detail}");
}

impl From<StoreError> for CeremonyError {
    /// A storage-layer conflict carries the database driver's own text
    /// ("UNIQUE constraint failed: accounts.email", plus a JS stack trace
    /// under D1), which names tables and columns and must not reach a
    /// caller. The detail is logged and replaced with
    /// [`GENERIC_CONFLICT`]; ceremonies that can say something accurate
    /// and actionable build [`CeremonyError::Conflict`] directly instead
    /// of relying on this conversion.
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::Conflict(detail) => {
                log_detail(&format!("storage conflict: {detail}"));
                CeremonyError::Conflict(GENERIC_CONFLICT.to_string())
            }
            StoreError::Internal(msg) => CeremonyError::Internal(msg),
        }
    }
}

impl From<ChainError> for CeremonyError {
    fn from(err: ChainError) -> Self {
        match err {
            ChainError::Internal(msg) => CeremonyError::Internal(msg),
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
            CeremonyError::NotFound(_) => ErrorCode::NotFound,
            CeremonyError::Invalid(_) => ErrorCode::InvalidArgument,
            CeremonyError::Unauthorized(_) => ErrorCode::Unauthorized,
            CeremonyError::Forbidden(_) => ErrorCode::Forbidden,
            CeremonyError::Internal(_) => ErrorCode::InternalError,
        }
    }
}
