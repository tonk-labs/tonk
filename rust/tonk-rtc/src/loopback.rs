//! Same-machine signalling: a loopback listener the browser navigates to.
//!
//! This is deliberately the same shape as the CLI's account-authorization
//! callback (`tonk-cli/src/callback.rs`): bind an ephemeral loopback port,
//! hand the browser a `callback=` URL, let the page navigate back with the
//! payload in the **fragment**, and have a tiny bridge page re-submit it as
//! a same-origin POST.
//!
//! The indirection buys two specific things, and it is worth being clear
//! about which:
//!
//! - **The page never fetches loopback.** An `https://` page issuing a
//!   `fetch` to `http://127.0.0.1` is a cross-origin request to a local
//!   network address, which means CORS *and* Chrome's Local Network Access
//!   gate. A navigation is subject to neither.
//! - **The SDP never reaches a server log.** Fragments are not sent over
//!   the network. The bridge page reads it locally, strips it from history,
//!   and POSTs the value same-origin.
//!
//! # What this does not do
//!
//! It only works when the browser and the CLI are on the same machine.
//! That is the whole premise of loopback, and no amount of care here
//! changes it. When the peers move apart, this type gets replaced by a
//! relay-backed implementation — which is why the CLI depends on the
//! `receive`/`url` pair below and not on anything inside it.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Form, State};
use axum::response::Html;
use axum::routing::get;
use serde::Deserialize;
use tokio::sync::{Notify, oneshot};

/// How long to wait for the browser before giving up.
///
/// Long enough to open a tab and click through, short enough that an
/// abandoned ceremony does not leave a port bound for the life of the
/// shell.
const DEADLINE: Duration = Duration::from_secs(300);

/// Why a signalling exchange did not complete.
#[derive(Debug, thiserror::Error)]
pub enum SignalError {
    /// The loopback listener could not bind.
    #[error("could not bind the signalling listener: {0}")]
    Bind(String),
    /// The browser connected but never delivered an answer.
    #[error("the browser closed without answering")]
    Closed,
    /// The listener itself failed.
    #[error("the signalling listener failed: {0}")]
    Server(String),
    /// Nobody answered in time.
    #[error("timed out waiting for the browser to answer")]
    Timeout,
}

/// A bound loopback listener waiting for exactly one answer.
pub struct Loopback {
    url: String,
    listener: tokio::net::TcpListener,
}

#[derive(Deserialize)]
struct Delivery {
    #[serde(default)]
    answer: Option<String>,
}

#[derive(Clone)]
struct Waiting {
    shutdown: Arc<Notify>,
    sender: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

impl Loopback {
    /// Bind on an ephemeral loopback port.
    ///
    /// Port 0 so two `tonk` processes negotiating at once cannot collide
    /// and no free-port scan is needed.
    pub async fn bind() -> Result<Self, SignalError> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .map_err(|error| SignalError::Bind(error.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|error| SignalError::Bind(error.to_string()))?
            .port();
        Ok(Self {
            url: format!("http://127.0.0.1:{port}"),
            listener,
        })
    }

    /// The URL to hand the browser page as its `callback=`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Serve one answer, then shut down.
    pub async fn receive(self) -> Result<String, SignalError> {
        let (sender, receiver) = oneshot::channel();
        let shutdown = Arc::new(Notify::new());
        let state = Waiting {
            shutdown: shutdown.clone(),
            sender: Arc::new(Mutex::new(Some(sender))),
        };
        let app = Router::new()
            .route("/", get(bridge).post(deliver))
            .with_state(state);
        let server = axum::serve(self.listener, app).with_graceful_shutdown(async move {
            shutdown.notified().await;
        });

        let served = async {
            // `axum::serve(..).with_graceful_shutdown(..)` is
            // `IntoFuture`, not `Future`, so it cannot be pinned
            // directly; the async block gives `tokio::pin!` something
            // that is.
            let serving = async { server.await };
            tokio::pin!(serving);
            let mut receiver = receiver;
            tokio::select! {
                outcome = &mut receiver => {
                    // The browser may be holding another half-open
                    // connection; its graceful drain must not hold an
                    // answer we already have. Give the response a
                    // moment to flush, then move on.
                    let _ = tokio::time::timeout(Duration::from_secs(1), &mut serving).await;
                    (Ok(()), outcome)
                }
                result = &mut serving => (result, receiver.await),
            }
        };

        match tokio::time::timeout(DEADLINE, served).await {
            Ok((Ok(()), Ok(answer))) => Ok(answer),
            Ok((Ok(()), Err(_))) => Err(SignalError::Closed),
            Ok((Err(error), _)) => Err(SignalError::Server(error.to_string())),
            Err(_) => Err(SignalError::Timeout),
        }
    }
}

/// Land the browser's navigation, then re-submit its fragment on loopback.
///
/// A bare GET does not consume the exchange, so a prefetch or a stray
/// visit cannot kill a pending negotiation.
async fn bridge() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<meta charset="utf-8">
<meta name="referrer" content="no-referrer">
<title>tonk rtc</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0;
         min-height: 100vh; display: grid; place-items: center; }
  main { text-align: center; padding: 2rem; }
