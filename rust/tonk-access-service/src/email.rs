//! Email delivery for customer registration.
//!
//! [`EmailSender`] mirrors [`Store`](crate::store::Store): declared
//! through the dual `async_trait` forms so callers are generic over the
//! trait, never `dyn EmailSender`. The transport is the shared
//! [`tonk_email`] Resend client.

use async_trait::async_trait;

/// Errors surfaced by an [`EmailSender`] implementation.
#[derive(Debug)]
pub enum EmailError {
    /// The underlying transport failed to send the message.
    Send(String),
}

/// Delivery backend for activation links.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait EmailSender {
    /// Send an activation link to `email`.
    async fn send_activation(&self, email: &str, link: &str) -> Result<(), EmailError>;
}

/// An [`EmailSender`] that records every send instead of delivering it,
/// for tests and local development. Holds `(email, link)` pairs.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct CapturedEmail(pub std::sync::Mutex<Vec<(String, String)>>);

#[cfg(any(test, feature = "helpers"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl EmailSender for CapturedEmail {
    async fn send_activation(&self, email: &str, link: &str) -> Result<(), EmailError> {
        self.0
            .lock()
            .expect("captured email mutex poisoned")
            .push((email.to_string(), link.to_string()));
        Ok(())
    }
}

/// Resend-backed [`EmailSender`], for production use.
#[cfg(target_arch = "wasm32")]
pub struct Resend(tonk_email::Resend);

#[cfg(target_arch = "wasm32")]
impl Resend {
    /// Construct a sender that authenticates with `api_key` and sends
    /// from `from`.
    pub fn new(api_key: String, from: String) -> Self {
        Self(tonk_email::Resend::new(api_key, from))
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl EmailSender for Resend {
    async fn send_activation(&self, email: &str, link: &str) -> Result<(), EmailError> {
        self.0
            .send(
                email,
                "Activate your tonk account",
                &format!(
                    "Confirm your email address and accept the terms of service to activate your tonk account:\n\n{link}\n\nIf you did not request this, ignore this message."
                ),
            )
            .await
            .map_err(|err| EmailError::Send(err.to_string()))
    }
}
