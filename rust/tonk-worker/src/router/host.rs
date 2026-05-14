//! Host/guest iframe bridge.
//!
//! A hosting document (the Tonk shell) discovers its own
//! service-worker Client ID from the `X-Tonk-Client-Id` header
//! the SW echoes on every response. It then embeds a sandboxed
//! iframe pointed at
//! `/api/repository/{repo}/branch/{branch}/host/{host}/{entity}`.
//! When the browser issues the initial navigation fetch for that
//! URL the service worker:
//!
//! - records `resulting_client_id → {repo, branch}` in the
//!   [`GuestBindings`] map hanging off `TonkState`, so any
//!   subsequent subresource fetch from the same iframe can be
//!   identified by client id alone without re-parsing the URL;
//! - serves the entity's body by selecting the claim
//!   `(the = <mime>, of = <entity>)` against the branch and
//!   returning its `is` value with the chosen MIME as the
//!   response's `Content-Type`.
//!
//! Subsequent subresource fetches from inside the iframe are
//! intercepted by the worker's `route_for`, which sees the
//! registered guest client and rewrites the path to live under
//! `/api/repository/{repo}/branch/{branch}/...`. That gives the
//! iframe-rendered content a virtual root scoped to the branch
//! it was loaded from, so `<script src="/foo.js">` resolves to
//! `/api/repository/{repo}/branch/{branch}/foo.js`.

use std::{collections::HashMap, sync::Arc};

use ::axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Value};
use dialog_repository::RepositoryExt as _;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::RwLock;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// A service worker Client ID, extracted from a `FetchEvent` by
/// the worker's `on_fetch` and attached to each request as an
/// extension.
///
/// This is the stable identifier for the document/worker that
/// initiated the request — it outlives any single fetch and is
/// stable for the lifetime of the document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(pub String);

/// Binding that records which repository and branch an iframe
/// client is associated with.
///
/// Populated on the iframe's initial navigation fetch; looked up
/// on every subsequent subresource fetch from the same client.
#[derive(Clone, Debug)]
pub struct GuestBinding {
    /// The repository name the iframe is scoped to.
    pub repo: String,
    /// The branch name the iframe is scoped to.
    pub branch: String,
}

/// Shared map of guest `ClientId → GuestBinding`.
///
/// Lives on [`TonkState::guests`] behind its own interior
/// `RwLock`, so guest registration / lookup doesn't serialize
/// against profile or operator access on the outer state lock.
pub type GuestBindings = Arc<RwLock<HashMap<ClientId, GuestBinding>>>;

/// Path parameters for the bridge route.
#[derive(Debug, Deserialize)]
pub struct GuestPath {
    /// The hosting document's Client ID. Cosmetic — the
    /// authoritative binding is keyed off the *current*
    /// fetch's client id, not this URL segment.
    #[allow(dead_code)]
    pub host: String,
    /// The repository name the iframe is scoped to.
    pub repo: String,
    /// The branch name the iframe is scoped to.
    pub branch: String,
    /// The entity URI whose body should fill the iframe.
    /// May carry a trailing `.<ext>` to pick a non-HTML MIME
    /// type — useful for entities served as images, scripts,
    /// stylesheets, etc.
    pub entity: String,
}

/// Split a trailing `.<ext>` off the entity segment so callers
/// can request a specific MIME type via the URL alone (handy
/// for `<img src=".../{entity}.png">`-style references).
///
/// The split is conservative: only a final dot followed by an
/// alphanumeric token counts. Anything else (DIDs, URI-shaped
/// entities, names that legitimately contain dots) is returned
/// as-is with no extension.
fn split_extension(entity: &str) -> (&str, Option<&str>) {
    let Some(dot) = entity.rfind('.') else {
        return (entity, None);
    };
    let ext = &entity[dot + 1..];
    if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        (&entity[..dot], Some(ext))
    } else {
        (entity, None)
    }
}

/// Map a URL extension to the MIME type used as the claim's
/// `the` attribute. Anything we don't have an explicit mapping
/// for falls back to `application/<ext>` — a producer can assert
/// under any attribute name and the bridge will route to it
/// without a code change.
fn mime_for_extension(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html".to_string(),
        "css" => "text/css".to_string(),
        "js" | "mjs" => "application/javascript".to_string(),
        "json" => "application/json".to_string(),
        "txt" => "text/plain".to_string(),
        "md" => "text/markdown".to_string(),
        "svg" => "image/svg+xml".to_string(),
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "wasm" => "application/wasm".to_string(),
        other => format!("application/{}", other),
    }
}

