//! Typed refusal responses.
//!
//! The reason a request was refused travels as itself: the body of a
//! non-2xx answer is the serde-tagged [`AuthorizeError`] or
//! [`Rejection`] built while deciding, exactly the value
//! `dialog-remote-ucan-s3`'s client reads back out. There is no code
//! table on either side — adding a reason upstream does not mean
//! teaching two codebases a new string.

use dialog_capability::access::AuthorizeError;
use dialog_effects::Rejection;
use worker::Response;

/// A refused request: the typed reason plus nothing else.
#[derive(Debug)]
pub enum Refusal {
    /// The request was understood and the answer is no — or no
    /// decision could be reached about the caller's input.
    Authorization(AuthorizeError),
    /// The request was not carried out for a reason that is not an
    /// access decision (our own machinery, an unrecognized failure).
    Rejection(Rejection),
}

impl From<AuthorizeError> for Refusal {
    fn from(reason: AuthorizeError) -> Self {
        Self::Authorization(reason)
    }
}

impl From<Rejection> for Refusal {
    fn from(rejection: Rejection) -> Self {
        Self::Rejection(rejection)
    }
}

impl Refusal {
    /// A refusal for a failure of our own machinery that is not worth
    /// retrying as-is.
    /// The refusal's stable kind tag, as it appears on the wire.
    pub fn kind(&self) -> String {
        let value = match self {
            Refusal::Authorization(reason) => serde_json::to_value(reason),
            Refusal::Rejection(rejection) => serde_json::to_value(rejection),
        };
        value
            .ok()
            .and_then(|value| value["kind"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Unclassified".to_string())
    }

    pub fn unclassified(detail: impl Into<String>) -> Self {
        Self::Rejection(Rejection::Unclassified {
            detail: detail.into(),
        })
    }

    /// The HTTP status this refusal answers with. Advisory — the
    /// client classifies by parsing the body, not the status — but
    /// kept honest for logs, proxies, and anything else that only
    /// sees the status line.
    pub fn status(&self) -> u16 {
        match self {
            Self::Authorization(reason) => match reason {
                AuthorizeError::InvalidSignature { .. }
                | AuthorizeError::InvalidAudience { .. }
                | AuthorizeError::Expired { .. }
                | AuthorizeError::NotValidBefore { .. } => 401,
                AuthorizeError::UnprovenSubject { .. }
                | AuthorizeError::CommandEscalation { .. }
                | AuthorizeError::PolicyViolation { .. }
                | AuthorizeError::Declined { .. }
                | AuthorizeError::Revoked { .. } => 403,
                AuthorizeError::Malformed { .. } | AuthorizeError::UnavailableProof { .. } => 400,
                AuthorizeError::Unavailable { .. } => 503,
            },
            Self::Rejection(rejection) => {
                if rejection.is_transient() {
                    503
                } else {
                    500
                }
            }
        }
    }

    /// Convert to a worker [`Response`]: the JSON-encoded reason under
    /// the matching status.
    pub fn to_response(&self) -> worker::Result<Response> {
        let response = match self {
            Self::Authorization(reason) => Response::from_json(reason),
            Self::Rejection(rejection) => Response::from_json(rejection),
        }?;
        Ok(response.with_status(self.status()))
    }
}
