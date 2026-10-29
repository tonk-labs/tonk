//! The space writer's Durable Object: one per space and MCP session (named
//! by the space and the session's writer DID), holding the [`SpaceWriter`]
//! that signs every revision for the pair.
//!
//! The object keeps nothing durable. Its writer opens on the first request
//! with the session the front hands in, and lives as long as the object
//! stays in memory; an evicted object opens a new one on its next request,
//! as the same writer key.
//!
//! Wasm memory never shrinks, so a long-lived object sits at the high-water
//! mark of everything it has served. Once memory passes its high water
//! ([`HIGH_WATER`], or `SPACE_WRITER_HIGH_WATER_MIB`), the
//! object recycles itself from an alarm, when no request is in flight: it
//! drops its writer and resets (`ctx.abort()`), so the next request opens
//! a writer cold instead of near the 128 MB limit. See the memory budget in
//! `plans/2026-10-28-hosted-agent-peer.md`.

use std::cell::Cell;
use std::rc::Rc;

use axum::body::Body;
use futures_util::lock::Mutex;
use http::{Method, Request, StatusCode, header};
use js_sys::{Function, Reflect};
use tonk_worker::{SpaceWriter, writer_seed};
use wasm_bindgen::{JsCast as _, JsValue};
use worker::{DurableObject, Env, State, durable_object};
use zeroize::Zeroizing;

/// The binding the front reaches the space writers through.
const BINDING: &str = "SPACE_WRITER";

/// Headers the front hands the session in. The binding is internal, so
/// they reach the object only from the front.
const TOKEN: &str = "x-tonk-writer-token";
const MCP_SESSION: &str = "x-tonk-writer-mcp-session";
const SPACE: &str = "x-tonk-writer-space";
const REMOTE: &str = "x-tonk-writer-remote";

/// The session a request comes in on, as the front decoded it.
pub(crate) struct Identity {
    /// The writer key's DID, which names the object.
    pub writer: String,
    /// The agent session token, as it arrived. The object decodes it
    /// itself when it opens its writer.
    pub token: String,
    pub mcp_session: String,
    pub space: String,
    /// The access service's `/ucan/`.
    pub remote: String,
}

/// Hand `request` to the writer of the identity's space and writer key
/// (one per MCP session), at `path` relative to the space.
pub(crate) async fn forward(
    env: &Env,
    identity: &Identity,
    request: http::Request<Body>,
    path: &str,
) -> worker::Result<http::Response<Body>> {
    let (mut parts, body) = request.into_parts();
    let query = parts
        .uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    parts.uri = format!("https://space-writer.invalid/{path}{query}")
        .parse()
        .map_err(|error: http::uri::InvalidUri| worker::Error::RustError(error.to_string()))?;
    for (name, value) in [
        (TOKEN, &identity.token),
        (MCP_SESSION, &identity.mcp_session),
        (SPACE, &identity.space),
        (REMOTE, &identity.remote),
    ] {
        parts.headers.insert(
            name,
            value
                .parse()
                .map_err(|_| worker::Error::RustError(format!("{name} is not a header value")))?,
        );
    }
    let request = worker::Request::try_from(http::Request::from_parts(parts, body))?;
    let name = format!("{}/{}", identity.space, identity.writer);
    let mut response = env
        .durable_object(BINDING)?
        .id_from_name(&name)?
        .get_stub()?
        .fetch_with_request(request)
        .await?;
    let mut answer = http::Response::builder().status(response.status_code());
    if let Some(content_type) = response.headers().get("content-type")? {
        answer = answer.header(header::CONTENT_TYPE, content_type);
    }
    answer
        .body(Body::from(response.bytes().await?))
        .map_err(|error| worker::Error::RustError(error.to_string()))
}

/// The wasm memory, in bytes, above which an object recycles itself.
/// An object has 128 MB; this leaves room for the request that crosses it.
/// `SPACE_WRITER_HIGH_WATER_MIB` overrides it, to exercise recycling
/// without filling a space.
const HIGH_WATER: usize = 96 * 1024 * 1024;

