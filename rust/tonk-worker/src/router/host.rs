//! Host/guest iframe bridge.
//!
//! A hosting document (e.g. the Tonk shell) discovers its own
//! Client ID from the `X-Tonk-Client-Id` header that the service
//! worker echoes on every response. It then embeds a sandboxed
//! iframe pointed at `/api/host/{host_id}/guest/{repo}/`. When the
//! browser issues the initial navigation fetch for that URL the
//! service worker:
//!
//! - records `resulting_client_id → {host_id, repo}` in the
//!   [`GuestBindings`] map hanging off `TonkState`, so that all
//!   subsequent subresource fetches from the same iframe can be
//!   identified by client id alone without parsing the URL each
//!   time;
//! - serves a hardcoded HTML placeholder that exercises the full
//!   round-trip (fetch back into the SW, read the clientId
//!   binding, dispatch to an existing API handler).
//!
//! This is step 1 of the design. Path-rewriting into repository-
//! scoped routes will come in a later step.
//!
//! The host_id is validated against the live set of controlled
//! clients (via `self.clients.get(host_id)`) at some point — for
//! now we accept any non-empty string so the first iteration is
//! easy to poke at; tightening that up is a follow-up.

use std::{collections::HashMap, sync::Arc};

use ::axum::{
    Json,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use tokio::sync::RwLock;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::{AppState, identify::IdentifyResponse};

/// Snapshot what the guest-register / guest-lookup path needs
/// out of [`AppState`], released before any awaits on the guest
/// map itself, so that guest traffic never contends with
/// profile/operator access on the outer [`TonkState`] lock.
async fn snapshot(state: &AppState) -> (GuestBindings, String) {
    let tonk = state.read().await;
    (tonk.guests.clone(), tonk.profile.did().to_string())
}

/// A service worker Client ID, extracted from a `FetchEvent` by
/// the JS shim and attached to each request as an extension.
///
/// This is the stable identifier for the document/worker that
/// initiated the request — it outlives any single fetch and is
/// stable for the lifetime of the document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

/// Binding that records which repository (and which hosting
/// document) an iframe client is associated with.
///
/// Populated on the iframe's initial navigation fetch; looked up
/// on every subsequent subresource fetch from the same client.
#[derive(Clone, Debug)]
pub struct GuestBinding {
    /// The Client ID of the hosting document — the one that
    /// rendered the iframe.
    pub host: ClientId,
    /// The repository name the iframe is scoped to.
    pub repo: String,
}

/// Shared map of guest `ClientId → GuestBinding`.
///
/// Lives on [`TonkState::guests`] behind its own interior
/// `RwLock`, so guest registration / lookup doesn't serialize
/// against profile or operator access on the outer state lock.
pub type GuestBindings = Arc<RwLock<HashMap<ClientId, GuestBinding>>>;

/// Placeholder HTML served for a guest iframe.
///
/// This is what the parent's bootstrap script pulls in via
/// `fetch` + `document.write`. The `<base href="...">` is what
/// makes relative URLs resolve under the guest's namespace: the
/// iframe's own document URL is `about:srcdoc`, so without a
/// base the browser would resolve `api/identify` against that
/// (and fail). With the base set, `fetch("api/identify")` lands
/// at `/api/host/{host}/guest/{repo}/api/identify`, which the
/// guest router dispatches to the real identify handler.
fn placeholder_html(_host: &str, repo: &str) -> String {
    // The iframe's script fetches `"/"` without knowing its
    // own repo name — the service worker's `guest_rewrite`
    // middleware sees the request's clientId, looks up the
    // guest binding, and rewrites `/` to
    // `/api/repository/{repo}`. The iframe stays namespace-
    // agnostic; the SW handles the scoping.
    format!(
        r#"<!DOCTYPE html>
<meta charset="utf-8">
<title>Tonk Guest — {repo}</title>
<body>
  <pre class="repository" id="repo">loading…</pre>
  <script>
    fetch("/")
      .then(r => r.json())
      .then(info => {{
        document.getElementById("repo").textContent =
          JSON.stringify(info, null, 2);
      }})
      .catch(e => {{
        document.getElementById("repo").textContent =
          "error: " + e;
      }});
  </script>
</body>
"#
    )
}

/// Handler for `GET /api/host/{host_id}/guest/{repo}/{*rest}`.
///
/// On the iframe's initial navigation (empty `rest`) records a
/// guest binding against the requesting client's ID and returns
/// the placeholder HTML.
///
/// On subresource fetches (non-empty `rest`) looks up the
/// caller's binding and, for step 1, dispatches exactly one
/// known path (`api/identify`) to its real handler. Full path
/// rewriting into the repository-scoped routes is a later step.
#[wasm_compat]
pub async fn guest(
    State(state): State<AppState>,
    Path((host_id, repo, rest)): Path<(String, String, String)>,
    request: Request,
) -> Response {
    let client_id = request.extensions().get::<ClientId>().cloned();

    log!(
        "GUEST host={} repo={} rest={:?} client={:?}",
        host_id,
        repo,
        rest,
        client_id,
    );

    // Navigation request: empty rest means the iframe is loading
    // its own document. Register the client and serve HTML.
    if rest.is_empty() {
        if let Some(client) = client_id {
            let guests = state.read().await.guests.clone();
            guests.write().await.insert(
                client.clone(),
                GuestBinding {
                    host: ClientId(host_id.clone()),
                    repo: repo.clone(),
                },
            );
            log!(
                "Registered guest client {:?} -> host={} repo={}",
                client,
                host_id,
                repo
            );
        }

        let body = placeholder_html(&host_id, &repo);
        let mut response = (StatusCode::OK, body).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        return response;
    }

    // Subresource request: client must already be bound.
    let Some(client) = client_id else {
        return (StatusCode::FORBIDDEN, "missing client id").into_response();
    };

    let (guests, did) = snapshot(&state).await;
    let binding = guests.read().await.get(&client).cloned();
    let Some(binding) = binding else {
        return (StatusCode::FORBIDDEN, "client is not a registered guest").into_response();
    };

    // The authoritative binding is on clientId; the host_id/repo
    // in the URL is cosmetic. If they disagree, something has
    // drifted and it's not safe to serve.
    if binding.repo != repo || binding.host.0 != host_id {
        return (StatusCode::FORBIDDEN, "client binding does not match URL").into_response();
    }

    if rest == "api/identify" {
        return Json(IdentifyResponse { did }).into_response();
    }

    (StatusCode::NOT_FOUND, format!("no guest route for {rest}")).into_response()
}

/// Thin shim for the empty-path variant of the guest route.
///
/// axum's `{*rest}` wildcard requires at least one segment, so a
/// request for the bare trailing-slash URL doesn't match the
/// wildcard route — it needs its own entry. This handler just
/// constructs a fake empty `rest` and defers to [`guest`], so
/// all the logic stays in one place.
#[wasm_compat]
pub async fn guest_empty_rest(
    state: State<AppState>,
    Path((host_id, repo)): Path<(String, String)>,
    request: Request,
) -> Response {
    let path = ::axum::extract::Path((host_id, repo, String::new()));
    guest(state, path, request).await
}
