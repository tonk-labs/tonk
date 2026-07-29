use thiserror::Error;

/// Errors that can occur in the Tonk UI application.
#[derive(Error, Debug, Clone)]
pub enum TonkUiError {
    /// Error from the local API.
    #[error("Error from local API: {0}")]
    ApiError(String),

    /// Structured synchronization failure returned by the worker.
    #[error("{message}")]
    Sync {
        /// Stable routing code such as `CREDENTIAL_REVOKED`.
        code: String,
        /// Fixed caller-safe message.
        message: String,
    },

    /// Structured analyzer error returned by the worker —
    /// preserved so the UI can route it to the editor as a
    /// source-positioned diagnostic instead of a banner.
    #[error("{message}")]
    Analyze {
        /// Stable error code (`E_INCOMPLETE_ASSERTION`, etc).
        code: String,
        /// Human-readable message.
        message: String,
        /// Source range in the submitted document, when known.
        range: Option<lsp_types::Range>,
    },
}