/// Reset the object `state` is the `DurableObjectState` of: its
/// in-memory state is dropped, and every request in flight in it fails.
///
/// workers-rs 0.8.5 has no binding for `ctx.abort()`, so it is looked up
/// on the state. It is called from a microtask, with no wasm on the stack:
/// called from inside wasm, the reset unwinds the module mid-call and
/// leaves its async executor stuck, so the next request in the isolate
/// hangs.
fn abort(state: &JsValue, reason: &str) -> Result<(), JsValue> {
    let abort: Function = Reflect::get(state, &"abort".into())?.dyn_into()?;
    let abort = abort.bind1(state, &reason.into());
    let global = js_sys::global();
    let queue: Function = Reflect::get(&global, &"queueMicrotask".into())?.dyn_into()?;
    queue.call1(&global, &abort)?;
    Ok(())
}

/// The wasm memory of this isolate, in bytes. It never shrinks.
fn memory() -> usize {
    core::arch::wasm32::memory_size::<0>() * 64 * 1024
}

/// An opened writer, and the seed of its key.
type Opened = (Zeroizing<[u8; 32]>, Rc<SpaceWriter>);

/// One space and MCP session's writer.
#[durable_object]
pub struct SpaceWriterObject {
    state: State,
    /// The same state, to reset the object with ([`abort`]).
    raw: JsValue,
    /// The writer, once opened. Opening is serialized, so two first
    /// requests open one writer.
    writer: Mutex<Option<Opened>>,
    /// The memory when the writer opened. An isolate holds more than one
    /// object, and a reset may not give its memory back: an object that
    /// opened above its high water recycles only once memory grows past
    /// where it opened, rather than on every request.
    opened_at: Cell<usize>,
    high_water: usize,
    /// Requests being answered. The object recycles only at none.
    in_flight: Cell<usize>,
    /// A recycle alarm is set.
    recycling: Cell<bool>,
}

impl DurableObject for SpaceWriterObject {
    fn new(state: State, env: Env) -> Self {
        let high_water = env
            .var("SPACE_WRITER_HIGH_WATER_MIB")
            .ok()
            .and_then(|mib| mib.to_string().parse::<usize>().ok())
            .map_or(HIGH_WATER, |mib| mib * 1024 * 1024);
        let raw = state._inner();
        Self {
            raw: JsValue::from(raw.clone()),
            state: State::from(raw),
            writer: Mutex::new(None),
            opened_at: Cell::new(0),
            high_water,
            in_flight: Cell::new(0),
            recycling: Cell::new(false),
        }
    }

    async fn fetch(&self, request: worker::Request) -> worker::Result<worker::Response> {
        self.in_flight.set(self.in_flight.get() + 1);
        let response = self.answer(request).await;
        self.in_flight.set(self.in_flight.get() - 1);
        if self.over_high_water() && !self.recycling.replace(true) {
            // From an alarm, so the reset fails no request: this one is
            // answered first.
            if let Err(error) = self.state.storage().set_alarm(0).await {
                self.recycling.set(false);
                worker::console_error!("space writer: no recycle alarm: {error}");
            }
        }
        response
    }

    async fn alarm(&self) -> worker::Result<worker::Response> {
        self.recycling.set(false);
        if !self.over_high_water() {
            // A fresh object, as an alarm retried after a reset finds.
            return worker::Response::empty();
        }
        // Opening holds the writer's lock, and a write holds its own.
        let Some(mut opened) = self.writer.try_lock() else {
            return worker::Response::empty();
        };
        let idle = self.in_flight.get() == 0
            && opened
                .as_ref()
                .is_some_and(|(_, writer)| !writer.holds_write());
        if !idle {
            // The next request to finish sets the alarm again.
            return worker::Response::empty();
        }
        // Nothing awaits from the check to the reset, so no request starts
        // in between.
        drop(opened.take());
        worker::console_log!(
            "space writer: recycling at {} MiB",
            memory() / (1024 * 1024)
        );
        if let Err(error) = abort(&self.raw, "space writer recycled at high water") {
            worker::console_error!("space writer: no reset: {error:?}");
        }
        worker::Response::empty()
    }
}

