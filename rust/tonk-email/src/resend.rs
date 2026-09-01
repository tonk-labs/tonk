//! Resend-backed transport, for production use.

use serde::Serialize;
use worker::wasm_bindgen::JsValue;
use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::EmailError;

/// A Resend client that sends text and HTML messages.
pub struct Resend {
    /// Resend API key, sent as a bearer token.
    api_key: String,
    /// The address messages are sent from.
    from: String,
}

/// The JSON body of a Resend `POST /emails` request.
#[derive(Serialize)]
struct SendRequest<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
}

impl Resend {
    /// Construct a sender that authenticates with `api_key` and sends
    /// from `from`.
    pub fn new(api_key: String, from: String) -> Self {
        Self { api_key, from }
    }

    /// Send a plain-text message.
    pub async fn send(&self, to: &str, subject: &str, text: &str) -> Result<(), EmailError> {
        self.send_message(to, subject, Some(text), None).await
    }

    /// Send an HTML message with a plain-text fallback.
    pub async fn send_html(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: &str,
    ) -> Result<(), EmailError> {
        self.send_message(to, subject, Some(text), Some(html)).await
    }

    async fn send_message(
        &self,
        to: &str,
        subject: &str,
        text: Option<&str>,
        html: Option<&str>,
    ) -> Result<(), EmailError> {
        let body = SendRequest {
            from: &self.from,
            to: [to],
            subject,
            text,
            html,
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
