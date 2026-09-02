//! In-process language server, exposed over HTTP.
//!
//! The carry asserted-notation language server lives inside this
//! service worker — same process as the dialog-db, so completion
//! providers (when we add them) can hit live data without crossing
//! a network boundary.
//!
//! One logical endpoint per repository/branch scope, with two methods:
//!
//! - `POST …/language-server` — request/response. The body is
//!   a single JSON-RPC 2.0 message (no LSP `Content-Length`
//!   framing — that only matters for stdio transports). The
//!   response body is the matching JSON-RPC reply, or empty for
//!   notifications.
//!
//! - `GET …/language-server` (with `Accept: text/event-stream`)
//!   — opens a server-sent-event subscription. Server-initiated
//!   notifications (most importantly
//!   `textDocument/publishDiagnostics`) arrive here.
//!
//! Two methods on one route rather than two routes is a deliberate
//! shape choice: the *client* should care about LSP, not about how
//! it's plumbed. Different operations on the same logical resource
//! (a JSON-RPC channel) belong on the same URL with different
//! verbs. WebSocket would be tidier still — but service workers
//! don't intercept WebSocket connections, so the channel has to
//! ride on `fetch`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    Extension,
    body::{Body, Bytes},
    extract::{Path, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_wasm_macros::wasm_compat;
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use tokio::sync::{Mutex, broadcast};
// `wasm_compat` expands to code that uses `oneshot` by bare path
// on the wasm cfg; on native it isn't referenced. Import it
// unconditionally so both cfgs see the symbol.
#[allow(unused_imports)]
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tonk_common::log;
use tonk_language_server::{EnvProvider, Repo, Server};

use crate::router::lsp_env::LspEnvProvider;
use crate::router::update_pending;
use crate::router::{AppState, ClientId};

/// Channel capacity for outbound LSP notifications.
///
/// Each event is at most a few hundred bytes (a `publishDiagnostics`
/// payload). The capacity sets an upper bound on how many events a
/// slow consumer can fall behind before we start dropping — once
/// every receiver is at the head, the buffer wraps. 256 is enough
/// for any realistic burst (a `didChange` produces exactly one
/// notification today; even rapid typing won't outrun the SSE
/// stream).
const OUTBOUND_BUFFER: usize = 256;

/// Repository + branch authority encoded in the worker route. The portal
/// derives this path from its trusted `with`/`allow` state; authored JSON-RPC
/// can only name document URIs inside this exact scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LspScope {
    repo: LspScopeRepo,
    branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LspScopeRepo {
    Named(String),
    Profile(String),
}

impl LspScope {
    fn named(repo: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            repo: LspScopeRepo::Named(repo.into()),
            branch: branch.into(),
        }
    }

    fn profile(profile: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            repo: LspScopeRepo::Profile(profile.into()),
            branch: branch.into(),
        }
    }

    /// Canonical LSP URI root for this route.
    fn uri_root(&self) -> String {
        let branch = tonk_worker_api::encode_lsp_scope_segment(&self.branch);
        let repo = match &self.repo {
            LspScopeRepo::Named(repo) => tonk_worker_api::encode_lsp_scope_segment(repo),
            LspScopeRepo::Profile(profile) => {
                let profile = tonk_worker_api::encode_lsp_scope_segment(profile);
                return format!("tonk-buffer:///profile:{profile}/{branch}/");
            }
        };
        format!("tonk-buffer:///{repo}/{branch}/")
    }

    /// Exact canonical HTTP endpoint for this authority. Checking the raw URI
    /// prevents alternate percent spellings from becoming route aliases.
    fn endpoint_path(&self) -> String {
        let branch = tonk_worker_api::encode_lsp_scope_segment(&self.branch);
        match &self.repo {
            LspScopeRepo::Named(repo) => format!(
                "/api/repository/{}/branch/{branch}/language-server",
                tonk_worker_api::encode_lsp_scope_segment(repo),
            ),
            LspScopeRepo::Profile(profile) => format!(
                "/api/profile/{}/branch/{branch}/language-server",
                tonk_worker_api::encode_lsp_scope_segment(profile),
            ),
        }
    }

    fn matches_endpoint_path(&self, path: &str) -> bool {
        let identities_are_legal = match &self.repo {
            LspScopeRepo::Named(repo) | LspScopeRepo::Profile(repo) => canonical_identity(repo),
        } && canonical_identity(&self.branch);
        identities_are_legal && path == self.endpoint_path()
    }

    /// A text document must have a non-empty suffix beneath the canonical
    /// scope root. The slash boundary prevents `mainish`/prefix aliases.
    fn contains_document_uri(&self, uri: &str) -> bool {
        uri.strip_prefix(&self.uri_root())
            .is_some_and(|suffix| !suffix.is_empty())
    }

    fn contains_root_or_document_uri(&self, uri: &str) -> bool {
        uri == self.uri_root() || self.contains_document_uri(uri)
    }

    pub(super) fn matches(&self, repo: &Repo, branch: &str) -> bool {
        if self.branch != branch {
            return false;
        }
        match (&self.repo, repo) {
            (LspScopeRepo::Named(expected), Repo::Named(actual)) => expected == actual,
            (LspScopeRepo::Profile(expected), Repo::Profile(actual)) => expected == actual,
            _ => false,
        }
    }

    pub(super) fn profile_name(&self) -> Option<&str> {
        match &self.repo {
            LspScopeRepo::Profile(name) => Some(name),
            LspScopeRepo::Named(_) => None,
        }
    }
}

