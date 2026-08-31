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

#[cfg(any(target_arch = "wasm32", test))]
fn activation_email_html(link: &str) -> String {
    let link = link
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Activate your tonk account</title>
  </head>
  <body style="margin:0;background:#e8e6e4;color:#38182a;font-family:'Arial Narrow','Helvetica Neue',Arial,sans-serif;-webkit-font-smoothing:antialiased;">
    <div style="display:none;max-height:0;overflow:hidden;opacity:0;">Verify your email address to activate syncing for your tonk account.</div>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="width:100%;border-collapse:collapse;background:#e8e6e4;">
      <tr>
        <td align="center" style="padding:48px 16px 64px;">
          <table role="presentation" width="432" cellpadding="0" cellspacing="0" style="width:100%;max-width:432px;border-collapse:separate;border-spacing:0;">
            <tr>
              <td align="center" style="padding:0 0 40px;">
                <img src="https://tonk.network/images/tonk-wordmark.svg" width="132" height="44" alt="tonk" style="display:block;width:132px;max-width:100%;height:auto;border:0;outline:none;text-decoration:none;color:#38182a;font-size:20px;font-weight:800;line-height:44px;text-align:center;">
              </td>
            </tr>
            <tr>
              <td style="padding:15px 16px 10px;background:#f7f6f5;border:1px solid #38182a;font-size:13px;font-weight:700;line-height:1;letter-spacing:.02em;text-transform:lowercase;">activate your account</td>
            </tr>
            <tr>
              <td style="padding-top:7px;">
                <a href="{link}" style="display:block;box-sizing:border-box;min-height:44px;padding:15px 24px 13px;background:#38182a;color:#f7f6f5;font-size:13px;font-weight:700;line-height:16px;letter-spacing:.02em;text-align:right;text-decoration:none;text-transform:lowercase;">verify email</a>
              </td>
            </tr>
            <tr>
              <td style="padding-top:7px;">
                <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="width:100%;border-collapse:collapse;background:#f7f6f5;">
                  <tr>
                    <td style="padding:14px 16px 15px;font-family:'Helvetica Neue',Arial,sans-serif;font-size:13px;line-height:1.5;color:#38182a;">Confirm your email address to activate syncing for your tonk account. The button opens Tonk so you can review and complete activation.</td>
                  </tr>
                </table>
              </td>
            </tr>
            <tr>
              <td style="padding:18px 16px 0;font-family:'Helvetica Neue',Arial,sans-serif;font-size:12px;line-height:1.5;color:#5b4953;">If you did not request this, you can safely ignore this email.</td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#
    )
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl EmailSender for Resend {
    async fn send_activation(&self, email: &str, link: &str) -> Result<(), EmailError> {
        self.0
            .send_html(
                email,
                "Activate your tonk account",
                &format!(
                    "Confirm your email address and accept the terms of service to activate your tonk account:\n\n{link}\n\nIf you did not request this, ignore this message."
                ),
                &activation_email_html(link),
            )
            .await
            .map_err(|err| EmailError::Send(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::activation_email_html;

    #[test]
    fn activation_email_uses_a_styled_verification_button() {
        let html = activation_email_html("https://tonk.network/activate?token=abc");

        assert!(html.contains("background:#e8e6e4"));
        assert!(html.contains("background:#38182a"));
        assert!(html.contains(
            r#"<img src="https://tonk.network/images/tonk-wordmark.svg" width="132" height="44" alt="tonk""#
        ));
        assert!(
            html.contains(r#"href="https://tonk.network/activate?token=abc" style="display:block"#)
        );
        assert!(html.contains(">verify email</a>"));
    }

    #[test]
    fn activation_email_escapes_the_link_for_an_html_attribute() {
        let html = activation_email_html(
            "https://tonk.network/activate?one=1&two=\"quoted\"&three=<value>",
        );

        assert!(html.contains(
            r#"href="https://tonk.network/activate?one=1&amp;two=&quot;quoted&quot;&amp;three=&lt;value&gt;""#
        ));
        assert!(!html.contains(r#"href="https://tonk.network/activate?one=1&two="#));
    }
}
