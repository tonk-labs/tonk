//! The space writer's Durable Object: one per space and MCP session (named
//! by the space and the session's writer DID), holding the [`SpaceWriter`]
//! that signs every revision for the pair.
//!
//! The object keeps nothing durable. Its writer opens on the first request
//! with the session the front hands in, and lives as long as the object
//! stays in memory; an evicted object opens a new one on its next request,
//! as the same writer key.

use std::rc::Rc;

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_effects::storage::Directory;
use dialog_ucan_core::DelegationChain;
use futures_util::lock::Mutex;
use http::{Method, Request, StatusCode, header};
use tonk_worker::{SpaceWriter, WriterSession, writer_seed};
use worker::{DurableObject, Env, State, durable_object};
use zeroize::Zeroizing;

/// The binding the front reaches the space writers through.
const BINDING: &str = "SPACE_WRITER";

/// Headers the front hands the session in. The binding is internal, so
/// they reach the object only from the front.
const SEED: &str = "x-tonk-writer-seed";
const POWERLINE: &str = "x-tonk-writer-powerline";
const MCP_SESSION: &str = "x-tonk-writer-mcp-session";
const SPACE: &str = "x-tonk-writer-space";
const REMOTE: &str = "x-tonk-writer-remote";

/// The session a request comes in on, as the front decoded it.
pub(crate) struct Identity {
    /// The writer key's DID, which names the object.
    pub writer: String,
    /// The session key's seed, base64url.
    pub seed: String,
    /// The `agent space -> session` powerline container, base64url.
    pub powerline: String,
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
    request: worker::HttpRequest,
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
        (SEED, &identity.seed),
        (POWERLINE, &identity.powerline),
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

/// An opened writer, and the seed of its key.
type Opened = (Zeroizing<[u8; 32]>, Rc<SpaceWriter>);

/// One space and MCP session's writer.
#[durable_object]
pub struct SpaceWriterObject {
    /// The writer, once opened. Opening is serialized, so two first
    /// requests open one writer.
    writer: Mutex<Option<Opened>>,
}

impl DurableObject for SpaceWriterObject {
    fn new(_state: State, _env: Env) -> Self {
        Self {
            writer: Mutex::new(None),
        }
    }

    async fn fetch(&self, mut request: worker::Request) -> worker::Result<worker::Response> {
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
}

impl SpaceWriterObject {
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
        let seed: [u8; 32] = B64
            .decode(header(SEED)?)
            .ok()
            .and_then(|seed| seed.try_into().ok())
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "malformed seed".to_owned()))?;
        let seed = Zeroizing::new(seed);
        let mcp_session = header(MCP_SESSION)?;
        let key = writer_seed(&seed, &mcp_session);

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
        let powerline = B64
            .decode(header(POWERLINE)?)
            .ok()
            .and_then(|bytes| DelegationChain::try_from(bytes.as_slice()).ok())
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "malformed powerline".to_owned()))?;
        let space = header(SPACE)?
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, "malformed space".to_owned()))?;
        let remote = url::Url::parse(&header(REMOTE)?)
            .map_err(|_| (StatusCode::BAD_REQUEST, "malformed remote".to_owned()))?;
        let session = WriterSession {
            seed,
            powerline,
            mcp_session,
        };
        // In-memory storage, so the directory only has to be one no other
        // peer in this isolate uses.
        let directory = Directory::At(format!(
            "/space-writer/{:016x}",
            (js_sys::Math::random() * 1e16) as u64
        ));
        let writer = SpaceWriter::open(directory, &session, &space, &remote)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        let writer = Rc::new(writer);
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