fn canonical_identity(identity: &str) -> bool {
    let encoded = tonk_worker_api::encode_lsp_scope_segment(identity);
    tonk_worker_api::decode_lsp_scope_segment(&encoded).as_deref() == Some(identity)
}

/// Parse, classify, and scope one inbound JSON-RPC message before it can touch
/// document state or a live repository. Unknown methods and batch/response
/// shapes fail closed: adding an LSP operation requires reviewing every URI or
/// workspace field it can carry here first.
fn scope_inbound(raw: &[u8], scope: &LspScope) -> Result<Vec<u8>, String> {
    let mut value: serde_json::Value =
        serde_json::from_slice(raw).map_err(|error| format!("invalid JSON-RPC: {error}"))?;
    let message = value
        .as_object_mut()
        .ok_or_else(|| "expected one JSON-RPC object".to_owned())?;
    if message.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return Err("expected JSON-RPC 2.0".to_owned());
    }
    let method = message
        .get("method")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "client responses are not accepted by this server".to_owned())?
        .to_owned();
    let is_request = message.contains_key("id");
    let params = message.entry("params").or_insert(serde_json::Value::Null);

    match method.as_str() {
        "initialize" if is_request => scope_initialize(params, scope)?,
        "textDocument/completion" | "textDocument/hover" if is_request => {
            scope_text_document(params, scope)?;
        }
        "shutdown" if is_request => reject_location_fields(params)?,
        "initialized" | "exit" if !is_request => reject_location_fields(params)?,
        "$/cancelRequest" if !is_request => validate_cancel(params)?,
        "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didClose"
            if !is_request =>
        {
            scope_text_document(params, scope)?;
        }
        _ => {
            return Err(format!(
                "unsupported or ambiguous LSP message shape: {method}"
            ));
        }
    }

    serde_json::to_vec(&value).map_err(|error| format!("serialize scoped JSON-RPC: {error}"))
}

/// The bundled CodeMirror client cancels an in-flight completion when its
/// editor context is invalidated. The embedded server currently ignores that
/// notification, but rejecting it at HTTP would turn an ordinary cancellation
/// into a transport failure and reconnect. Accept only the exact id-only shape;
/// it cannot carry repository authority or document locations.
fn validate_cancel(params: &serde_json::Value) -> Result<(), String> {
    let params = params
        .as_object()
        .ok_or_else(|| "cancelRequest params must be an object".to_owned())?;
    if params.len() != 1 {
        return Err("cancelRequest accepts only an id".to_owned());
    }
    match params.get("id") {
        Some(serde_json::Value::String(_)) | Some(serde_json::Value::Number(_)) => Ok(()),
        _ => Err("cancelRequest id must be a string or number".to_owned()),
    }
}

fn scope_initialize(params: &mut serde_json::Value, scope: &LspScope) -> Result<(), String> {
    let params = params
        .as_object_mut()
        .ok_or_else(|| "initialize params must be an object".to_owned())?;
    let root = scope.uri_root();
    match params.get("rootUri") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(uri)) if uri == "tonk-buffer:///" || uri == &root => {}
        Some(_) => return Err("initialize rootUri is outside the route scope".to_owned()),
    }
    params.insert("rootUri".to_owned(), serde_json::Value::String(root));

    if params.get("rootPath").is_some_and(|value| !value.is_null()) {
        return Err("filesystem rootPath is not valid for a scoped Tonk session".to_owned());
    }
    if let Some(folders) = params.get("workspaceFolders")
        && !folders.is_null()
    {
        let folders = folders
            .as_array()
            .ok_or_else(|| "workspaceFolders must be null or an array".to_owned())?;
        for folder in folders {
            let uri = folder
                .get("uri")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "workspace folder is missing a URI".to_owned())?;
            if !scope.contains_root_or_document_uri(uri) {
                return Err("workspace folder URI is outside the route scope".to_owned());
            }
        }
    }
    location_fields_in_scope(&serde_json::Value::Object(params.clone()), scope)
}

fn scope_text_document(params: &serde_json::Value, scope: &LspScope) -> Result<(), String> {
    let uri = params
        .pointer("/textDocument/uri")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "textDocument URI is required".to_owned())?;
    if !scope.contains_document_uri(uri) {
        return Err("textDocument URI is outside the route scope".to_owned());
    }
    location_fields_in_scope(params, scope)
}

