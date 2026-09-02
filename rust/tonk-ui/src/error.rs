use thiserror::Error;

/// Closed transport boundary for account API failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountTransportKind {
    /// Request did not receive an HTTP response.
    Network,
    /// HTTP response reported a failure.
    Http,
    /// Response body did not match the expected wire shape.
    Decode,
    /// Browser-local state could not satisfy the operation.
    Local,
}

/// Errors that can occur in the Tonk UI application.
#[derive(Error, Debug, Clone)]
pub enum TonkUiError {
    /// Error from the local API.
    #[error("Error from local API: {0}")]
    ApiError(String),

    /// A curated, already-caller-facing message from an account flow.
    /// Shown verbatim because `ApiError` wrapping would bury the sentence
    /// someone needs to read beneath transport details.
    #[error("{0}")]
    Account(String),

    /// Structured account boundary evidence plus a local-only diagnostic.
    #[error("{diagnostic}")]
    AccountApi {
        /// Which boundary failed.
        transport_kind: AccountTransportKind,
        /// Numeric HTTP status, when a response was received.
        status: Option<u16>,
        /// Stable service code. It is normalized again by analytics before use.
        service_code: Option<String>,
        /// Exact local diagnostic; never included in account analytics.
        diagnostic: String,
    },

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
