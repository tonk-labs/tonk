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

/// The copy that fills the shared branded email layout.
#[cfg(any(target_arch = "wasm32", test))]
struct BrandedEmail<'a> {
    /// Document title, shown by clients that surface it.
    title: &'a str,
    /// Hidden inbox preview line.
    preheader: &'a str,
    /// Lowercase card heading.
    heading: &'a str,
    /// Lowercase label of the card's one button.
    action: &'a str,
    /// Destination of the action button, escaped here.
    link: &'a str,
    /// The message in the card body.
    body: &'a str,
    /// Muted closing line under the card.
    footnote: &'a str,
}

/// Escape `value` for an HTML attribute or text node.
#[cfg(any(target_arch = "wasm32", test))]
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The email-client-safe Tonk layout: tables and inline styles only, the
/// hosted wordmark, then the account task card (heading, message, and one
/// button as its footer). Clients without `border-radius` square it off.
#[cfg(any(target_arch = "wasm32", test))]
fn branded_email_html(email: BrandedEmail<'_>) -> String {
    let BrandedEmail {
        title,
        preheader,
        heading,
        action,
        link,
        body,
        footnote,
    } = email;
    let link = escape_html(link);

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title}</title>
  </head>
  <body style="margin:0;background:#e8e6e4;color:#38182a;font-family:'Arial Narrow','Helvetica Neue',Arial,sans-serif;-webkit-font-smoothing:antialiased;">
    <div style="display:none;max-height:0;overflow:hidden;opacity:0;">{preheader}</div>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="width:100%;border-collapse:collapse;background:#e8e6e4;">
      <tr>
        <td align="center" style="padding:48px 16px 64px;">
          <table role="presentation" width="360" cellpadding="0" cellspacing="0" style="width:100%;max-width:360px;border-collapse:separate;border-spacing:0;">
            <tr>
              <td align="center" style="padding:0 0 48px;">
                <img src="https://tonk.network/images/tonk-wordmark.svg" width="132" height="44" alt="tonk" style="display:block;width:132px;max-width:100%;height:auto;border:0;outline:none;text-decoration:none;color:#38182a;font-size:20px;font-weight:800;line-height:44px;text-align:center;">
              </td>
            </tr>
            <tr>
              <td style="background:#efecee;border:1.5px solid #4f3546;border-radius:25px;overflow:hidden;">
                <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="width:100%;border-collapse:collapse;">
                  <tr>
                    <td style="padding:15px 18px 14px;">
                      <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="width:100%;border-collapse:collapse;">
                        <tr>
                          <td width="18" valign="middle" style="width:18px;"><div style="width:18px;height:18px;border-radius:50%;background:#38182a;font-size:0;line-height:0;">&nbsp;</div></td>
                          <td align="right" valign="middle" style="padding-left:16px;font-family:'IBM Plex Sans Condensed','Arial Narrow',Arial,sans-serif;font-size:17px;font-weight:700;line-height:1.1;color:#38182a;text-align:right;text-transform:lowercase;">{heading}</td>
                        </tr>
                      </table>
                    </td>
                  </tr>
                  <tr>
                    <td style="padding:12px 18px 24px;font-family:'IBM Plex Sans Condensed','Arial Narrow',Arial,sans-serif;font-size:18px;line-height:1.5;color:#38182a;">{body}</td>
                  </tr>
                  <tr>
                    <td style="background:#38182a;border-radius:0 0 24px 24px;">
                      <a href="{link}" style="display:block;box-sizing:border-box;min-height:48px;padding:15px 18px 14px;background:#38182a;border-radius:0 0 24px 24px;color:#f7f6f5;font-family:'IBM Plex Sans Condensed','Arial Narrow',Arial,sans-serif;font-size:17px;font-weight:700;line-height:1.1;text-align:right;text-decoration:none;text-transform:lowercase;">{action}</a>
                    </td>
                  </tr>
                </table>
              </td>
            </tr>
            <tr>
              <td style="padding:18px 18px 0;font-family:'Helvetica Neue',Arial,sans-serif;font-size:13px;line-height:1.5;color:#5b4953;">{footnote}</td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>"#
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn activation_email_html(link: &str) -> String {
    branded_email_html(BrandedEmail {
        title: "Activate your tonk account",
        preheader: "Confirm your email address to activate your account.",
        heading: "activate your account",
        action: "verify email",
        link,
        body: "Confirm your email address to activate your account. This allows Tonk to sync your data across browsers and devices, meaning other people and tools can connect to it.",
        footnote: "If you did not request this, you can safely ignore this email.",
    })
}

