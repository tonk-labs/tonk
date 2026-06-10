//! Error payloads a consumer element dispatches as a `*:error`
//! custom event (e.g. `<tonk-display>`'s `tonk-display:error`).

use serde::Serialize;

/// What went wrong. Browser code reads this off `event.detail`.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorDetail {
    /// Stable kind identifier — `unknown-source`, `network`,
    /// `parse`, `descriptor`.
    pub kind: ErrorKind,
    /// Human-readable description.
    pub message: String,
}

/// Categories of failure surfaced by the element.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// Phase-1 returned no concept matching `source`.
    UnknownSource,
    /// HTTP / fetch / SSE transport failure.
    Network,
    /// JSON or wire-shape parsing failed.
    Parse,
    /// Descriptor reconstruction or `phase2_query` rejected the
    /// descriptor.
    Descriptor,
}

impl ErrorDetail {
    /// Construct a new error detail.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}
