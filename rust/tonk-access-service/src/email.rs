//! Email delivery for customer registration.
//!
//! [`EmailSender`] mirrors [`Store`](crate::store::Store): declared
//! through the dual `async_trait` forms so callers are generic over the
//! trait, never `dyn EmailSender`. The transport is the shared
//! [`tonk_email`] Resend client.

use async_trait::async_trait;

/// The stored spelling of an email address.
///
/// The address is a lookup key -- `did:web:{host}:customer:{domain}:{local}`
/// resolves one to a customer -- so the form written at enrollment has to
/// be the one a caller can reconstruct from the address they hold. Every
/// write and every lookup passes through here, so the two cannot drift.
///
/// This is the form the account service already stores (see its
/// `core::accounts` and `core::deletion`), so both databases agree on
/// what one address looks like.
///
/// Case folding is ASCII-only and the local part is folded along with
/// the domain. RFC 5321 makes the local part case-sensitive, but no mail
/// provider in practice treats it that way, and folding it is what makes
/// an address one key rather than several.
pub fn normalize_email(address: &str) -> String {
    address.trim().to_lowercase()
}

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
