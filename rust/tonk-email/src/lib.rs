//! Shared email transport for tonk worker services.
//!
//! Services keep their own delivery traits (what gets sent and when is
//! service policy); this crate holds the one transport they share, so a
//! second service does not grow a second Resend client.

/// Errors surfaced by the email transport.
#[derive(Debug)]
pub enum EmailError {
    /// The underlying transport failed to send the message.
    Send(String),
}

impl std::fmt::Display for EmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmailError::Send(detail) => write!(f, "email send failed: {detail}"),
        }
    }
}

impl std::error::Error for EmailError {}

#[cfg(target_arch = "wasm32")]
mod resend;
#[cfg(target_arch = "wasm32")]
pub use resend::Resend;