/// Render a [`Value`] into an HTTP response body.
///
/// Strings/symbols/entities go out as-is. Bytes/records are
/// served raw. Numerics and booleans are stringified — they
/// don't have an unambiguous binary form, and the caller asked
/// for a specific MIME type, so a text rendering is the least
/// surprising option.
fn body_for(value: Value) -> Body {
    match value {
        Value::String(s) => Body::from(s),
        Value::Symbol(s) => Body::from(s.to_string()),
        Value::Entity(e) => Body::from(e.to_string()),
        Value::Bytes(b) => Body::from(b),
        Value::Record(r) => Body::from(r),
        Value::Boolean(b) => Body::from(b.to_string()),
        Value::SignedInt(i) => Body::from(i.to_string()),
        Value::UnsignedInt(u) => Body::from(u.to_string()),
        Value::Float(f) => Body::from(f.to_string()),
    }
}

/// Vanilla-JS `<tonk-concept>` runtime, inlined into every served
/// `text/html` body via [`wrap_html_body`]. The script registers
/// the element in the iframe's own `customElements` registry —
/// the parent shell's registration doesn't reach across documents.
const CONCEPT_RUNTIME: &str = include_str!("../../assets/tonk-concept.js");

/// Wrap an agent-authored body fragment in a fixed shell that
/// hydrates `<tonk-concept>` elements. The agent writes only the
/// `<body>` content; we provide the doctype, the script, and the
/// `<body>` boundary.
fn wrap_html_body(body: &str) -> String {
    format!(
        "<!doctype html>\n\
         <html>\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <script>{runtime}</script>\n\
         </head>\n\
         <body>\n\
         {body}\n\
         </body>\n\
         </html>\n",
        runtime = CONCEPT_RUNTIME,
        body = body,
    )
}

/// Handler for `GET /api/repository/{repo}/branch/{branch}/host/{host}/{entity}`.
///
/// Registers a guest binding for the requesting client and
/// returns the entity's body. The MIME type comes from the
/// entity's trailing extension (`{entity}.html`, `.css`, ...);
/// a bare entity defaults to `text/html`. The selected MIME is
/// used both as the claim's `the` attribute and as the
/// response's `Content-Type`.
#[wasm_compat]
pub async fn guest(
    State(state): State<AppState>,
    Path(params): Path<GuestPath>,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    let client_id = request.extensions().get::<ClientId>().cloned();

    let (entity_str, extension) = split_extension(&params.entity);
    let attribute_str = match extension {
        Some(ext) => mime_for_extension(ext),
        None => "text/html".to_string(),
    };
    log!(
        "guest navigation: repo={} branch={} entity={} the={} client={:?}",
        params.repo,
        params.branch,
        entity_str,
        attribute_str,
        client_id,
    );

    if let Some(client) = client_id {
        let guests = state.read().await.guests.clone();
        guests.write().await.insert(
            client,
            GuestBinding {
                repo: params.repo.clone(),
                branch: params.branch.clone(),
            },
        );
    }

    let attribute: Attribute = attribute_str.parse().map_err(|e| {
        TonkWorkerError::Router(format!("Invalid attribute '{}': {}", attribute_str, e))
    })?;
    let entity: Entity = entity_str
        .parse()
        .map_err(|e| TonkWorkerError::Router(format!("Invalid entity '{}': {}", entity_str, e)))?;

    let tonk = state.read().await;

    let repo = tonk
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", params.repo, e))
        })?;

    let branch = repo
        .branch(params.branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", params.branch, e))
        })?;

    let selector = ArtifactSelector::new().the(attribute).of(entity);
    let stream = branch
        .claims()
        .select(selector)
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Query execution error: {}", e)))?;

    tokio::pin!(stream);

    while let Some(result) = stream.next().await {
        match result {
            Ok(artifact) => {
                // Agent-authored HTML is treated as a body
                // fragment: wrap it with the doctype, the
                // `<tonk-concept>` runtime, and the `<body>`
                // boundary before serving. The agent never sees
                // these — it writes the body of a layout and the
                // host hydrates it.
                let body = if attribute_str == "text/html"
                    && let Value::String(s) = &artifact.is
                {
                    Body::from(wrap_html_body(s))
                } else {
                    body_for(artifact.is)
                };
                let mut response = (StatusCode::OK, body).into_response();
                if let Ok(value) = HeaderValue::from_str(&attribute_str) {
                    response.headers_mut().insert(header::CONTENT_TYPE, value);
                }
                return Ok(response);
            }
            Err(e) => {
                log!("guest: error reading artifact: {:?}", e);
            }
        }
    }

    Err(TonkWorkerError::NotFound(format!(
        "No claim found for entity={} attribute={}",
        entity_str, attribute_str,
    )))
}