/// Future additions to an already-known method can carry location fields too.
/// Walk the complete params tree rather than trusting serde to reject unknown
/// fields (it does not). Any URI-shaped field must stay in scope; a non-null
/// filesystem root is never meaningful in the browser worker.
fn location_fields_in_scope(value: &serde_json::Value, scope: &LspScope) -> Result<(), String> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if key.eq_ignore_ascii_case("rootPath") {
                    if !child.is_null() {
                        return Err("filesystem paths are outside the Tonk route scope".to_owned());
                    }
                    continue;
                }
                if is_uri_field(key) {
                    let uri = child
                        .as_str()
                        .ok_or_else(|| format!("{key} must be a scoped URI string"))?;
                    if !scope.contains_root_or_document_uri(uri) {
                        return Err(format!("{key} is outside the route scope"));
                    }
                    continue;
                }
                location_fields_in_scope(child, scope)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                location_fields_in_scope(child, scope)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_location_fields(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if key.eq_ignore_ascii_case("rootPath") || is_uri_field(key) {
                    return Err(format!("{key} is not valid on this global LSP operation"));
                }
                reject_location_fields(child)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                reject_location_fields(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_uri_field(key: &str) -> bool {
    key.to_ascii_lowercase().ends_with("uri")
}

/// The embedded server currently emits only diagnostics. Keep the outbound
/// side default-deny too, so a future server notification cannot accidentally
/// become a worker-global SSE broadcast.
fn outbound_is_in_scope(method: &str, params: &serde_json::Value, scope: &LspScope) -> bool {
    method == "textDocument/publishDiagnostics"
        && params
            .get("uri")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|uri| scope.contains_document_uri(uri))
        && location_fields_in_scope(params, scope).is_ok()
}

/// Server state and SSE channel are isolated by BOTH trusted route scope and
/// logical portal client. The service-worker ClientId is used for top-level
/// callers; a sealed portal replaces any authored header with its own id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LspSessionKey {
    scope: LspScope,
    client: String,
}

impl LspSessionKey {
    fn new(scope: LspScope, client: impl Into<String>) -> Self {
        Self {
            scope,
            client: client.into(),
        }
    }
}

struct LspSession {
    server: Mutex<Server>,
    outbound: Mutex<Option<broadcast::Sender<Bytes>>>,
}

impl LspSession {
    fn new() -> Arc<Self> {
        let (outbound, _drop) = broadcast::channel(OUTBOUND_BUFFER);
        Arc::new(Self {
            server: Mutex::new(Server::new()),
            outbound: Mutex::new(Some(outbound)),
        })
    }
}

#[derive(Default)]
struct LspHubState {
    shutdown: bool,
    sessions: BTreeMap<LspSessionKey, Arc<LspSession>>,
}

/// Hub shared across LSP routes. One instance per worker; lives as
/// long as the worker does. Added to the router as an `Extension`
/// rather than wedged into `AppState` so it stays orthogonal to
/// the rest of the app's state shape.
pub struct LspHub {
    state: Mutex<LspHubState>,
}

impl LspHub {
    /// Construct a fresh hub with an empty document set and a
    /// lazily-created session map.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(LspHubState::default()),
        })
    }

    async fn session(&self, key: &LspSessionKey) -> Option<Arc<LspSession>> {
        let mut state = self.state.lock().await;
        if state.shutdown {
            return None;
        }
        Some(
            state
                .sessions
                .entry(key.clone())
                .or_insert_with(LspSession::new)
                .clone(),
        )
    }

    /// Run an LSP message through the server and queue any
    /// resulting outbound notifications onto the broadcast.
    /// Returns the JSON-RPC response (for requests) or `None`
    /// (for notifications and unparseable messages).
    ///
    /// `env` is the per-request [`LspEnvProvider`] — the language
    /// server resolves diagnostics, completion, and hover against
    /// whatever live branch it opens through it.
    async fn dispatch<P: EnvProvider>(
        &self,
        key: &LspSessionKey,
        raw: &[u8],
        env: &P,
    ) -> Result<Option<Vec<u8>>, String> {
        let scoped = scope_inbound(raw, &key.scope)?;
        let session = self
            .session(key)
            .await
            .ok_or_else(|| "the language server hub has shut down".to_owned())?;
        let mut server = session.server.lock().await;
        let reply = server.handle_message(&scoped, env).await;
        let outbound = session.outbound.lock().await;
        for note in server.take_outbound() {
            if !outbound_is_in_scope(note.method, &note.params, &key.scope) {
                log!(
                    "[lsp] dropped outbound method or URI outside session scope: {}",
                    note.method
                );
                continue;
            }
            let bytes = match serde_json::to_vec(&note) {
                Ok(v) => v,
                Err(err) => {
                    log!("[lsp] failed to serialize outbound notification: {err}");
                    continue;
                }
            };
            if let Some(sender) = outbound.as_ref() {
                // `send` errors only when there are zero receivers —
                // harmless, future subscribers will see future events.
                let _ = sender.send(Bytes::from(bytes));
            }
            // After shutdown there are no subscribers anyway; drop.
        }
        Ok(reply)
    }

    /// Subscribe to the outbound channel, or `None` once
    /// [`shutdown`](Self::shutdown) has run — matching what the
    /// `outbound` field documents.
    ///
    /// This used to hand back a receiver unconditionally, because
    /// `shutdown` installed a FRESH sender instead of emptying the
    /// slot. That made the hub's teardown reversible by any client
    /// that redialed, and the LSP client redials on a flat timer: the
    /// old worker dropped its streams on `updatefound`, then ~5 s
    /// later handed out a brand new one and re-pinned itself, parking
    /// its replacement in `waiting` for good.
    async fn subscribe(&self, key: &LspSessionKey) -> Option<broadcast::Receiver<Bytes>> {
        let session = self.session(key).await?;
        Some(session.outbound.lock().await.as_ref()?.subscribe())
    }

    /// Hang up every active SSE subscriber, terminally.
    ///
    /// Called from the worker's `onupdatefound` export when a newer
    /// SW version begins installing. Taking the sender (rather than
    /// swapping in a fresh one) drops it: every receiver surfaces
    /// `Closed`, each `BroadcastStream` ends, and the SSE response
    /// bodies finish — which settles the in-flight fetch events the
    /// spec was using to keep this worker alive.
    ///
    /// Terminal is the point. Installing a fresh sender here left the
    /// hub able to serve a NEW stream moments later, and the LSP
    /// client's reconnect timer did exactly that — re-pinning the
    /// worker this teardown existed to release.
    ///
    /// A worker that turns out not to be replaced after all (a failed
    /// install, a canceled upgrade) is covered by `handle_events`
    /// reading [`update_pending`] live: with no successor waiting it
    /// answers `503` + `Retry-After` instead of a dead 200, and the
    /// client's next dial after the successor activates lands on the
    /// new worker's hub, which has a sender of its own.
    pub async fn shutdown(&self) {
        let sessions = {
            let mut state = self.state.lock().await;
            state.shutdown = true;
            std::mem::take(&mut state.sessions)
        };
        for session in sessions.into_values() {
            session.outbound.lock().await.take();
        }
        // Every sender drops here — receivers tied to them surface `Closed`.
    }
}