</style>
<main>
  <p id="status">returning the answer to tonk…</p>
  <noscript>JavaScript is required to return the answer to tonk.</noscript>
</main>
<script>
  const fields = new URLSearchParams(window.location.hash.slice(1));
  history.replaceState(null, "", window.location.pathname + window.location.search);
  if (!fields.has("answer")) {
    document.querySelector("#status").textContent =
      "no answer was provided. you can close this window.";
  } else {
    const form = document.createElement("form");
    form.method = "post";
    form.action = window.location.pathname + window.location.search;
    form.hidden = true;
    const input = document.createElement("input");
    input.type = "hidden";
    input.name = "answer";
    input.value = fields.get("answer");
    form.appendChild(input);
    document.body.appendChild(form);
    form.submit();
  }
</script>
"##,
    )
}

/// Accept the posted answer and release the listener.
async fn deliver(State(state): State<Waiting>, Form(delivery): Form<Delivery>) -> Html<String> {
    let Some(answer) = delivery.answer.filter(|value| !value.is_empty()) else {
        return Html(page("no answer was delivered."));
    };

    let taken = state
        .sender
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .map(|sender| sender.send(answer).is_ok())
        .unwrap_or(false);

    if taken {
        state.shutdown.notify_waiters();
        Html(page("connected. you can return to your terminal."))
    } else {
        // A second delivery, or a terminal that stopped waiting. Say so
        // rather than implying this one took effect.
        Html(page("this connection was already answered."))
    }
}

fn page(message: &str) -> String {
    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<meta name="color-scheme" content="light dark">
<title>tonk rtc</title>
<style>
  body {{ font: 16px/1.5 system-ui, sans-serif; margin: 0;
         min-height: 100vh; display: grid; place-items: center; }}
  main {{ text-align: center; padding: 2rem; }}
</style>
<main><p>{message}</p></main>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bridge must not consume the exchange on a GET, must carry
    /// the fragment into a POST rather than a fetch, and must name the
    /// field `Delivery` deserializes — three literals that have to
    /// agree and live in two languages.
    #[test]
    fn the_bridge_page_posts_a_form_and_never_fetches() {
        let Html(source) = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(bridge());
        assert!(source.contains(r#"form.method = "post""#));
        assert!(source.contains("history.replaceState"));
        assert!(
            source.contains(r#"input.name = "answer""#),
            "the bridge posts a field `Delivery` does not read"
        );
        assert!(
            !source.contains("fetch("),
            "the bridge must navigate, not fetch: a loopback fetch from an \
             https page hits CORS and Local Network Access"
        );
    }

    #[tokio::test]
    async fn binding_yields_a_loopback_url() {
        let loopback = Loopback::bind().await.unwrap();
        assert!(loopback.url().starts_with("http://127.0.0.1:"));
        assert_ne!(loopback.url(), "http://127.0.0.1:0");
    }

    /// Two concurrent ceremonies get different ports, so one terminal
    /// cannot steal another's answer.
    #[tokio::test]
    async fn concurrent_ceremonies_do_not_collide() {
        let first = Loopback::bind().await.unwrap();
        let second = Loopback::bind().await.unwrap();
        assert_ne!(first.url(), second.url());
    }

    /// The whole round trip, driven the way the browser drives it.
    #[tokio::test]
    async fn an_answer_posted_to_the_listener_reaches_the_caller() {
        let loopback = Loopback::bind().await.unwrap();
        let url = loopback.url().to_owned();
        let waiting = tokio::spawn(loopback.receive());

        // Give the listener a moment to start serving.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream = tokio::net::TcpStream::connect(url.trim_start_matches("http://"))
            .await
            .unwrap();
        let body = "answer=an-encoded-answer".to_owned();
        let request = format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Content-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        use tokio::io::AsyncWriteExt as _;
        let mut stream = stream;
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        assert_eq!(waiting.await.unwrap().unwrap(), "an-encoded-answer");
    }
}
