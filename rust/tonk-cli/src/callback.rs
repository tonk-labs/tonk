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

use axum::Router;
use axum::extract::{Form, State};
use axum::response::Html;
use axum::routing::get;
use base64::Engine as _;
use serde::Deserialize;
use tokio::sync::{Notify, oneshot};

/// How long to wait for the browser before giving up.
const DEADLINE: Duration = Duration::from_secs(300);

/// Stable callback failure classification retained beside the local detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackFailureKind {
    /// The loopback listener could not bind or expose its address.
    Bind,
    /// The browser connection closed without an answer.
    Closed,
    /// The callback HTTP server failed.
    Server,
    /// No answer arrived before the deadline.
    Timeout,
}

/// A callback failure with closed telemetry evidence and local-only detail.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct CallbackFailure {
    kind: CallbackFailureKind,
    detail: String,
}

impl CallbackFailure {
    fn new(kind: CallbackFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }

    /// Stable callback failure kind.
    pub fn kind(&self) -> CallbackFailureKind {
        self.kind
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn callback_failure_keeps_kind_beside_local_detail() {
        for kind in [
            CallbackFailureKind::Bind,
            CallbackFailureKind::Closed,
            CallbackFailureKind::Server,
            CallbackFailureKind::Timeout,
        ] {
            let failure = CallbackFailure::new(kind, "local diagnostic with secret");
            assert_eq!(failure.kind(), kind);
            assert!(failure.to_string().contains("local diagnostic"));
        }
    }
}

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
    pub async fn bind() -> Result<Self, CallbackFailure> {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .map_err(|error| {
                CallbackFailure::new(
                    CallbackFailureKind::Bind,
                    format!("failed to bind the authorization callback listener: {error}"),
                )
            })?;
        let port = listener
            .local_addr()
            .map_err(|error| {
                CallbackFailure::new(
                    CallbackFailureKind::Bind,
                    format!("failed to read the callback listener address: {error}"),
                )
            })?
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
    pub async fn receive(
        self,
        redirect_origin: Option<String>,
    ) -> Result<Authorization, CallbackFailure> {
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
            Ok((Ok(()), Err(_))) => Err(CallbackFailure::new(
                CallbackFailureKind::Closed,
                "the authorization page closed without answering",
            )),
            Ok((Err(error), _)) => Err(CallbackFailure::new(
                CallbackFailureKind::Server,
                format!("the authorization callback server failed: {error}"),
            )),
            Err(_) => Err(CallbackFailure::new(
                CallbackFailureKind::Timeout,
                "timed out waiting for authorization in the browser",
            )),
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

/// The fallback page the browser lands on once the terminal has its answer.
///
/// The account page normally supplies a same-origin redirect and renders the
/// outcome itself. An older or custom page may not, so this standalone page
/// uses the same ceremony styling and offers to close. Browsers permit closing
/// a window that was scripted open and ignore the request otherwise, so the
/// message stands on its own either way.
fn confirmation(message: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>Tonk · Command-line access</title>
<style>
  :root {{
    color-scheme: light dark;
    --page: #ececec;
    --ink: #131313;
    --on-ink: #fbfaef;
    --soft: #55544f;
    --ring: rgb(19 19 19 / 85%);
    --frost-solid: #fafafa;
    --wash-p: rgb(251 250 239 / 16%);
    --cond: "IBM Plex Sans Condensed", "Bahnschrift", "Arial Narrow", sans-serif;
    --sans: "IBM Plex Sans", Helvetica, Arial, sans-serif;
  }}

  * {{ box-sizing: border-box; }}

  ::selection {{
    background: var(--ink);
    color: var(--on-ink);
  }}

  body {{
    display: grid;
    min-height: 100vh;
    min-height: 100dvh;
    margin: 0;
    padding: 48px 16px 80px;
    place-items: center;
    background: var(--page);
    color: var(--ink);
    font-family: var(--cond);
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }}

  .account {{
    width: min(576px, calc(100vw - 32px));
  }}

  .account__logo {{
    display: block;
    width: 132px;
    height: auto;
    margin: 0 auto clamp(36px, 6vh, 56px);
    color: var(--ink);
  }}

  .account__ceremony {{
    width: min(432px, 100%);
    margin-inline: auto;
  }}

  .account__stack {{
    display: flex;
    flex-direction: column;
    gap: 7px;
  }}

  .account__ceremony-head {{
    display: flex;
    height: 36px;
    margin: 0;
    padding: 0 16px 9px;
    align-items: flex-end;
    background: var(--frost-solid);
    box-shadow: 0 0 0 1px var(--ring);
    font-size: 13px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0.02em;
    text-transform: lowercase;
    text-wrap: balance;
  }}

  .account__run {{
    display: flex;
    min-height: 44px;
    margin: 0;
    padding: 0 10px 9px 24px;
    align-items: flex-end;
    justify-content: flex-end;
    border: 0;
    border-radius: 0;
    appearance: none;
    background: var(--ink);
    box-shadow: 0 0 0 1px var(--ink);
    color: var(--on-ink);
    font: 600 13px/1 var(--cond);
    letter-spacing: 0.02em;
    text-align: right;
    text-transform: lowercase;
    cursor: pointer;
    transition-property: scale, background-color;
    transition-duration: 150ms;
    transition-timing-function: ease-out;
  }}

  .account__run:hover {{
    background: linear-gradient(var(--wash-p), var(--wash-p)), var(--ink);
  }}

  .account__run:active {{ scale: 0.96; }}

  .account__run:focus-visible {{
    outline: 0;
    box-shadow: inset 0 0 0 2px var(--on-ink), inset 0 0 0 4px var(--ink);
  }}

  .account__narrator {{
    width: min(432px, 100%);
    margin: 7px auto 0;
    padding: 10px 16px 11px;
    background: var(--frost-solid);
    box-shadow: 0 0 0 1px var(--ring);
    color: var(--soft);
    font-family: var(--sans);
    font-size: 13px;
    line-height: 1.55;
    text-wrap: pretty;
    overflow-wrap: anywhere;
  }}

  .account__narrator p {{ margin: 0; }}

  @media (prefers-color-scheme: dark) {{
    :root {{
      --page: #161613;
      --ink: #e9e6d6;
      --on-ink: #22221c;
      --soft: #cdcaba;
      --ring: rgb(233 230 214 / 55%);
      --frost-solid: #1e1e19;
      --wash-p: rgb(19 19 19 / 14%);
    }}
  }}

  @media (max-width: 463px) {{
    body {{ padding-top: max(24px, env(safe-area-inset-top)); }}
    .account {{ width: min(432px, calc(100vw - 32px)); }}
    .account__logo {{
      width: 98px;
      margin-bottom: 32px;
    }}
  }}

  @media (prefers-reduced-motion: reduce) {{
    .account__run {{ transition: none; }}
  }}
</style>
<main class="account">
  <svg class="account__logo" viewBox="0 0 1024 343" role="img" aria-label="tonk" xmlns="http://www.w3.org/2000/svg">
    <path fill="currentColor" d="M337.41 169.57C337.41 136.215 363.983 109.171 396.764 109.171C429.539 109.171 456.112 136.215 456.112 169.57C456.112 202.932 429.539 229.975 396.764 229.975C363.983 229.975 337.41 202.932 337.41 169.57ZM643.51 54C614.396 54 589.91 61.6682 572.703 77.2353C563.913 85.1861 557.227 94.8795 552.759 105.986C552.206 97.3086 549.67 85.1977 540.937 75.1754C530.712 63.4453 515.039 55.8233 494.461 56.2041C488.76 56.3253 484.229 61.045 484.345 66.7514C484.46 72.3886 489.06 76.8833 494.668 76.8833C494.737 76.8833 494.812 76.8833 494.881 76.8833C509.044 76.6006 519.252 81.8339 525.282 88.6654C534.084 98.6415 532.072 113.834 532.02 114.197C531.167 119.817 535.012 125.079 540.626 125.962C541.168 126.048 545.537 126.602 547.612 124.565C546.673 130.203 546.171 136.076 546.171 142.192C546.171 183.349 531.225 224.038 507.39 247.764C488.403 266.66 465.219 275.805 434.911 279.382C479.549 263.671 511.569 221.038 511.569 170.874C511.569 107.365 460.262 55.8868 396.966 55.8868C333.675 55.8868 282.368 107.365 282.368 170.874C282.368 234.383 333.675 285.868 396.966 285.868C399.121 285.868 401.254 285.793 403.375 285.677C451.126 285.1 484.569 276.722 510.508 250.902C535.196 226.328 550.592 184.555 550.592 142.192C550.592 90.1771 586.197 57.8658 643.51 57.8658C737.703 57.8658 741.576 107.169 741.657 113.96C731.477 116.378 708.795 127.093 708.795 147.247V284.702H742.314C745.744 284.702 746.107 282.556 746.107 281.333V114.209C746.107 107.983 743.208 54 643.51 54Z"/>
    <path fill="currentColor" d="M144.171 58.5683L129.662 58.5856C106.081 58.6318 91.4574 69.3695 83.3356 77.9031C72.9486 88.8081 67 103.873 67 119.221C67 148.151 86.5636 179.285 133.299 179.285V155.658C100.513 155.658 90.6043 136.029 90.6043 119.221C90.6043 104.029 99.7116 81.7516 130.135 81.7516L144.171 81.7458V261.448C131.691 261.448 124.786 248.968 124.786 238.08L104.974 238.092C104.974 267.316 130.095 284.383 143.98 284.706H170.432C195.829 284.706 215.053 259.717 215.053 196.924V107.306H225.924V155.658H274.164V58.5683H144.171Z"/>
    <path fill="currentColor" d="M926.713 197.694L895.344 197.463C903.656 193.101 910.337 187.937 915.461 183.073C941.844 158.049 955.222 116.16 955.222 58.5828H938.99C938.99 111.527 927.318 149.447 904.302 171.286C893.817 181.227 875.475 192.721 847.444 191.544V58.5828H776.988V211.663C776.988 244.701 754.468 249.9 754.468 249.9C754.474 249.9 754.468 284.715 754.468 284.715H847.444V216.815C847.444 211.703 856.764 211.721 856.764 216.815C856.764 225.54 856.741 239.803 856.741 248.977C856.741 275.98 879.85 289.735 907.633 289.735C932.039 289.735 955.43 274.601 956.098 248.977C956.536 232.186 960.612 197.694 926.713 197.694Z"/>
  </svg>
  <section class="account__ceremony" aria-labelledby="confirmation-title">
    <div class="account__stack">
      <h1 id="confirmation-title" class="account__ceremony-head">command-line access</h1>
      <button class="account__run" type="button" onclick="window.close()">close this window</button>
    </div>
    <div class="account__narrator" role="status">
      <p>{message}</p>
    </div>
  </section>
</main>
</html>
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

    #[test]
    fn confirmation_matches_the_account_ceremony_shell() {
        let page = confirmation("Authorized. You can return to your terminal.");

        assert!(page.contains(r#"aria-label="tonk""#));
        assert!(page.contains("command-line access"));
        assert!(page.contains(r#"role="status""#));
        assert!(page.contains("Authorized. You can return to your terminal."));
        assert!(page.contains("--page: #ececec"));
        assert!(page.contains("--page: #161613"));
        assert!(page.contains("width: min(432px, 100%)"));
        assert!(page.contains("min-height: 44px"));
        assert!(page.contains("@media (max-width: 463px)"));
        assert!(page.contains("@media (prefers-reduced-motion: reduce)"));
    }

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