/// Mount the LSP route onto an axum router. Takes the worker's
/// [`AppState`] so the POST handler can open the live environment
/// for completion / hover / diagnostics. Returns both the router
/// *and* a handle to the [`LspHub`] so the worker entry point can
/// call [`LspHub::shutdown`] when a newer service worker version
/// begins installing.
pub fn lsp_router(state: AppState) -> (axum::Router, Arc<LspHub>) {
    let hub = LspHub::new();
    let router = axum::Router::new()
        .route(
            "/api/repository/{repo}/branch/{branch}/language-server",
            get(handle_named_events).post(handle_named_post),
        )
        .route(
            "/api/profile/{profile}/branch/{branch}/language-server",
            get(handle_profile_events).post(handle_profile_post),
        )
        .layer(Extension(hub.clone()))
        .with_state(state);
    (router, hub)
}

#[wasm_compat]
async fn handle_named_post(
    Path((repo, branch)): Path<(String, String)>,
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    handle_post(LspScope::named(repo, branch), hub, state, request).await
}

#[wasm_compat]
async fn handle_profile_post(
    Path((profile, branch)): Path<(String, String)>,
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    handle_post(LspScope::profile(profile, branch), hub, state, request).await
}

/// Read one scoped JSON-RPC message, validate every authority-bearing field,
/// and dispatch it into the route+client session.
async fn handle_post(
    scope: LspScope,
    hub: Arc<LspHub>,
    state: AppState,
    request: Request,
) -> Response {
    if !scope.matches_endpoint_path(request.uri().path()) {
        return (
            StatusCode::BAD_REQUEST,
            "non-canonical language-server route",
        )
            .into_response();
    }
    let key = match session_key(scope.clone(), &request) {
        Ok(key) => key,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let bytes = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            log!("[lsp] failed to read request body: {err}");
            return (StatusCode::BAD_REQUEST, "failed to read body").into_response();
        }
    };

    // Always respond `200 OK`, even when the message was a
    // notification (no JSON-RPC reply). Using `204` would be
    // semantically cleaner, but the SW's response adapter
    // (`router::axum`) attaches a body stream for every response,
    // and 204 forbids one — the browser would throw. Empty 200 is
    // the JSON-RPC "no reply" signal anyway.
    let env = LspEnvProvider::new(state, scope);
    let body = match hub.dispatch(&key, &bytes, &env).await {
        Ok(body) => body.unwrap_or_default(),
        Err(error) => {
            log!("[lsp] rejected scoped message: {error}");
            return (StatusCode::BAD_REQUEST, error).into_response();
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[wasm_compat]
async fn handle_named_events(
    Path((repo, branch)): Path<(String, String)>,
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let retiring = state.read().await.is_retiring();
    handle_events(LspScope::named(repo, branch), hub, request, retiring).await
}

#[wasm_compat]
async fn handle_profile_events(
    Path((profile, branch)): Path<(String, String)>,
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let retiring = state.read().await.is_retiring();
    handle_events(LspScope::profile(profile, branch), hub, request, retiring).await
}

/// Return the SSE stream for exactly one scoped client session. SSE framing is
/// `data: <json>\n\n`; no worker-global broadcast channel exists.
async fn handle_events(
    scope: LspScope,
    hub: Arc<LspHub>,
    request: Request,
    retiring: bool,
) -> Response {
    if !scope.matches_endpoint_path(request.uri().path()) {
        return (
            StatusCode::BAD_REQUEST,
            "non-canonical language-server route",
        )
            .into_response();
    }
    // Refuse to open a long-lived stream while a successor is waiting.
    // An SSE body is a fetch event that never settles, and the spec
    // keeps a worker alive while any fetch event is in flight — so one
    // stream opened here re-pins this retiring worker and parks its
    // replacement in `waiting`. The query-subscription route has
    // refused for the same reason; this one didn't, and the LSP
    // client's reconnect timer made it the reliable way to wedge an
    // update.
    if retiring || update_pending() {
        return retry_later("a newer service worker is waiting to activate");
    }
    // `None` means `shutdown` already ran on this hub. Same answer:
    // the client should come back, and by then the successor will be
    // the controller and will answer with its own live hub.
    let key = match session_key(scope, &request) {
        Ok(key) => key,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let Some(receiver) = hub.subscribe(&key).await else {
        return retry_later("the language server hub has shut down");
    };
    // `BroadcastStream` adapts the receiver into a `Stream`. Lagged
    // items surface as `Err(BroadcastStreamRecvError::Lagged(n))`
    // which we silently filter — a slow consumer just catches up
    // from the newest message; the next edit will refresh the
    // diagnostic state for any document the client cares about.
    // When the hub's sender is dropped (via `LspHub::shutdown`) the
    // receiver yields `Closed` and the stream ends, terminating
    // this response body cleanly.
    let body_stream =
        BroadcastStream::new(receiver).filter_map(|result: Result<Bytes, _>| async move {
            let bytes = result.ok()?;
            let mut framed = Vec::with_capacity(bytes.len() + 8);
            framed.extend_from_slice(b"data: ");
            framed.extend_from_slice(&bytes);
            framed.extend_from_slice(b"\n\n");
            Some(Ok::<_, std::io::Error>(Bytes::from(framed)))
        });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(body_stream))
        .unwrap()
}

/// Bind a scoped route to a logical LSP client. A portal-provided id is safe
/// only because every relay layer strips the authored header and stamps its
/// own value after authorizing the route. Direct top-page clients fall back to
/// the browser's service-worker ClientId. Combining both prevents collisions
/// across tabs even if two portals happened to mint the same token.
fn session_key(scope: LspScope, request: &Request) -> Result<LspSessionKey, String> {
    let worker_client = request
        .extensions()
        .get::<ClientId>()
        .map(|client| client.0.as_str())
        .filter(|client| !client.is_empty());
    let mut portal_clients = request
        .headers()
        .get_all(tonk_worker_api::LSP_CLIENT_HEADER)
        .iter();
    let portal_client = portal_clients
        .next()
        .map(|value| {
            value
                .to_str()
                .map_err(|_| "invalid LSP client header".to_owned())
        })
        .transpose()?
        .filter(|client| !client.is_empty());
    if portal_clients.next().is_some() {
        return Err("ambiguous LSP client header".to_owned());
    }
    if portal_client.is_some_and(|client| !tonk_worker_api::is_canonical_lsp_client_chain(client)) {
        return Err("invalid LSP client chain".to_owned());
    }

    let client = match (worker_client, portal_client) {
        (Some(worker), Some(portal)) => format!("{worker}/portal/{portal}"),
        (Some(worker), None) => worker.to_owned(),
        (None, Some(portal)) => format!("portal/{portal}"),
        (None, None) => return Err("missing trusted LSP client identity".to_owned()),
    };
    if client.len() > 256
        || !client
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'/'))
    {
        return Err("invalid LSP client identity".to_owned());
    }
    Ok(LspSessionKey::new(scope, client))
}

