use thiserror::Error;

/// Errors that can occur in the Tonk UI application.
#[derive(Error, Debug, Clone)]
pub enum TonkUiError {
    /// Error from the local API.
    #[error("Error from local API: {0}")]
    ApiError(String),
}
