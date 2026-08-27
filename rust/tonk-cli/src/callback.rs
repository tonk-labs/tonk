//! A loopback callback server for browser authorization ceremonies.
//!
//! The CLI cannot run a passkey ceremony — that needs a browser — but it can
//! receive the delegation one produces. It binds a short-lived server on
//! loopback, hands the browser a `callback=` URL pointing at it, and waits for
//! the authorizing page to deliver the result directly. No remote service sits
//! in the middle, so nothing but the two local processes ever sees the grant.
//!
//! The page navigates here with a bodyless GET and carries the result in the
//! URL fragment, which browsers do not send over the network. A local bridge
//! page removes that fragment from history and submits it by same-origin POST.
//! Keeping the HTTPS-to-HTTP hop bodyless avoids browsers losing a cross-scheme
//! form body while showing an insecure-navigation warning; keeping the POST on
//! loopback avoids CORS entirely.
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
use axum::routing::get;
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
    /// Where to send the browser once the terminal has its answer, so the
    /// authorizing page reports the outcome in its own styling. Honored
    /// only for a URL on the page's own origin.
    #[serde(default)]
    redirect: Option<String>,
}

#[derive(Clone)]
struct Waiting {
    shutdown: Arc<Notify>,
    sender: Arc<std::sync::Mutex<Option<oneshot::Sender<Authorization>>>>,
    /// The only origin a `redirect` may name: the page this process opened.
    redirect_origin: Option<String>,
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
    /// `redirect_origin` is the origin of the page this process opened, the
    /// only place a delivered `redirect` may point back to.
    pub async fn receive(self, redirect_origin: Option<String>) -> Result<Authorization> {
        let (sender, receiver) = oneshot::channel();
        let shutdown = Arc::new(Notify::new());
        let state = Waiting {
            shutdown: shutdown.clone(),
            sender: Arc::new(std::sync::Mutex::new(Some(sender))),
            redirect_origin,
        };
        let app = Router::new()
            .route("/", get(bridge).post(deliver))
            .with_state(state);
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

/// Land a bodyless cross-scheme GET, then deliver its fragment on loopback.
///
/// Fragments never reach this server. The browser reads the fragment locally,
/// removes it from history before creating any DOM fields, and submits those
/// fields back to this same origin. A GET without an outcome does not consume
/// the callback, so a prefetch or accidental visit cannot deny the request.
async fn bridge() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<meta charset="utf-8">
<meta name="referrer" content="no-referrer">
<title>Tonk</title>
<style>
  body { font: 16px/1.5 system-ui, sans-serif; margin: 0;
         min-height: 100vh; display: grid; place-items: center; }
  main { text-align: center; padding: 2rem; }
</style>
<main>
  <p id="status">Returning authorization to Tonk…</p>
  <noscript>JavaScript is required to return authorization to Tonk.</noscript>
</main>
<script>
  const fields = new URLSearchParams(window.location.hash.slice(1));
  history.replaceState(null, "", window.location.pathname + window.location.search);
  if (!fields.has("authorize") && !fields.has("deny")) {
    document.querySelector("#status").textContent =
      "No authorization was provided. You can close this window.";
  } else {
    const form = document.createElement("form");
    form.method = "post";
    form.action = window.location.pathname + window.location.search;
    form.hidden = true;
    for (const [name, value] of fields) {
      const input = document.createElement("input");
      input.type = "hidden";
      input.name = name;
      input.value = value;
      form.appendChild(input);
    }
    document.body.appendChild(form);
    form.submit();
  }
</script>
"##,
    )
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

/// A page-styled landing for the finished exchange: `redirect` with the
/// outcome appended, when the page asked for one on its own origin.
fn redirect_back(
    state: &Waiting,
    redirect: Option<&str>,
    status: &str,
    message: Option<&str>,
) -> Option<String> {
    let allowed = state.redirect_origin.as_deref()?;
    let mut target: url::Url = redirect?.parse().ok()?;
    if target.origin().ascii_serialization() != allowed {
        return None;
    }
    target.query_pairs_mut().append_pair("link", status);
    if let Some(message) = message {
        target.query_pairs_mut().append_pair("message", message);
    }
    Some(target.to_string())
}

/// Receive the page's POST, hand the result to the waiter, and stop.
async fn deliver(
    State(state): State<Waiting>,
    Form(form): Form<CallbackForm>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (outcome, status, page) = match (form.authorize, form.deny) {
        (Some(encoded), _) => match base64::engine::general_purpose::STANDARD.decode(&encoded) {
            Ok(bytes) => (
                Authorization::Granted(bytes),
                ("ok", None),
                "Authorized. You can return to your terminal.",
            ),
            // A malformed body is reported as a denial rather than retried:
            // the page has already run its ceremony, so there is nothing to
            // wait for, and the caller learns why instead of timing out.
            Err(error) => (
                Authorization::Denied(format!("authorization was not valid base64: {error}")),
                ("invalid", Some("the authorization was not readable")),
                "Could not read the authorization — check your terminal.",
            ),
        },
        (None, Some(reason)) => (
            Authorization::Denied(reason),
            ("denied", Some("authorization was declined")),
            "Authorization declined.",
        ),
        (None, None) => (
            Authorization::Denied("the page sent no authorization".to_owned()),
            ("invalid", Some("nothing was authorized")),
            "Nothing was authorized.",
        ),
    };

    let landing = redirect_back(&state, form.redirect.as_deref(), status.0, status.1);
    if let Ok(mut slot) = state.sender.lock()
        && let Some(sender) = slot.take()
    {
        let _ = sender.send(outcome);
    }
    state.shutdown.notify_one();
    match landing {
        Some(target) => axum::response::Redirect::to(&target).into_response(),
        None => Html(confirmation(page)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crossing from HTTPS to loopback HTTP must not carry the grant in the
    /// request body: Safari can discard that POST while showing its insecure
    /// navigation warning. The first request therefore lands on a bridge page;
    /// the bridge keeps the listener alive for its same-origin POST.
    #[tokio::test]
    async fn it_bridges_a_fragment_grant_through_a_loopback_get() {
        let callback = Callback::bind().await.unwrap();
        let url = callback.url().to_owned();

        let posting = tokio::spawn(async move {
            let client = reqwest::Client::new();
            let response = client
                .get(&url)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            assert_eq!(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some("text/html; charset=utf-8")
            );
            let page = response.text().await.unwrap();
            assert!(
                page.contains("location.hash"),
                "bridge must read the fragment"
            );
            assert!(
                page.contains("history.replaceState"),
                "bridge must remove the grant from browser history"
            );
            assert!(
                page.contains("form.method = \"post\""),
                "bridge must deliver by same-origin POST"
            );

            let body = base64::engine::general_purpose::STANDARD.encode(b"fragment-grant");
            client
                .post(&url)
                .form(&[("authorize", body)])
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        });

        let authorization = callback.receive(None).await.unwrap();
        posting.await.unwrap();
        match authorization {
            Authorization::Granted(bytes) => assert_eq!(bytes, b"fragment-grant"),
            Authorization::Denied(reason) => panic!("expected a grant, got denial: {reason}"),
        }
    }

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

        let authorization = callback.receive(None).await.unwrap();
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

        let authorization = callback.receive(None).await.unwrap();
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

        let authorization = callback.receive(None).await.unwrap();
        posting.await.unwrap();
        match authorization {
            Authorization::Denied(reason) => assert!(reason.contains("base64")),
            Authorization::Granted(_) => panic!("unreadable bytes must not read as a grant"),
        }
    }
}