/// Tell the client to come back rather than handing it a stream this
/// worker must not open. `Retry-After` is advisory; the client's own
/// held reconnect (it waits for `controllerchange`) is what actually
/// paces the redial onto the successor.
fn retry_later(reason: &str) -> Response {
    log!("[lsp] refusing SSE subscription: {reason}");
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::RETRY_AFTER, "5")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "control": "update-pending", "reason": reason }).to_string(),
        ))
        .expect("response builder failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tonk_language_server::NoEnv;
    use tower::ServiceExt as _;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// `shutdown` must be TERMINAL. It used to install a fresh sender,
    /// so a client that redialed moments later got a working stream on
    /// a worker that was trying to retire — and an SSE body is a fetch
    /// event that never settles, so that stream re-pinned the outgoing
    /// worker and parked its replacement in `waiting` indefinitely.
    /// That is the "Safari keeps the old version through every reload"
    /// symptom: the reloads land on the old ACTIVE worker.
    #[dialog_common::test]
    async fn it_refuses_to_subscribe_after_shutdown() {
        let hub = LspHub::new();
        let key = test_key();
        assert!(
            hub.subscribe(&key).await.is_some(),
            "a live hub hands out receivers"
        );

        hub.shutdown().await;
        assert!(
            hub.subscribe(&key).await.is_none(),
            "a hub that has shut down must not hand out a new receiver"
        );

        // And it stays terminal: a second dial doesn't revive it either.
        assert!(
            hub.subscribe(&key).await.is_none(),
            "shutdown is one-way for this hub's lifetime"
        );
    }

    /// The route — not just the hub — must decline after shutdown, and
    /// it must decline in the shape the client can act on: a `503`
    /// carrying `update-pending`, so the consumer HOLDS its reconnect
    /// for `controllerchange` instead of redialing this worker on a
    /// timer. A plain error would be indistinguishable from a network
    /// blip and would be retried on the short backoff, which is what
    /// re-pinned the outgoing worker.
    #[dialog_common::test]
    async fn it_answers_retry_later_after_shutdown() {
        let hub = LspHub::new();
        hub.shutdown().await;

        let response = handle_events(
            named_scope("did:key:zOwn", "main"),
            hub,
            lsp_request(),
            false,
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a retiring worker declines rather than opening a stream"
        );
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("5"),
            "the client is told to come back"
        );
        assert_ne!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream"),
            "declining must NOT hand back a stream — that is the whole bug"
        );

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("update-pending"),
            "the client distinguishes this from a blip by the control \
             signal, and holds its reconnect on it: {text}"
        );
    }

    /// A live hub still opens a real stream. The refusal must be
    /// conditional — a worker that is not retiring has to keep serving
    /// diagnostics, or this "fix" would simply break the LSP.
    #[dialog_common::test]
    async fn it_opens_a_stream_while_not_retiring() {
        let hub = LspHub::new();

        let response = handle_events(
            named_scope("did:key:zOwn", "main"),
            hub,
            lsp_request(),
            false,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream"),
            "the normal path is still a live SSE subscription"
        );
    }

    /// The synchronous generation latch is sufficient to refuse a reconnect
    /// even before the asynchronous hub drain acquires its state lock.
    #[dialog_common::test]
    async fn it_refuses_a_stream_once_the_worker_is_retiring() {
        let hub = LspHub::new();

        let response = handle_events(
            named_scope("did:key:zOwn", "main"),
            hub,
            lsp_request(),
            true,
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("5")
        );
    }

    /// Shutting down ends the streams already open, which is what
    /// settles the in-flight fetch events keeping the worker alive.
    #[dialog_common::test]
    async fn it_closes_open_subscribers_on_shutdown() {
        let hub = LspHub::new();
        let mut receiver = hub.subscribe(&test_key()).await.expect("live hub");

        hub.shutdown().await;

        assert!(
            matches!(
                receiver.recv().await,
                Err(broadcast::error::RecvError::Closed)
            ),
            "an open receiver must see Closed so its SSE response body ends"
        );
    }

    fn named_scope(repo: &str, branch: &str) -> LspScope {
        LspScope::named(repo, branch)
    }

    fn test_key() -> LspSessionKey {
        LspSessionKey::new(named_scope("did:key:zOwn", "main"), "portal-test")
    }

    fn lsp_request() -> Request {
        Request::builder()
            .uri("/api/repository/did%3Akey%3AzOwn/branch/main/language-server")
            .header(
                tonk_worker_api::LSP_CLIENT_HEADER,
                "v1/p-11111111111111111111111111111111",
            )
            .body(Body::empty())
            .expect("LSP request")
    }

    #[dialog_common::test]
    fn it_uses_the_complete_canonical_nested_principal() {
        let scope = named_scope("did:key:zOwn", "main");
        let nested = "v1/p-11111111111111111111111111111111/p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sibling = "v1/p-11111111111111111111111111111111/p-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let request = |client: &str| {
            Request::builder()
                .uri("/api/repository/did%3Akey%3AzOwn/branch/main/language-server")
                .header(tonk_worker_api::LSP_CLIENT_HEADER, client)
                .body(Body::empty())
                .expect("LSP request")
        };

        let nested_key = session_key(scope.clone(), &request(nested)).expect("nested key");
        let sibling_key = session_key(scope.clone(), &request(sibling)).expect("sibling key");
        assert_ne!(nested_key, sibling_key);
        assert!(session_key(scope, &request("portal-forged")).is_err());

        let duplicate = Request::builder()
            .uri("/api/repository/did%3Akey%3AzOwn/branch/main/language-server")
            .header(tonk_worker_api::LSP_CLIENT_HEADER, nested)
            .header(tonk_worker_api::LSP_CLIENT_HEADER, sibling)
            .body(Body::empty())
            .expect("duplicate LSP request");
        assert!(
            session_key(named_scope("did:key:zOwn", "main"), &duplicate).is_err(),
            "a direct duplicate header must not select one authored identity",
        );
    }

    fn wire(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("JSON-RPC message")
    }

    /// Every method the embedded server currently understands is classified
    /// here at the HTTP trust boundary. A new LSP method is denied until its
    /// location-bearing fields have an explicit scope rule; serde's default
    /// "ignore unknown fields" behaviour must never become an authority leak.
    #[dialog_common::test]
    fn it_accepts_only_known_message_shapes_inside_the_route_scope() {
        let scope = named_scope("did:key:zOwn", "main");
        let own = "tonk-buffer:///did%3Akey%3AzOwn/main/scratch-1";

        for message in [
            json!({"jsonrpc":"2.0","id":1,"method":"textDocument/completion","params":{"textDocument":{"uri":own},"position":{"line":0,"character":0}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":own},"position":{"line":0,"character":0}}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":own,"languageId":"dialog-yaml","version":1,"text":""}}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":own,"version":2},"contentChanges":[{"text":"person:\n"}]}}),
            json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":own}}}),
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
            json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":99}}),
            json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}),
            json!({"jsonrpc":"2.0","method":"exit","params":null}),
        ] {
            assert!(
                scope_inbound(&wire(message), &scope).is_ok(),
                "known same-scope message must remain usable",
            );
        }

        for message in [
            json!({"jsonrpc":"2.0","id":4,"method":"workspace/symbol","params":{"query":"person"}}),
            json!({"jsonrpc":"2.0","method":"workspace/didChangeWorkspaceFolders","params":{"event":{"added":[{"uri":own,"name":"own"}],"removed":[]}}}),
            json!({"jsonrpc":"2.0","id":5,"method":"textDocument/didOpen","params":{"textDocument":{"uri":own,"languageId":"dialog-yaml","version":1,"text":""}}}),
            json!({"jsonrpc":"2.0","method":"shutdown","params":null}),
            json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":99,"uri":own}}),
            json!({"jsonrpc":"2.0","id":6,"result":null}),
            json!([{"jsonrpc":"2.0","method":"initialized","params":{}}]),
        ] {
            assert!(
                scope_inbound(&wire(message), &scope).is_err(),
                "unknown, ambiguous, or request/notification-mismatched message must fail closed",
            );
        }
    }

    /// Named and profile repositories share one worker process. A document URI
    /// is therefore authority-bearing input, not merely an editor identifier:
    /// every supported text-document operation must remain under the endpoint
    /// scope selected by the trusted portal.
    #[dialog_common::test]
    fn it_rejects_cross_repository_and_cross_profile_document_uris() {
        let scope = named_scope("did:key:zOwn", "main");
        for foreign in [
            "tonk-buffer:///did%3Akey%3AzOther/main/scratch-1",
            "tonk-buffer:///did%3Akey%3AzOwn/draft/scratch-1",
            "tonk-buffer:///profile:tonk/main/scratch-1",
        ] {
            for message in [
                json!({"jsonrpc":"2.0","id":1,"method":"textDocument/completion","params":{"textDocument":{"uri":foreign},"position":{"line":0,"character":0}}}),
                json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":foreign},"position":{"line":0,"character":0}}}),
                json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":foreign,"languageId":"dialog-yaml","version":1,"text":""}}}),
                json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":foreign,"version":2},"contentChanges":[{"text":""}]}}),
                json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":foreign}}}),
            ] {
                assert!(
                    scope_inbound(&wire(message), &scope).is_err(),
                    "foreign URI must be rejected: {foreign}",
                );
            }
        }
    }

    #[dialog_common::test]
    fn it_keeps_slash_branches_in_one_canonical_route_and_uri_segment() {
        let scope = named_scope("did:key:zOwn", "feat/artifact");
        assert_eq!(
            scope.endpoint_path(),
            "/api/repository/did%3Akey%3AzOwn/branch/feat%2Fartifact/language-server",
        );
        assert_eq!(
            scope.uri_root(),
            "tonk-buffer:///did%3Akey%3AzOwn/feat%2Fartifact/",
        );
        let profile = LspScope::profile("team/tonk", "feat/artifact");
        assert_eq!(
            profile.endpoint_path(),
            "/api/profile/team%2Ftonk/branch/feat%2Fartifact/language-server",
        );
        assert_eq!(
            profile.uri_root(),
            "tonk-buffer:///profile:team%2Ftonk/feat%2Fartifact/",
        );
        assert!(
            !named_scope("did\0key", "main")
                .matches_endpoint_path("/api/repository/did%00key/branch/main/language-server")
        );

        let own = json!({
            "jsonrpc":"2.0", "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":"tonk-buffer:///did%3Akey%3AzOwn/feat%2Fartifact/scratch-1",
                "languageId":"dialog-yaml", "version":1, "text":""
            }}
        });
        assert!(scope_inbound(&wire(own), &scope).is_ok());

        for foreign in [
            "tonk-buffer:///did%3Akey%3AzOwn/feat%2Fother/scratch-1",
            "tonk-buffer:///did%3Akey%3AzOther/feat%2Fartifact/scratch-1",
            "tonk-buffer:///did:key:zOwn/feat%2Fartifact/scratch-1",
        ] {
            let message = json!({
                "jsonrpc":"2.0", "method":"textDocument/didOpen",
                "params":{"textDocument":{
                    "uri":foreign, "languageId":"dialog-yaml", "version":1, "text":""
                }}
            });
            assert!(
                scope_inbound(&wire(message), &scope).is_err(),
                "cross-scope or aliased URI was accepted: {foreign}",
            );
        }
    }

    /// Axum matches on encoded path structure and decodes each extractor
    /// segment. Pin that seam together with our raw-path canonicality check:
    /// `%2F` must stay one branch segment, while alias spellings never become
    /// a second authorized route.
    #[dialog_common::test]
    async fn it_routes_only_canonical_encoded_scope_segments() {
        async fn authorize(
            Path((repo, branch)): Path<(String, String)>,
            request: Request,
        ) -> StatusCode {
            if LspScope::named(repo, branch).matches_endpoint_path(request.uri().path()) {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            }
        }

        let app = axum::Router::new().route(
            "/api/repository/{repo}/branch/{branch}/language-server",
            get(authorize),
        );
        for (path, expected) in [
            (
                "/api/repository/did%3Akey%3AzOwn/branch/feat%2Fartifact/language-server",
                StatusCode::OK,
            ),
            (
                "/api/repository/did:key:zOwn/branch/feat%2Fartifact/language-server",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/repository/did%3akey%3azOwn/branch/feat%2fartifact/language-server",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/repository/did%00key/branch/main/language-server",
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("route request"),
                )
                .await
                .expect("route response");
            assert_eq!(response.status(), expected, "route {path}");
        }
    }

    /// The client library initializes with the transport-wide root
    /// `tonk-buffer:///`. The scoped worker replaces that neutral default with
    /// its immutable route root, while rejecting an authored foreign root or
    /// workspace declaration rather than silently ignoring it.
    #[dialog_common::test]
    fn it_rewrites_only_the_neutral_initialize_root() {
        let scope = named_scope("did:key:zOwn", "main");
        let neutral = wire(json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"processId":null,"rootUri":"tonk-buffer:///","capabilities":{}}
        }));
        let scoped = scope_inbound(&neutral, &scope).expect("neutral root is scoped");
        let scoped: Value = serde_json::from_slice(&scoped).expect("sanitized JSON");
        assert_eq!(
            scoped.pointer("/params/rootUri").and_then(Value::as_str),
            Some("tonk-buffer:///did%3Akey%3AzOwn/main/"),
        );

        for params in [
            json!({"rootUri":"tonk-buffer:///did%3Akey%3AzOther/main/","capabilities":{}}),
            json!({"rootPath":"/tmp/foreign","capabilities":{}}),
            json!({"rootUri":"tonk-buffer:///","workspaceFolders":[{"uri":"tonk-buffer:///did%3Akey%3AzOther/main/","name":"other"}],"capabilities":{}}),
        ] {
            let message = wire(json!({
                "jsonrpc":"2.0", "id":2, "method":"initialize", "params":params
            }));
            assert!(
                scope_inbound(&message, &scope).is_err(),
                "foreign or ambiguous initialize scope must be denied",
            );
        }
    }

    /// One portal client must never receive another client's diagnostics, even
    /// when both edit the same repository. Repository scope also partitions a
    /// reused client id. The matching client still receives its own frame.
    #[dialog_common::test]
    async fn it_isolates_outbound_frames_by_scope_and_client() {
        let hub = LspHub::new();
        let own_scope = named_scope("did:key:zOwn", "main");
        let other_scope = named_scope("did:key:zOther", "main");
        let own = LspSessionKey::new(own_scope.clone(), "portal-a");
        let same_repo_other_client = LspSessionKey::new(own_scope.clone(), "portal-b");
        let other_repo_same_client = LspSessionKey::new(other_scope, "portal-a");

        let mut own_events = hub.subscribe(&own).await.expect("own stream");
        let mut other_client_events = hub
            .subscribe(&same_repo_other_client)
            .await
            .expect("second client stream");
        let mut other_repo_events = hub
            .subscribe(&other_repo_same_client)
            .await
            .expect("other repo stream");

        let did_open = wire(json!({
            "jsonrpc":"2.0", "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":"tonk-buffer:///did%3Akey%3AzOwn/main/scratch-1",
                "languageId":"dialog-yaml", "version":1, "text":"person:\n  name:"
            }}
        }));
        let sanitized = scope_inbound(&did_open, &own_scope).expect("same-scope open");
        hub.dispatch(&own, &sanitized, &NoEnv)
            .await
            .expect("same-scope dispatch");

        let frame = own_events
            .try_recv()
            .expect("matching client receives diagnostics");
        let frame: Value = serde_json::from_slice(&frame).expect("outbound JSON");
        assert_eq!(
            frame.pointer("/params/uri").and_then(Value::as_str),
            Some("tonk-buffer:///did%3Akey%3AzOwn/main/scratch-1"),
        );
        assert!(
            matches!(
                other_client_events.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "another client in the same repo must not observe the frame",
        );
        assert!(
            matches!(
                other_repo_events.try_recv(),
                Err(broadcast::error::TryRecvError::Empty)
            ),
            "another repo must not observe the frame",
        );
    }

    #[dialog_common::test]
    fn it_drops_any_outbound_method_or_uri_outside_the_session_scope() {
        let scope = named_scope("did:key:zOwn", "main");
        assert!(outbound_is_in_scope(
            "textDocument/publishDiagnostics",
            &json!({"uri":"tonk-buffer:///did%3Akey%3AzOwn/main/scratch-1","diagnostics":[]}),
            &scope,
        ));
        assert!(!outbound_is_in_scope(
            "textDocument/publishDiagnostics",
            &json!({"uri":"tonk-buffer:///did%3Akey%3AzOther/main/scratch-1","diagnostics":[]}),
            &scope,
        ));
        assert!(!outbound_is_in_scope(
            "window/showMessage",
            &json!({"message":"global broadcast"}),
            &scope,
        ));
    }
}
