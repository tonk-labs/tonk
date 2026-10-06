//! The loopback address a native host's sign-in is answered on.
//!
//! A page in a browser comes back to its own `/settings/link`. A native
//! host has no https page, and the approving deployment delivers only to
//! an https page or a bare `http://127.0.0.1:<port>/`, so the host
//! listens there the way the `tonk` CLI does: one listener per request,
//! on a port the OS picks, closed once it is answered or the deadline
//! passes.
//!
//! The approving page navigates here with a bodyless GET and carries the
//! answer in the URL fragment, which browsers do not send over the
//! network. The bridge page reads the fragment, drops it from history,
//! and posts it back to this same origin. The answer is then finished
//! exactly as a page's callback would be: [`super::complete`] checks it
//! against the request this profile recorded and installs the grant.
//!
//! The loopback address can carry no query, so the request id cannot ride
//! the callback as it does in a browser. It is attached here instead, and
//! what stands in for it is that the port is chosen per request and
//! answers once. A crafted link to this address during the wait is
//! therefore not told apart from the real answer, the same exposure the
//! CLI's callback has.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use serde::Deserialize;
use tokio::sync::Notify;
use tonk_common::log;
use url::Url;

/// How long the listener waits for the person to approve. An abandoned
/// approval should not leave a port open for the life of the process.
const DEADLINE: Duration = Duration::from_secs(5 * 60);

/// The listener waiting on the latest request. Asking again replaces it:
/// only the newest request is recorded, so an older listener could only
/// ever deliver a refused answer.
static WAITING: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

#[derive(Clone)]
struct Waiting {
    state: crate::router::AppState,
    /// The request this listener answers, as recorded by `start`.
    request: String,
    /// The origin of the deployment asked: the only place a delivered
    /// `redirect` may send the browser.
    via: String,
    /// This listener's own address.
    callback: String,
    answered: Arc<AtomicBool>,
    done: Arc<Notify>,
}

/// What the bridge page posts: the fragment's fields.
#[derive(Deserialize)]
struct Delivery {
    #[serde(default)]
    authorize: Option<String>,
    #[serde(default)]
    deny: Option<String>,
    /// Where to send the browser afterwards, so the approving page shows
    /// the outcome in its own styling.
    #[serde(default)]
    redirect: Option<String>,
}

/// Listen for the answer to `request`, asked of `via`, and answer the
/// callback address to hand the approving deployment.
pub(super) async fn listen(
    state: crate::router::AppState,
    request: String,
    via: String,
) -> Result<String, String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("the sign-in callback could not listen: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("the sign-in callback has no address: {error}"))?
        .port();
    let callback = callback_url(port);
    let done = Arc::new(Notify::new());
    let waiting = Waiting {
        state,
        request,
        via,
        callback: callback.clone(),
        answered: Arc::new(AtomicBool::new(false)),
        done: done.clone(),
    };
    let app = Router::new()
        .route("/", get(bridge).post(deliver))
        .with_state(waiting);
    let task = tokio::spawn(async move {
        let serving =
            axum::serve(listener, app).with_graceful_shutdown(async move { done.notified().await });
        match tokio::time::timeout(DEADLINE, serving).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log!("sign-in-via: the callback stopped: {error}"),
            Err(_) => log!("sign-in-via: no answer arrived; the callback closed"),
        }
    });
    let previous = WAITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .replace(task.abort_handle());
    if let Some(previous) = previous {
        previous.abort();
    }
    Ok(callback)
}

/// The callback address for a listener on `port`: bare, as the approving
/// deployment requires of a loopback callback.
fn callback_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The answer as a page's callback would have received it: the request
/// id in the query, the outcome in the fragment.
fn answer_url(callback: &str, request: &str, outcome: (&str, &str)) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("request", request)
        .finish();
    let fragment = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(outcome.0, outcome.1)
        .finish();
    format!("{callback}/?{query}#{fragment}")
}

/// Whether `redirect` is on the deployment asked.
fn redirect_allowed(redirect: &str, via: &str) -> bool {
    Url::parse(redirect).is_ok_and(|url| url.origin().ascii_serialization() == via)
}

async fn deliver(State(waiting): State<Waiting>, Form(delivery): Form<Delivery>) -> Response {
    let outcome = match (&delivery.authorize, &delivery.deny) {
        (Some(grant), _) if !grant.is_empty() => ("authorize", grant.as_str()),
        (_, Some(reason)) => ("deny", reason.as_str()),
        _ => return (StatusCode::BAD_REQUEST, "No answer was delivered.").into_response(),
    };
    if waiting.answered.swap(true, Ordering::SeqCst) {
        return (StatusCode::CONFLICT, "This sign-in was already answered.").into_response();
    }
    let answer = answer_url(&waiting.callback, &waiting.request, outcome);
    let state = waiting.state.clone();
    crate::detach(async move { super::complete(&state, None, &answer).await });
    waiting.done.notify_one();
    match delivery
        .redirect
        .filter(|redirect| redirect_allowed(redirect, &waiting.via))
    {
        Some(redirect) => Redirect::to(&redirect).into_response(),
        None => Html(RETURNED).into_response(),
    }
}

/// Shown when the approving page names nowhere to return to.
const RETURNED: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Tonk</title>
<p style="font: 16px/1.5 system-ui, sans-serif; text-align: center; margin-top: 30vh">
  Tonk has your answer. You can close this window.
</p>
"#;

/// Land the bodyless GET, then post its fragment back to this origin.
///
/// A visit without an answer does not use up the callback, so a prefetch
/// or a stray visit cannot answer the request.
async fn bridge() -> Html<&'static str> {
    Html(
        r##"<!doctype html>
<meta charset="utf-8">
<meta name="referrer" content="no-referrer">
<title>Tonk</title>
<p id="status" style="font: 16px/1.5 system-ui, sans-serif; text-align: center; margin-top: 30vh">
  Returning your answer to Tonk…
</p>
<script>
  const fields = new URLSearchParams(window.location.hash.slice(1));
  history.replaceState(null, "", window.location.pathname);
  if (!fields.has("authorize") && !fields.has("deny")) {
    document.querySelector("#status").textContent =
      "No answer was provided. You can close this window.";
  } else {
    const form = document.createElement("form");
    form.method = "post";
    form.action = window.location.pathname;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_hands_out_a_callback_the_approving_deployment_delivers_to() {
        let callback = callback_url(4321);
        assert!(tonk_worker_api::callback::delivery_url(&callback, &[]).is_ok());
    }

    #[test]
    fn it_finishes_the_answer_as_a_page_callback_would() {
        let granted = answer_url("http://127.0.0.1:4321", "r1", ("authorize", "eyJhIjoxfQ=="));
        assert_eq!(
            super::super::parse_answer(&granted).unwrap(),
            super::super::Answer {
                request: "r1".into(),
                outcome: super::super::Outcome::Granted("eyJhIjoxfQ==".into()),
            }
        );

        let denied = answer_url("http://127.0.0.1:4321", "r2", ("deny", "declined & closed"));
        assert_eq!(
            super::super::parse_answer(&denied).unwrap(),
            super::super::Answer {
                request: "r2".into(),
                outcome: super::super::Outcome::Denied("declined & closed".into()),
            }
        );
    }

    #[test]
    fn it_returns_the_browser_only_to_the_deployment_asked() {
        let via = "https://tonk.network";
        assert!(redirect_allowed(
            "https://tonk.network/settings?done=1",
            via
        ));
        assert!(!redirect_allowed("https://evil.example/", via));
        assert!(!redirect_allowed("http://tonk.network/", via));
        assert!(!redirect_allowed("not a url", via));
    }
}