impl SpaceWriterObject {
    /// Whether the writer is open and memory is past the high water and
    /// past where it opened.
    fn over_high_water(&self) -> bool {
        let opened_at = self.opened_at.get();
        let now = memory();
        opened_at != 0 && now > self.high_water && now > opened_at
    }

    async fn answer(&self, mut request: worker::Request) -> worker::Result<worker::Response> {
        let writer = match self.writer(&request).await {
            Ok(writer) => writer,
            Err((status, message)) => return failure(status, &message),
        };
        let url = request.url()?;
        let path = url.path().trim_start_matches('/').to_owned();
        let method = request.method();
        let content_type = request
            .headers()
            .get("content-type")?
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        let body = Body::from(request.bytes().await?);
        // The Host Worker API of a space, onto the routes of its branch.
        let response = match (method, path.as_str()) {
            (worker::Method::Post, "api/query") => {
                let request = branch(Method::POST, "/evaluate?transact=false", "text/plain", body);
                writer.read(request).await
            }
            (worker::Method::Post, "api/evaluate") => {
                let request = branch(Method::POST, "/evaluate?transact=true", "text/plain", body);
                writer.write(request).await
            }
            (worker::Method::Put, "") => {
                writer
                    .write(branch(Method::POST, "/blob", &content_type, body))
                    .await
            }
            (worker::Method::Get, path) if path.starts_with("asset/") => {
                let entity = &path["asset/".len()..];
                let request = branch(Method::GET, &format!("/blob/{entity}"), "", Body::empty());
                writer.read(request).await
            }
            _ => return failure(StatusCode::NOT_FOUND, "no such route"),
        };
        worker::Response::try_from(response)
    }

    /// The writer, opened for the session `request` carries on its first
    /// request. Every later request must carry the same session: the
    /// object is named by the writer's DID, so another writer key here
    /// means the name was reached some other way, and is refused.
    async fn writer(
        &self,
        request: &worker::Request,
    ) -> Result<Rc<SpaceWriter>, (StatusCode, String)> {
        let header = |name: &str| {
            request
                .headers()
                .get(name)
                .ok()
                .flatten()
                .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("missing {name}")))
        };
        let token = header(TOKEN)?;
        let mcp_session = header(MCP_SESSION)?;
        let space: dialog_varsig::Did = header(SPACE)?
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, "malformed space".to_owned()))?;
        let verified = tonk_worker::mcp::verify(&token)
            .await
            .map_err(|error| (StatusCode::UNAUTHORIZED, error))?;
        let session = verified.writer_session(&space, &mcp_session);
        let key = writer_seed(&session.seed, &mcp_session);

        let mut opened = self.writer.lock().await;
        if let Some((held, writer)) = opened.as_ref() {
            if **held != *key {
                return Err((
                    StatusCode::CONFLICT,
                    "this writer serves another session".to_owned(),
                ));
            }
            return Ok(writer.clone());
        }
        let remote = url::Url::parse(&header(REMOTE)?)
            .map_err(|_| (StatusCode::BAD_REQUEST, "malformed remote".to_owned()))?;
        // In-memory storage, so the directory only has to be one no other
        // peer in this isolate uses.
        let root = format!(
            "/space-writer/{:016x}",
            (js_sys::Math::random() * 1e16) as u64
        );
        let writer = SpaceWriter::open(&root, &session, &space, &remote)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        let writer = Rc::new(writer);
        self.opened_at.set(memory());
        worker::console_log!("space writer: opened at {} MiB", memory() / (1024 * 1024));
        *opened = Some((key, writer.clone()));
        Ok(writer)
    }
}

/// A request to `path` on the writer's branch.
fn branch(method: Method, path: &str, content_type: &str, body: Body) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(path);
    if !content_type.is_empty() {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    request.body(body).expect("static request")
}

fn failure(status: StatusCode, message: &str) -> worker::Result<worker::Response> {
    let mut response = worker::Response::from_json(&serde_json::json!({ "error": message }))?;
    response = response.with_status(status.as_u16());
    Ok(response)
}
