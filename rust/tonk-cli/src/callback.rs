//! A loopback callback server for browser authorization ceremonies.
//!
//! The CLI cannot run a passkey ceremony — that needs a browser — but it can
//! receive the delegation one produces. It binds a short-lived server on
//! loopback, hands the browser a `callback=` URL pointing at it, and waits for
//! the authorizing page to deliver the result directly. No remote service sits
//! in the middle, so nothing but the two local processes ever sees the grant.
//!
//! The page delivers by **form POST rather than `fetch`**, which sidesteps
//! CORS entirely: a cross-origin form submission needs no preflight and no
//! permissive `Access-Control-Allow-Origin` on a server that exists for one
//! request. It also lets the server answer with a page the user can read.
//!
//! This is the transport only. What the delegation must satisfy — audience,
//! subject, expiry — belongs to the caller, which knows what it asked for.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::{Form, State};
use axum::response::Html;
use axum::routing::post;
use base64::Engine as _;
use serde::Deserialize;
use tokio::sync::{Notify, oneshot};

/// How long to wait for the browser before giving up.
const DEADLINE: Duration = Duration::from_secs(300);

/// What the authorizing page sent back.
pub enum Authorization {
    /// The user approved: the raw delegation bytes the page delivered.
    Granted(Vec<u8>),
    /// The user declined, with whatever reason the page supplied.
    Denied(String),
}

#[derive(Deserialize)]
struct CallbackForm {
    #[serde(default)]
    authorize: Option<String>,
    #[serde(default)]
    deny: Option<String>,
}

#[derive(Clone)]
struct Waiting {
    shutdown: Arc<Notify>,
    sender: Arc<std::sync::Mutex<Option<oneshot::Sender<Authorization>>>>,
}

/// A bound loopback listener waiting for one authorization.
pub struct Callback {
    url: String,
    listener: tokio::net::TcpListener,
}

impl Callback {
    /// Bind a loopback listener on an ephemeral port.
    ///
    /// Port 0 lets the OS choose, so two `tonk` processes authorizing at once
    /// cannot collide and no scan is needed to find a free port.
    pub async fn bind() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .context("failed to bind the authorization callback listener")?;
        let port = listener
            .local_addr()
            .context("failed to read the callback listener address")?
            .port();
        Ok(Self {
            url: format!("http://127.0.0.1:{port}"),
            listener,
        })
    }

    /// The URL to hand the authorizing page as its `callback=`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Serve exactly one authorization, then shut down.
    ///
    /// Times out after five minutes: a browser tab the user abandoned should
    /// not leave a listener bound for the life of the shell.
    pub async fn receive(self) -> Result<Authorization> {
        let (sender, receiver) = oneshot::channel();
        let shutdown = Arc::new(Notify::new());
        let state = Waiting {
            shutdown: shutdown.clone(),
            sender: Arc::new(std::sync::Mutex::new(Some(sender))),
        };
        let app = Router::new().route("/", post(deliver)).with_state(state);
        let server = axum::serve(self.listener, app).with_graceful_shutdown(async move {
            shutdown.notified().await;
        });

        match tokio::time::timeout(DEADLINE, async {
            let served = server.await;
            (served, receiver.await)
        })
        .await
        {
            Ok((Ok(()), Ok(authorization))) => Ok(authorization),
            Ok((Ok(()), Err(_))) => bail!("the authorization page closed without answering"),
            Ok((Err(error), _)) => Err(error).context("the authorization callback server failed"),
            Err(_) => bail!("timed out waiting for authorization in the browser"),
        }
    }
}

/// The page the browser lands on once the terminal has its answer.
///
/// A redirect back to the account page would be worse: the tab's purpose is
/// finished, and sending the user somewhere else leaves them guessing whether
/// it worked. This says what happened and offers to close, which browsers
/// permit for a window that was scripted open and ignore otherwise — so the
/// message stands on its own either way.
fn confirmation(message: &str) -> String {
    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>Tonk</title>
<style>
  body {{ font: 16px/1.5 system-ui, sans-serif; margin: 0;
         min-height: 100vh; display: grid; place-items: center; }}
  main {{ text-align: center; padding: 2rem; }}
  button {{ font: inherit; margin-top: 1rem; padding: 0.5rem 1rem; }}
</style>
<main>
  <p>{message}</p>
  <button onclick="window.close()">Close this window</button>
</main>
"#
    )
}

