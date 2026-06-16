use thiserror::Error;

/// Errors that can occur in the Tonk UI application.
#[derive(Error, Debug, Clone)]
pub enum TonkUiError {
    /// Error from the local API.
    #[error("Error from local API: {0}")]
    ApiError(String),

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

    /// Catch-all for errors that don't fit the API or analyzer
    /// shapes (FS Access API failures, missing browser globals,
    /// malformed stored state, etc.).
    #[error("{0}")]
    Other(String),
}

impl TonkUiError {
    /// Convenience constructor for the [`TonkUiError::Other`] catch-all.
    pub fn other(message: impl Into<String>) -> Self {
        TonkUiError::Other(message.into())
    }
}