/// Subject line of the space invitation email.
#[cfg(any(target_arch = "wasm32", test))]
const INVITATION_SUBJECT: &str = "You’re invited to a Tonk space";

/// The invitation message, shared by the HTML and plain-text parts.
#[cfg(any(target_arch = "wasm32", test))]
const INVITATION_BODY: &str = "Someone invited this email address to a shared space on Tonk. Open the invitation, then create an account or sign in with this email address. Once you’ve confirmed it, you can join the space.";

/// The invitation's closing line, shared by the HTML and plain-text parts.
#[cfg(any(target_arch = "wasm32", test))]
const INVITATION_FOOTNOTE: &str = "This invitation expires in seven days and only works for this email address. If you weren’t expecting it, you can ignore this email.";

#[cfg(any(target_arch = "wasm32", test))]
fn invitation_email_html(link: &str) -> String {
    branded_email_html(BrandedEmail {
        title: INVITATION_SUBJECT,
        preheader: "Create an account with this email address, confirm it, then join the space.",
        heading: "you’re invited to a space",
        action: "open invitation",
        link,
        body: INVITATION_BODY,
        footnote: INVITATION_FOOTNOTE,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn invitation_email_text(link: &str) -> String {
    format!(
        "You’re invited to a space on Tonk\n\n{INVITATION_BODY}\n\nOpen the invitation:\n{link}\n\n{INVITATION_FOOTNOTE}"
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
                    "Confirm your email address to activate your account. This allows Tonk to sync your data across browsers and devices, meaning other people and tools can connect to it.\n\n{link}\n\nIf you did not request this, ignore this message."
                ),
                &activation_email_html(link),
            )
            .await
            .map_err(|err| EmailError::Send(err.to_string()))
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl crate::invitation::InvitationEmailSender for Resend {
    async fn send_invitation(
        &self,
        email: &str,
        link: &str,
        id: &str,
    ) -> Result<(), crate::invitation::InvitationError> {
        self.0
            .send_html_idempotent(
                email,
                INVITATION_SUBJECT,
                &invitation_email_text(link),
                &invitation_email_html(link),
                &format!("tonk-email-invitation-{id}"),
            )
            .await
            .map_err(|_| crate::invitation::InvitationError::Unavailable)
    }
}

#[cfg(any(test, feature = "helpers"))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl crate::invitation::InvitationEmailSender for CapturedEmail {
    async fn send_invitation(
        &self,
        email: &str,
        link: &str,
        _id: &str,
    ) -> Result<(), crate::invitation::InvitationError> {
        let mut messages = self.0.lock().expect("captured email mutex");
        let message = (email.to_owned(), link.to_owned());
        if !messages.contains(&message) {
            messages.push(message);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{activation_email_html, invitation_email_html, invitation_email_text};

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

    #[test]
    fn invitation_email_uses_the_branded_layout_with_one_button() {
        let link = format!("https://tonk.network/email-invite#{}", "a".repeat(64));
        let html = invitation_email_html(&link);

        assert!(html.contains("background:#e8e6e4"));
        assert!(html.contains(
            r#"<img src="https://tonk.network/images/tonk-wordmark.svg" width="132" height="44" alt="tonk""#
        ));
        assert!(html.contains(">you’re invited to a space</td>"));
        assert!(html.contains(&format!(r#"href="{link}" style="display:block"#)));
        assert!(html.contains(">open invitation</a>"));
        assert_eq!(html.matches("<a ").count(), 1);
        assert!(html.contains("expires in seven days"));
    }

    #[test]
    fn invitation_email_escapes_the_link_for_an_html_attribute() {
        let html = invitation_email_html("https://tonk.network/email-invite?a=1&b=\"<x>\"#id");

        assert!(html.contains(
            r#"href="https://tonk.network/email-invite?a=1&amp;b=&quot;&lt;x&gt;&quot;#id""#
        ));
    }

    #[test]
    fn invitation_email_has_a_plain_text_alternative_with_the_same_copy() {
        let link = format!("https://tonk.network/email-invite#{}", "a".repeat(64));
        let text = invitation_email_text(&link);

        assert!(text.contains(&format!("\n{link}\n")));
        assert!(text.contains("create an account or sign in with this email address"));
        assert!(text.contains("expires in seven days"));
        assert!(!text.contains('<'));
    }
}