/// Receive the page's POST, hand the result to the waiter, and stop.
async fn deliver(State(state): State<Waiting>, Form(form): Form<CallbackForm>) -> Html<String> {
    let (outcome, page) = match (form.authorize, form.deny) {
        (Some(encoded), _) => match base64::engine::general_purpose::STANDARD.decode(&encoded) {
            Ok(bytes) => (
                Authorization::Granted(bytes),
                "Authorized. You can return to your terminal.",
            ),
            // A malformed body is reported as a denial rather than retried:
            // the page has already run its ceremony, so there is nothing to
            // wait for, and the caller learns why instead of timing out.
            Err(error) => (
                Authorization::Denied(format!("authorization was not valid base64: {error}")),
                "Could not read the authorization — check your terminal.",
            ),
        },
        (None, Some(reason)) => (Authorization::Denied(reason), "Authorization declined."),
        (None, None) => (
            Authorization::Denied("the page sent no authorization".to_owned()),
            "Nothing was authorized.",
        ),
    };

    if let Ok(mut slot) = state.sender.lock()
        && let Some(sender) = slot.take()
    {
        let _ = sender.send(outcome);
    }
    state.shutdown.notify_one();
    Html(confirmation(page))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page delivers a grant by form POST and the CLI receives it.
    ///
    /// Over a real socket, because the point of this module is the transport:
    /// a mocked handler would prove the match arms and none of the binding,
    /// serving, or shutdown.
    #[tokio::test]
    async fn it_receives_a_granted_authorization() {
        let callback = Callback::bind().await.unwrap();
        let url = callback.url().to_owned();

        let posting = tokio::spawn(async move {
            let body = base64::engine::general_purpose::STANDARD.encode(b"delegation-bytes");
            reqwest::Client::new()
                .post(&url)
                .form(&[("authorize", body)])
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        });

        let authorization = callback.receive().await.unwrap();
        posting.await.unwrap();
        match authorization {
            Authorization::Granted(bytes) => assert_eq!(bytes, b"delegation-bytes"),
            Authorization::Denied(reason) => panic!("expected a grant, got denial: {reason}"),
        }
    }

    /// A declined ceremony reports the reason rather than timing out — the
    /// user already answered, so waiting out the deadline would be wrong.
    #[tokio::test]
    async fn it_reports_a_denial() {
        let callback = Callback::bind().await.unwrap();
        let url = callback.url().to_owned();

        let posting = tokio::spawn(async move {
            reqwest::Client::new()
                .post(&url)
                .form(&[("deny", "user declined")])
                .send()
                .await
                .unwrap();
        });

        let authorization = callback.receive().await.unwrap();
        posting.await.unwrap();
        match authorization {
            Authorization::Denied(reason) => assert!(reason.contains("declined")),
            Authorization::Granted(_) => panic!("a denial must not read as a grant"),
        }
    }

    /// A body that is not base64 is reported, not silently dropped: the page
    /// has already run its ceremony, so there is nothing left to wait for.
    #[tokio::test]
    async fn it_reports_an_unreadable_authorization() {
        let callback = Callback::bind().await.unwrap();
        let url = callback.url().to_owned();

        let posting = tokio::spawn(async move {
            reqwest::Client::new()
                .post(&url)
                .form(&[("authorize", "not-base64-!!")])
                .send()
                .await
                .unwrap();
        });

        let authorization = callback.receive().await.unwrap();
        posting.await.unwrap();
        match authorization {
            Authorization::Denied(reason) => assert!(reason.contains("base64")),
            Authorization::Granted(_) => panic!("unreadable bytes must not read as a grant"),
        }
    }
}
