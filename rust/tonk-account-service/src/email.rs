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

/// Delivery backend for account mail.
#[allow(async_fn_in_trait)]
pub trait EmailSender {
    /// Send `code` to `email`.
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError>;

    /// Tell an account holder that `credential` was enrolled.
    ///
    /// An anchor chain outlives revocation of whatever enrolled it, so the
    /// person has to hear about one being issued even when the ceremony
    /// succeeded exactly as intended. This is the loud half of the gate; the
    /// code is the second factor.
    async fn send_enrollment_notice(&self, email: &str, credential: &str)
    -> Result<(), EmailError>;
}

/// An [`EmailSender`] that records every send instead of delivering it,
/// for tests and local development.
#[cfg(any(test, feature = "helpers"))]
#[derive(Default)]
pub struct CapturedEmail {
    /// `(email, code)` for every verification code not sent.
    pub codes: std::sync::Mutex<Vec<(String, String)>>,
    /// `(email, credential DID)` for every enrollment notice not sent.
    pub notices: std::sync::Mutex<Vec<(String, String)>>,
}

#[cfg(any(test, feature = "helpers"))]
impl EmailSender for CapturedEmail {
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError> {
        self.codes
            .lock()
            .expect("captured email mutex poisoned")
            .push((email.to_string(), code.to_string()));
        Ok(())
    }

    async fn send_enrollment_notice(
        &self,
        email: &str,
        credential: &str,
    ) -> Result<(), EmailError> {
        self.notices
            .lock()
            .expect("captured email mutex poisoned")
            .push((email.to_string(), credential.to_string()));
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
pub mod resend;
