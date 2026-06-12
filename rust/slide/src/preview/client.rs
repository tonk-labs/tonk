//! One-shot client for `slide preview render`: posts a
//! [`RenderRequest`] to the running daemon and decodes the reply.

use crate::preview::protocol::{CAPABILITY_RENDER_PREVIEW, RenderReply, RenderRequest};

/// Why a render round-trip failed, with agent-actionable messages.
#[derive(Debug, thiserror::Error)]
pub enum RenderClientError {
    /// The daemon socket did not answer at all.
    #[error(
        "could not reach the preview daemon at {url} — is `slide preview serve` running? ({source})"
    )]
    DaemonUnreachable {
        /// The URL that was tried.
        url: String,
        /// Underlying transport error.
        source: reqwest::Error,
    },
    /// The daemon answered with a non-success status (e.g. 503 when
    /// no browser page is connected).
    #[error("daemon rejected the request ({status}): {message}")]
    Rejected {
        /// HTTP status code.
        status: u16,
        /// Daemon's error body.
        message: String,
    },
    /// The reply did not decode as a [`RenderReply`].
    #[error("malformed daemon reply: {0}")]
    MalformedReply(reqwest::Error),
}

/// Post `request` to the daemon on `port` and await the rendered
/// HTML.
pub async fn request_render(
    port: u16,
    request: &RenderRequest,
) -> Result<RenderReply, RenderClientError> {
    let url = format!("http://127.0.0.1:{port}/capability/{CAPABILITY_RENDER_PREVIEW}");
    let response = reqwest::Client::new()
        .post(&url)
        .json(request)
        .send()
        .await
        .map_err(|source| RenderClientError::DaemonUnreachable {
            url: url.clone(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        return Err(RenderClientError::Rejected {
            status: status.as_u16(),
            message,
        });
    }
    response
        .json()
        .await
        .map_err(RenderClientError::MalformedReply)
}
