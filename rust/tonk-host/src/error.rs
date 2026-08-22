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
    /// The HTTP status the transport reported, when the failure came
    /// from a response rather than the transport itself. `None` for a
    /// dropped connection, a parse failure, or a bad descriptor.
    ///
    /// Carried structurally because the status is the difference
    /// between failures that a retry can heal (a restarting worker) and
    /// answers that will never change (`404` — this repo is not one
    /// this device holds). Consumers classified on `kind` alone and so
    /// read every non-OK response as a transport hiccup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
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
    /// Construct a new error detail with no HTTP status.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
        }
    }

    /// Construct an error detail for a non-OK HTTP response, recording
    /// the status so consumers can tell a retryable hiccup from a
    /// settled answer.
    pub fn http(status: u16, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Network,
            message: message.into(),
            status: Some(status),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `run_in_browser` is declared crate-globally in `ops.rs`; these
    // tests are pure logic and need no DOM of their own.

    /// The status rides along structurally so consumers classify on it
    /// rather than pattern-matching the message text.
    #[dialog_common::test]
    fn it_carries_the_status_it_was_built_with() {
        let err = ErrorDetail::http(404, "gone");
        assert_eq!(err.status, Some(404));
        assert_eq!(err.kind, ErrorKind::Network);
        assert_eq!(ErrorDetail::new(ErrorKind::Parse, "bad").status, None);
    }
}
