//! Email delivery for the account service.
//!
//! [`EmailSender`] is a plain-`async fn` trait, mirroring
//! [`Store`](crate::store::Store): callers are generic over the trait,
//! never `dyn EmailSender`.

/// Errors surfaced by an [`EmailSender`] implementation.
#[derive(Debug)]
pub enum EmailError {
    /// The underlying transport failed to send the message.
    Send(String),
}

/// Delivery backend for one-time verification codes.
#[allow(async_fn_in_trait)]
pub trait EmailSender {
    /// Send `code` to `email`.
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError>;
}

/// An [`EmailSender`] that records every send instead of delivering it,
/// for tests and local development.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct CapturedEmail(pub std::sync::Mutex<Vec<(String, String)>>);

#[cfg(any(test, feature = "helpers"))]
impl EmailSender for CapturedEmail {
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError> {
        self.0
            .lock()
            .expect("captured email mutex poisoned")
            .push((email.to_string(), code.to_string()));
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
pub mod resend;
