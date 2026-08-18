//! Resend-backed [`EmailSender`](crate::email::EmailSender), for
//! production use.

use crate::email::{EmailError, EmailSender};

/// Resend-backed [`EmailSender`], for production use.
pub struct Resend(tonk_email::Resend);

impl Resend {
    /// Construct a sender that authenticates with `api_key` and sends
    /// from `from`.
    pub fn new(api_key: String, from: String) -> Self {
        Self(tonk_email::Resend::new(api_key, from))
    }
}

impl EmailSender for Resend {
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError> {
        self.0
            .send(
                email,
                "Your tonk verification code",
                &format!("Your code is {code}. It expires in 10 minutes."),
            )
            .await
            .map_err(|err| EmailError::Send(err.to_string()))
    }
}
