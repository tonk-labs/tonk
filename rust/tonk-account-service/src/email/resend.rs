//! Resend-backed [`EmailSender`](crate::email::EmailSender), for
//! production use.

use serde::Serialize;
use worker::wasm_bindgen::JsValue;
use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::email::{EmailError, EmailSender};

/// Resend-backed [`EmailSender`], for production use.
pub struct Resend {
    /// Resend API key, sent as a bearer token.
    api_key: String,
    /// The address verification codes are sent from.
    from: String,
}

impl Resend {
    /// Construct a sender that authenticates with `api_key` and sends
    /// from `from`.
    pub fn new(api_key: String, from: String) -> Self {
        Self { api_key, from }
    }
}

/// The JSON body of a Resend `POST /emails` request.
#[derive(Serialize)]
struct SendRequest<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    text: String,
}

impl Resend {
    async fn send(&self, email: &str, subject: &str, text: String) -> Result<(), EmailError> {
        let body = SendRequest {
            from: &self.from,
            to: [email],
            subject,
            text,
        };
        let body_json =
            serde_json::to_string(&body).map_err(|err| EmailError::Send(err.to_string()))?;

        let headers = Headers::new();
        headers
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .map_err(|err| EmailError::Send(err.to_string()))?;
        headers
            .set("Content-Type", "application/json")
            .map_err(|err| EmailError::Send(err.to_string()))?;

        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(JsValue::from_str(&body_json)));

        let request = Request::new_with_init("https://api.resend.com/emails", &init)
            .map_err(|err| EmailError::Send(err.to_string()))?;

        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|err| EmailError::Send(err.to_string()))?;

        let status = response.status_code();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(EmailError::Send(format!(
                "resend returned {status}: {text}"
            )));
        }
        Ok(())
    }
}

impl EmailSender for Resend {
    async fn send_code(&self, email: &str, code: &str) -> Result<(), EmailError> {
        self.send(
            email,
            "Your tonk verification code",
            format!("Your code is {code}. It expires in 10 minutes."),
        )
        .await
    }

    async fn send_enrollment_notice(
        &self,
        email: &str,
        credential: &str,
    ) -> Result<(), EmailError> {
        self.send(
            email,
            "A new passkey was added to your tonk account",
            format!(
                "A passkey was enrolled on your tonk account.\n\n\
                 Credential: {credential}\n\n\
                 If this was not you, sign in from a device you still trust \
                 and revoke it. Anyone holding this passkey can act as you."
            ),
        )
        .await
    }
}
