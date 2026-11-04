//! `/mcp`: the hosted agent's MCP endpoint, Streamable HTTP without SSE.
//!
//! A request carries an agent session token (`tonk-identity`'s
//! [`AgentSessionToken`]) as its Bearer: the `agent space -> session`
//! powerline and the session's seed, nothing else. The front decodes and
//! verifies it, and answers JSON-RPC over `POST`: `initialize`, `ping`,
//! `tools/list` and `tools/call`. A missing or bad token is answered
//! `401` with the protected resource metadata the OAuth client discovers
//! the authorization server from.
//!
//! `initialize` mints `Mcp-Session-Id`: a nonce and a MAC over it and the
//! token's hash, under a secret only the host holds. Every later request
//! must send an id that verifies for its own token, so a client can
//! neither pick an id nor reuse one under another token. The id names the
//! writer key (see [`super::writer_seed`]), so each MCP session writes as
//! its own DID.
//!
//! Spaces and access:
//!
//! - the agent space itself, through the powerline: read and write;
//! - each space the agent space holds a grant for: the approval retains
//!   each chain `space -> ... -> agent space` in the agent space, with a
//!   directory entry naming the space (see [`super::access`]). The front
//!   asks the agent space's own writer, which stays warm, to classify
//!   them: readable when a chain in force reaches `/use/get/...`,
//!   writable when one also reaches `/use/put/...`. `tonk_evaluate` is
//!   refused for a space without put grants, and the access service
//!   refuses a put such a chain carries anyway. The writer of a granted
//!   space pulls the same chains from the agent space to act with.
//!
//! What is not checked here: revocation. The access service checks every
//! chain an invocation carries against its revocation index, and every
//! read pulls and every write pulls and pushes through it, so a revoked
//! session or grant fails at its next sync. A read answered from the
//! writer's local state without a pull (a read while a write holds the
//! lock) is not re-checked.
//!
//! Nothing here knows where the writers live: [`Writers`] is a Durable
//! Object namespace in the Host Worker and a map of [`super::SpaceWriter`]s
//! in native tests.
//!
//! Each `tools/call` gets a trace id, sent to the writers it reaches and
//! answered in [`TRACE_HEADER`], and its steps' times are answered in
//! `Server-Timing`. The endpoint hands one [`ToolEvent`] per call to its
//! sink (see [`super::telemetry`]).

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_varsig::Did;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tonk_identity::agent::{AgentSessionToken, decode_agent_session_token};

use super::WriterSession;
pub use super::access::{AgentAccess, SpaceAccess};
use super::telemetry::{self, Outcome, Steps, ToolEvent};
pub use super::telemetry::{SERVER_TIMING, TRACE_HEADER};

#[path = "../../../../tonk-cli/src/output.rs"]
#[allow(dead_code)]
mod output;

mod undo;
pub use undo::{CHANGE_META, UNDO, UNDO_ROUTE};

#[path = "../../../../tonk-cli/src/guide.rs"]
#[allow(dead_code)]
mod guide;

/// The shared tool catalog every Tonk MCP host describes its tools from.
const CATALOG: &str = include_str!("../../../../../mcp/tools.json");

/// Tools this endpoint offers, in catalog order.
const TOOLS: &[&str] = &[
    "tonk_list_spaces",
    "tonk_query",
    "tonk_evaluate",
    "tonk_guide",
];

/// Tools that act on one space take its subject.
const SCOPED: &[&str] = &["tonk_query", "tonk_evaluate"];

/// The MCP revision answered when a client asks for one this endpoint
/// does not know.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// MCP revisions this endpoint speaks. None of them needs SSE from a
/// server that answers every request with JSON.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The `Mcp-Session-Id` header.
pub const MCP_SESSION_HEADER: &str = "mcp-session-id";

/// Context for the MAC key binding session ids to tokens.
const SESSION_MAC_CONTEXT: &str = "tonk 2026-10-29 hosted agent mcp session id v1";

/// Bytes of nonce and of MAC in a session id.
const NONCE_BYTES: usize = 16;
const MAC_BYTES: usize = 16;

/// The largest JSON-RPC request accepted, in bytes.
const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// The agent space writer's route answering [`AgentAccess`] as JSON.
pub const ACCESS_ROUTE: &str = "/api/access";

/// Header on a request to a granted space's writer: the CIDs of the
/// grants the front found for the space, comma separated, so the writer
/// pulls the agent space again when one is new to it
/// ([`super::SpaceWriter::admit`]).
pub const GRANTS_HEADER: &str = "x-tonk-space-grants";

/// Header on a request to a granted space's writer: the `home.address`
/// its grant carries, when it carries one.
pub const ADDRESS_HEADER: &str = "x-tonk-space-address";

/// Where a writer mounts a space: `named`, the `home.address` the space's
/// grant carries in its signed meta, as the CLI and the web app sync a
/// space with its recorded home; `configured`, the access service the
/// host is configured with, when the grant names none or names an
/// address that is not usable (`https`, or `http` on a loopback host).
///
/// The address comes from the grant, not the agent space's directory:
/// the agent can write its directory, but not a delegation's signed meta.
pub fn mount_address(named: Option<&str>, configured: &str) -> String {
    let usable = |address: &str| {
        url::Url::parse(address).is_ok_and(|url| match url.scheme() {
            "https" => url.host().is_some(),
            "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
            _ => false,
        })
    };
    match named {
        Some(named) if usable(named) => named.to_owned(),
        _ => configured.to_owned(),
    }
}

/// A token that decoded and verified: the agent space's powerline to the
/// session whose seed it carries. Which other spaces it reaches is the
/// agent space's to say ([`AgentAccess`]).
pub struct Verified {
    token: AgentSessionToken,
    hash: [u8; 32],
}

impl Verified {
    /// The decoded token.
    pub fn token(&self) -> &AgentSessionToken {
        &self.token
    }

    /// The agent space.
    pub fn agent(&self) -> &Did {
        self.token.agent()
    }

    /// What the writer of a space for the MCP session `mcp_session` acts
    /// with: the session's seed and powerline, and where the agent space
    /// syncs, to pull its grants from.
    pub fn writer_session(&self, mcp_session: &str) -> WriterSession {
        WriterSession {
            seed: self.token.seed().clone(),
            powerline: self.token.powerline().clone(),
            mcp_session: mcp_session.to_owned(),
            home: self.token.home().clone(),
        }
    }
}

/// Decode `bearer` and check it. Every refusal is a reason to answer
/// `401`.
///
/// [`decode_agent_session_token`] checks the token's own delegations:
/// the seed's DID is the outer delegation's issuer and audience, and the
/// powerline from the agent space is addressed to it and signed. Grants
/// the token may still carry from an older approval are ignored: access
/// to other spaces is read from the agent space.
pub async fn verify(bearer: &str) -> Result<Verified, String> {
    let token = decode_agent_session_token(bearer)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Verified {
        hash: *blake3::hash(bearer.trim().as_bytes()).as_bytes(),
        token,
    })
}

/// Where the front sends a space's reads and writes.
#[allow(async_fn_in_trait)]
pub trait Writers {
    /// Serve `request` on the writer of `space` for `mcp_session`.
    ///
    /// `request` is one of the writer's routes relative to the space:
    /// `POST /api/query` (a read), `POST /api/evaluate` (a write),
    /// `POST` [`UNDO_ROUTE`] (a write undoing one change), or `GET`
    /// [`ACCESS_ROUTE`] (to the agent space's writer). A request to
    /// another space carries [`GRANTS_HEADER`] and, when the directory
    /// names one, [`ADDRESS_HEADER`], the remote its writer mounts.
    /// `bearer` is the token as it arrived, for writers that decode it
    /// themselves.
    async fn call(
        &self,
        verified: &Verified,
        bearer: &str,
        mcp_session: &str,
        space: &Did,
        request: Request<Body>,
    ) -> Response<Body>;
}

/// The MAC key session ids are bound under, derived from the host's
/// secret.
#[derive(Clone)]
pub struct SessionKey([u8; 32]);

impl SessionKey {
    /// Derive the key from `secret`, any string with enough entropy.
    pub fn derive(secret: &str) -> Self {
        Self(blake3::derive_key(SESSION_MAC_CONTEXT, secret.as_bytes()))
    }

    fn mac(&self, token_hash: &[u8; 32], nonce: &[u8]) -> [u8; MAC_BYTES] {
        let mut hasher = blake3::Hasher::new_keyed(&self.0);
        hasher.update(token_hash);
        hasher.update(nonce);
        let mut mac = [0u8; MAC_BYTES];
        mac.copy_from_slice(&hasher.finalize().as_bytes()[..MAC_BYTES]);
        mac
    }

    /// A fresh session id for the token whose hash is `token_hash`.
    pub fn mint(&self, token_hash: &[u8; 32]) -> String {
        let nonce: [u8; NONCE_BYTES] = rand::random();
        let mut id = nonce.to_vec();
        id.extend_from_slice(&self.mac(token_hash, &nonce));
        B64.encode(id)
    }

    /// Whether `id` was minted by [`mint`](Self::mint) for this token.
    pub fn verify(&self, token_hash: &[u8; 32], id: &str) -> bool {
        let Ok(bytes) = B64.decode(id) else {
            return false;
        };
        if bytes.len() != NONCE_BYTES + MAC_BYTES {
            return false;
        }
        let (nonce, mac) = bytes.split_at(NONCE_BYTES);
        let expected = self.mac(token_hash, nonce);
        // Constant time, so a client cannot find a MAC byte by byte.
        mac.iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}

/// The endpoint: what serves `/mcp` given its origin, a session key and
/// the writers.
pub struct McpEndpoint<'a, W> {
    /// The origin `/mcp` is served at, such as `https://tonk.foundation`.
    pub origin: &'a str,
    /// The key session ids are bound under.
    pub key: &'a SessionKey,
    /// Where tool calls on a space go.
    pub writers: &'a W,
    /// The key view tickets are bound under.
    pub tickets: &'a super::view::TicketKey,
    /// Where each tool call's [`ToolEvent`] goes: [`telemetry::log`] in
    /// the Host Worker.
    pub events: telemetry::Sink<'a>,
}

/// What one tool call records as it runs.
struct Call {
    trace: String,
    /// When the request arrived ([`tonk_common::diagnostics::now_ms`]).
    began: f64,
    steps: Steps,
    /// Set where a call is refused, rejected or fails; otherwise read
    /// from the answer's `isError`.
    outcome: Option<(Outcome, String)>,
    space: Option<Did>,
}

impl Call {
    fn end(&mut self, outcome: Outcome, kind: &str) {
        self.outcome = Some((outcome, kind.to_owned()));
    }
}

/// How a writer's failed answer of `status` ends a call.
fn writer_outcome(status: StatusCode) -> Outcome {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => Outcome::Invalid,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Outcome::Refused,
        _ => Outcome::Error,
    }
}

/// A writer's error code as an event's reason: one in the writer's own
/// shape, such as `WRITE_NOT_LANDED`, lowercased; none for anything else.
fn writer_code(code: &str) -> Option<String> {
    let shaped = !code.is_empty()
        && code.len() <= 40
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_');
    shaped.then(|| code.to_ascii_lowercase())
}

/// The name a tool is recorded under: its own when it could be one this
/// endpoint serves, `other` otherwise, so a client cannot write arbitrary
/// text into the logs.
fn recorded_tool(name: &str) -> String {
    let plausible = !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    if plausible {
        name.to_owned()
    } else {
        "other".to_owned()
    }
}

impl<W: Writers> McpEndpoint<'_, W> {
    /// Answer one request to `/mcp`.
    pub async fn serve(&self, request: Request<Body>) -> Response<Body> {
        let began = tonk_common::diagnostics::now_ms();
        let mut steps = Steps::start();
        let bearer = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value
                    .strip_prefix("Bearer ")
                    .or_else(|| value.strip_prefix("bearer "))
            })
            .map(|value| value.trim().to_owned());
        let Some(bearer) = bearer.filter(|bearer| !bearer.is_empty()) else {
            return self.unauthorized("missing bearer token");
        };
        let verified = match verify(&bearer).await {
            Ok(verified) => verified,
            Err(error) => return self.unauthorized(&error),
        };
        steps.lap("verify");
        match *request.method() {
            Method::POST => {}
            // No server-initiated stream, and sessions end on their own.
            Method::GET | Method::DELETE => {
                return plain(StatusCode::METHOD_NOT_ALLOWED, "only POST is served");
            }
            _ => return plain(StatusCode::METHOD_NOT_ALLOWED, "only POST is served"),
        }
        let session = request
            .headers()
            .get(MCP_SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = match request.into_body().collect().await {
            Ok(body) => body.to_bytes(),
            Err(error) => return plain(StatusCode::BAD_REQUEST, &error.to_string()),
        };
        if body.len() > MAX_REQUEST_BYTES {
            return plain(StatusCode::PAYLOAD_TOO_LARGE, "request too large");
        }
        let message: Value = match serde_json::from_slice(&body) {
            Ok(message) => message,
            Err(error) => {
                return rpc(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": { "code": -32700, "message": error.to_string() },
                    }),
                );
            }
        };
        if !message.is_object() {
            return rpc(
                StatusCode::BAD_REQUEST,
                json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32600, "message": "batches are not served" },
                }),
            );
        }
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let id = message.get("id").cloned();

        if method == "initialize" {
            let Some(id) = id else {
                return plain(StatusCode::BAD_REQUEST, "initialize needs an id");
            };
            let session = self.key.mint(&verified.hash);
            let asked = message["params"]["protocolVersion"]
                .as_str()
                .unwrap_or_default();
            let version = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSION
            };
            let mut response = rpc(
                StatusCode::OK,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": version,
                        "capabilities": {
                            "tools": { "listChanged": false },
                            "resources": {},
                            "extensions": { "io.modelcontextprotocol/ui": {} },
                        },
                        "serverInfo": {
                            "name": "tonk",
                            "title": "Tonk",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions": instructions(self.origin),
                    },
                }),
            );
            if let Ok(value) = HeaderValue::from_str(&session) {
                response.headers_mut().insert(MCP_SESSION_HEADER, value);
            }
            return response;
        }

        let Some(session) = session else {
            return plain(StatusCode::BAD_REQUEST, "missing Mcp-Session-Id");
        };
        if !self.key.verify(&verified.hash, &session) {
            // The spec's answer to a session the server does not hold:
            // the client starts a new one with `initialize`.
            return plain(StatusCode::NOT_FOUND, "unknown MCP session");
        }
        let Some(id) = id else {
            // A notification or a response: nothing to answer.
            return accepted();
        };
        let result = match method.as_str() {
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut tools = tools();
                tools.push(undo::tool());
                tools.extend(super::view::tools());
                tools.extend(super::join::tools());
                tools.extend(super::propose::tools());
                Ok(json!({ "tools": tools }))
            }
            "resources/list" => {
                let mut resources = guide_resources();
                resources.extend(super::view::resources());
                resources.extend(super::join::resources());
                resources.extend(super::card::resources());
                Ok(json!({ "resources": resources }))
            }
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => {
                let uri = message["params"]["uri"].as_str().unwrap_or_default();
                read_guide(uri)
                    .or_else(|| super::view::read_resource(uri, self.origin))
                    .or_else(|| super::join::read_resource(uri, self.origin))
                    .or_else(|| super::card::read_resource(uri, self.origin))
                    .ok_or_else(
                        || json!({ "code": -32602, "message": format!("no resource {uri}") }),
                    )
            }
            "tools/call" => {
                let params = &message["params"];
                steps.lap("request");
                let mut call = Call {
                    trace: telemetry::trace(),
                    began,
                    steps,
                    outcome: None,
                    space: None,
                };
                let result = self
                    .call_tool(&verified, &bearer, &session, params, &mut call)
                    .await;
                let tool = recorded_tool(params["name"].as_str().unwrap_or_default());
                let answer = rpc(
                    StatusCode::OK,
                    json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                );
                return self.record(answer, call, tool, &verified, &session, &result);
            }
            _ => Err(json!({ "code": -32601, "message": format!("no method {method}") })),
        };
        rpc(
            StatusCode::OK,
            match result {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            },
        )
    }

    /// Send `call`'s event to the sink, and answer `answer` with its trace
    /// id and steps.
    fn record(
        &self,
        mut answer: Response<Body>,
        mut call: Call,
        tool: String,
        verified: &Verified,
        session: &str,
        result: &Value,
    ) -> Response<Body> {
        call.steps.lap("answer");
        let ms = tonk_common::diagnostics::now_ms() - call.began;
        let (outcome, kind) = match call.outcome {
            Some((outcome, kind)) => (outcome, Some(kind)),
            None if result["isError"] == json!(true) => (Outcome::Error, None),
            None => (Outcome::Ok, None),
        };
        let event = ToolEvent {
            event: "mcp.tool",
            trace: call.trace,
            tool,
            outcome,
            kind,
            ms: telemetry::round(ms),
            steps: telemetry::step_map(&call.steps),
            space: call.space.as_ref().map(telemetry::short_did),
            agent: telemetry::short_did(verified.agent()),
            session: telemetry::short(session),
        };
        let headers = answer.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&event.trace) {
            headers.insert(TRACE_HEADER, value);
        }
        let mut timing = call.steps;
        timing.push("total", ms);
        timing.insert(headers);
        (self.events)(&event);
        answer
    }

    async fn call_tool(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        params: &Value,
        call: &mut Call,
    ) -> Value {
        let name = params["name"].as_str().unwrap_or_default();
        let arguments = &params["arguments"];
        match name {
            "tonk_list_spaces" => match self.access(verified, bearer, session, call).await {
                Ok(access) => tool_result(json!({ "spaces": list_spaces(&access) }), false),
                Err(error) => tool_result(json!(error), true),
            },
            "tonk_guide" => {
                let topic = arguments["topic"].as_str().unwrap_or("");
                let body = if topic == "context" || topic == "agent" {
                    Some(HOSTED)
                } else {
                    guide::topic(topic)
                };
                match (topic, body) {
                    ("", _) => tool_result(
                        json!({
                            "topics": guide::TOPICS,
                            "usage": "Call tonk_guide with a topic.",
                        }),
                        false,
                    ),
                    (_, Some(body)) => tool_result(json!(body), false),
                    (_, None) => {
                        call.end(Outcome::Invalid, "unknown_topic");
                        tool_result(json!("Unknown guide topic."), true)
                    }
                }
            }
            "tonk_query" | "tonk_evaluate" => {
                let write = name == "tonk_evaluate";
                let Some(space) = arguments["space"].as_str() else {
                    call.end(Outcome::Invalid, "no_space");
                    return tool_result(
                        json!("`space` is required: an exact subject from tonk_list_spaces."),
                        true,
                    );
                };
                let Some(document) = arguments["document"]
                    .as_str()
                    .filter(|document| !document.is_empty())
                else {
                    call.end(Outcome::Invalid, "no_document");
                    return tool_result(json!("`document` is required."), true);
                };
                let Ok(space) = space.parse::<Did>() else {
                    call.end(Outcome::Invalid, "bad_space");
                    return tool_result(json!(format!("`{space}` is not a space DID.")), true);
                };
                call.space = Some(space.clone());
                if write {
                    let confirm = arguments["confirm"].as_bool().unwrap_or(false);
                    return self
                        .evaluate(verified, bearer, session, &space, document, confirm, call)
                        .await;
                }
                // The agent space is reached through the powerline alone;
                // any other space through what the agent space holds.
                let granted = if &space == verified.agent() {
                    None
                } else {
                    let access = match self.access(verified, bearer, session, call).await {
                        Ok(access) => access,
                        Err(error) => return tool_result(json!(error), true),
                    };
                    if let Some((kind, refusal)) = refusal(access.get(&space), &space, write) {
                        call.end(Outcome::Refused, kind);
                        return tool_result(json!(refusal), true);
                    }
                    access.get(&space).cloned()
                };
                let path = if write {
                    "/api/evaluate".to_owned()
                } else {
                    // A page of a large read: the writer caps reads and
                    // answers the next cursor.
                    let mut page = url::form_urlencoded::Serializer::new(String::new());
                    let limit = arguments["limit"].as_u64().unwrap_or(DEFAULT_LIMIT);
                    page.append_pair("limit", &limit.to_string());
                    if let Some(cursor) = arguments["cursor"].as_str() {
                        page.append_pair("cursor", cursor);
                    }
                    let page = page.finish();
                    if page.is_empty() {
                        "/api/query".to_owned()
                    } else {
                        format!("/api/query?{page}")
                    }
                };
                let mut request = Request::builder()
                    .method(Method::POST)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .header(TRACE_HEADER, &call.trace);
                if let Some(granted) = &granted {
                    request = request.header(GRANTS_HEADER, granted.grants.join(","));
                    if let Some(address) = &granted.address {
                        request = request.header(ADDRESS_HEADER, address);
                    }
                }
                let Ok(request) = request.body(Body::from(document.to_owned())) else {
                    call.end(Outcome::Error, "bad_directory_entry");
                    return tool_result(json!("The space's directory entry is malformed."), true);
                };
                call.steps.lap("prepare");
                let response = self
                    .writers
                    .call(verified, bearer, session, &space, request)
                    .await;
                call.steps.lap("writer");
                call.steps
                    .extend_within("writer", Steps::of(response.headers()));
                let status = response.status();
                let body = response
                    .into_body()
                    .collect()
                    .await
                    .map(|body| body.to_bytes())
                    .unwrap_or_default();
                let text = if status.is_success() {
                    notation(&body, write)
                } else {
                    None
                };
                let body = match serde_json::from_slice::<Value>(&body) {
                    Ok(value) => value,
                    Err(_) => json!(String::from_utf8_lossy(&body)),
                };
                if !status.is_success() {
                    let kind = body["code"]
                        .as_str()
                        .and_then(writer_code)
                        .unwrap_or_else(|| format!("http_{}", status.as_u16()));
                    call.end(writer_outcome(status), &kind);
                }
                // What a card needs to undo the write, in `_meta`, which
                // hosts hand the app and not the model.
                let change = (write && status.is_success())
                    .then(|| undo::change_meta(&space, &body))
                    .flatten();
                let mut answer = tool_result(without_revisions(body), !status.is_success());
                // The model reads notation; programs read the JSON beside it.
                if let Some(text) = text {
                    answer["content"] = json!([{ "type": "text", "text": text }]);
                }
                if let Some(change) = change {
                    answer["_meta"] = json!({ CHANGE_META: change });
                }
                answer
            }
            UNDO => self.undo(verified, bearer, session, arguments, call).await,
            super::propose::APPLY | super::propose::DISMISS => {
                self.answer_proposal(name, arguments, verified, bearer, session, call)
                    .await
            }
            super::join::JOIN | super::join::ACCEPT => {
                super::join::call(name, arguments, verified, self.writers, bearer, session)
                    .await
                    .unwrap_or_else(|| tool_result(json!("Tool unavailable."), true))
            }
            _ => match super::view::call(
                name,
                arguments,
                verified,
                self.writers,
                bearer,
                session,
                self.origin,
                self.tickets,
            )
            .await
            {
                Some(result) => result,
                None => tool_result(json!("Tool unavailable."), true),
            },
        }
    }

    /// `tonk_evaluate`: write `document` to `space`, or with `confirm`,
    /// propose it and write nothing (see [`super::propose`]). The result
    /// carries the card the host draws: the receipt, the ask, or a tell.
    #[allow(clippy::too_many_arguments)]
    async fn evaluate(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        space: &Did,
        document: &str,
        confirm: bool,
        call: &mut Call,
    ) -> Value {
        let reached = match self.reach(verified, bearer, session, space, call).await {
            Ok(reached) => reached,
            Err(refused) => return refused,
        };
        let route = if confirm {
            super::propose::PROPOSE_ROUTE
        } else {
            "/api/evaluate"
        };
        let Some(request) = space_request(
            route,
            reached.granted.as_ref(),
            "text/plain",
            document,
            &call.trace,
        ) else {
            call.end(Outcome::Error, "bad_directory_entry");
            return tool_result(json!("The space's directory entry is malformed."), true);
        };
        let (status, body) = self
            .send(verified, bearer, session, space, request, call)
            .await;
        let context = &reached.context;
        if !status.is_success() {
            let value = parsed(&body);
            writer_failed(call, status, &value);
            let reason = reason(&value);
            let mut answer = tool_result(without_revisions(value), true);
            if confirm {
                answer["content"] = json!([{
                    "type": "text",
                    "text": format!("Nothing was proposed or written: {reason}"),
                }]);
                answer["structuredContent"] = json!({
                    "card": super::card::Card::tell(
                        Some(context.place.clone()),
                        context.role,
                        "not proposed",
                        format!("I couldn’t prepare this change for {}.", context.place.label()),
                        context.agent.clone(),
                    )
                    .fine(reason)
                    .to_value(),
                });
            } else {
                answer["structuredContent"] =
                    json!({ "card": super::propose::not_saved(context, &reason).to_value() });
            }
            return answer;
        }
        if !confirm {
            let text = notation(&body, true);
            let evaluation = parsed(&body);
            // What a card needs to undo the write, in `_meta`, which
            // hosts hand the app and not the model.
            let change = undo::change_meta(space, &evaluation);
            let mut evaluation = without_revisions(evaluation);
            let card = super::propose::receipt(context, &evaluation).to_value();
            if let Some(fields) = evaluation.as_object_mut() {
                fields.insert("card".into(), card);
            }
            let mut answer = tool_result(evaluation, false);
            if let Some(text) = text {
                answer["content"] = json!([{ "type": "text", "text": text }]);
            }
            if let Some(change) = change {
                answer["_meta"] = json!({ CHANGE_META: change });
            }
            return answer;
        }
        let held = parsed(&body);
        let id = held["proposal"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let evaluation = without_revisions(held["evaluation"].clone());
        let ask = super::propose::ask(context, &id, &evaluation);
        let dry_run = serde_json::to_vec(&evaluation)
            .ok()
            .and_then(|bytes| notation(&bytes, false))
            .unwrap_or_default();
        let text = format!(
            "Asked the person to confirm in a card: {}. Nothing is written unless they apply it \
             there; the card tells you when they answer. Do not write it again with \
             tonk_evaluate meanwhile. What it would change, as a dry run against {} now:\n{dry_run}",
            ask.say,
            context.place.label(),
        );
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": {
                "proposal": held["proposal"],
                "evaluation": evaluation,
                "card": ask.to_value(),
            },
            "isError": false,
        })
    }

    /// `tonk_proposal_apply` and `tonk_proposal_dismiss`, which the card
    /// calls with the person's answer.
    async fn answer_proposal(
        &self,
        name: &str,
        arguments: &Value,
        verified: &Verified,
        bearer: &str,
        session: &str,
        call: &mut Call,
    ) -> Value {
        let apply = name == super::propose::APPLY;
        let Some(space) = arguments["space"]
            .as_str()
            .and_then(|space| space.parse::<Did>().ok())
        else {
            call.end(Outcome::Invalid, "bad_space");
            return tool_result(json!("Name the proposal's space."), true);
        };
        let Some(id) = arguments["id"].as_str().filter(|id| !id.is_empty()) else {
            call.end(Outcome::Invalid, "no_proposal");
            return tool_result(json!("Name the proposal."), true);
        };
        call.space = Some(space.clone());
        let reached = match self.reach(verified, bearer, session, &space, call).await {
            Ok(reached) => reached,
            Err(refused) => return refused,
        };
        let route = if apply {
            super::propose::APPLY_ROUTE
        } else {
            super::propose::DISMISS_ROUTE
        };
        let body = json!({ "id": id }).to_string();
        let Some(request) = space_request(
            route,
            reached.granted.as_ref(),
            "application/json",
            &body,
            &call.trace,
        ) else {
            call.end(Outcome::Error, "bad_directory_entry");
            return tool_result(json!("The space's directory entry is malformed."), true);
        };
        let (status, body) = self
            .send(verified, bearer, session, &space, request, call)
            .await;
        if !status.is_success() {
            writer_failed(call, status, &parsed(&body));
        }
        let context = &reached.context;
        let label = context.place.label();
        let card = |card: super::card::Card, text: String, error: bool| {
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": { "card": card.to_value() },
                "isError": error,
            })
        };
        if status == StatusCode::GONE {
            return card(
                super::propose::gone(context),
                format!(
                    "The person answered a proposal for {label} that was already answered or had \
                     lapsed; nothing was written. Propose it again if they still want it."
                ),
                true,
            );
        }
        if apply && status == StatusCode::PRECONDITION_FAILED {
            return card(
                super::propose::moved(context),
                format!(
                    "The person applied the change in {label}, but the space changed after the \
                     preview they saw, so nothing was written. Read it again and propose the \
                     change again if it still applies."
                ),
                true,
            );
        }
        if !status.is_success() {
            let reason = reason(&parsed(&body));
            return card(
                super::propose::not_saved(context, &reason),
                format!(
                    "The person {} the change in {label}, but the space did not confirm it: \
                     {reason}. Query the space before proposing it again; never replay it \
                     automatically.",
                    if apply { "applied" } else { "declined" }
                ),
                true,
            );
        }
        if !apply {
            return card(
                super::propose::dismissed(context),
                format!(
                    "The person chose not to apply the change in {label}. Nothing was written. \
                     Do not propose it again unless they bring it up."
                ),
                false,
            );
        }
        let evaluation = parsed(&body);
        let change = undo::change_meta(&space, &evaluation);
        let evaluation = without_revisions(evaluation);
        let receipt = super::propose::receipt(context, &evaluation);
        let mut text = format!(
            "The person applied the change: {} (space {space}).",
            receipt.say
        );
        if let Some(notation) = notation(&body, true) {
            text.push('\n');
            text.push_str(&notation);
        }
        let mut answer = card(receipt, text, false);
        answer["structuredContent"]["evaluation"] = evaluation;
        if let Some(change) = change {
            answer["_meta"] = json!({ CHANGE_META: change });
        }
        answer
    }

    /// The space a write acts on, as a card names it, or the tool's
    /// refusal: a tell, with no role where the connection cannot read the
    /// space.
    async fn reach(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        space: &Did,
        call: &mut Call,
    ) -> Result<Reached, Value> {
        let access = match self.access(verified, bearer, session, call).await {
            Ok(access) => access,
            Err(error) => return Err(tool_result(json!(error), true)),
        };
        let agent = super::card::Agent {
            did: verified.agent().to_string(),
            name: access
                .get(verified.agent())
                .and_then(|own| own.name.clone()),
        };
        let home = space == verified.agent();
        let held = access.get(space);
        let place = super::card::Place {
            did: space.to_string(),
            name: held.and_then(|held| held.name.clone()),
            item: None,
        };
        let role = held.filter(|held| held.read).map(|held| {
            if home || held.write {
                super::card::Role::Write
            } else {
                super::card::Role::Read
            }
        });
        let context = super::propose::Context {
            place,
            role: if home {
                Some(super::card::Role::Write)
            } else {
                role
            },
            agent,
            home,
        };
        if home {
            return Ok(Reached {
                granted: None,
                context,
            });
        }
        if let Some((kind, refusal)) = refusal(held, space, true) {
            call.end(Outcome::Refused, kind);
            let label = if context.role.is_some() {
                "read only"
            } else {
                "no access"
            };
            let card = super::card::Card::tell(
                Some(context.place.clone()),
                context.role,
                label,
                refusal.clone(),
                context.agent.clone(),
            );
            return Err(json!({
                "content": [{ "type": "text", "text": refusal }],
                "structuredContent": { "card": card.to_value() },
                "isError": true,
            }));
        }
        Ok(Reached {
            granted: held.cloned(),
            context,
        })
    }

    /// Send `request` to the writer of `space`, and read its answer.
    async fn send(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        space: &Did,
        request: Request<Body>,
        call: &mut Call,
    ) -> (StatusCode, axum::body::Bytes) {
        call.steps.lap("prepare");
        let response = self
            .writers
            .call(verified, bearer, session, space, request)
            .await;
        call.steps.lap("writer");
        call.steps
            .extend_within("writer", Steps::of(response.headers()));
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        (status, body)
    }

    /// `tonk_undo`: undo the change `arguments.revision` made to
    /// `arguments.space`, through the session's writer of that space,
    /// which the connection must be able to write.
    async fn undo(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        arguments: &Value,
        call: &mut Call,
    ) -> Value {
        let Some(space) = arguments["space"].as_str() else {
            call.end(Outcome::Invalid, "no_space");
            return tool_result(
                json!("`space` is required: an exact subject from tonk_list_spaces."),
                true,
            );
        };
        let Ok(space) = space.parse::<Did>() else {
            call.end(Outcome::Invalid, "bad_space");
            return tool_result(json!(format!("`{space}` is not a space DID.")), true);
        };
        let Some(revision) = arguments["revision"]
            .as_str()
            .filter(|revision| !revision.is_empty() && revision.len() <= 256)
        else {
            call.end(Outcome::Invalid, "no_revision");
            return tool_result(json!("`revision` is required."), true);
        };
        call.space = Some(space.clone());
        let mut request = undo::request(revision);
        if &space != verified.agent() {
            let access = match self.access(verified, bearer, session, call).await {
                Ok(access) => access,
                Err(error) => return tool_result(json!(error), true),
            };
            if let Some((kind, refusal)) = refusal(access.get(&space), &space, true) {
                call.end(Outcome::Refused, kind);
                return tool_result(json!(refusal), true);
            }
            if let Some(granted) = access.get(&space) {
                let headers = request.headers_mut();
                if let Ok(grants) = HeaderValue::from_str(&granted.grants.join(",")) {
                    headers.insert(GRANTS_HEADER, grants);
                }
                if let Some(address) = granted
                    .address
                    .as_deref()
                    .and_then(|address| HeaderValue::from_str(address).ok())
                {
                    headers.insert(ADDRESS_HEADER, address);
                }
            }
        }
        let response = self
            .writers
            .call(verified, bearer, session, &space, request)
            .await;
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        if !status.is_success() {
            call.end(Outcome::Error, &format!("http_{}", status.as_u16()));
            return tool_result(
                json!(format!(
                    "The undo did not land ({status}): {}. Read the space before trying again.",
                    String::from_utf8_lossy(&body)
                )),
                true,
            );
        }
        match serde_json::from_slice::<crate::UndoReport>(&body) {
            Ok(report) => undo::result(&report),
            Err(error) => {
                call.end(Outcome::Error, "undo_unreadable");
                tool_result(
                    json!(format!("The writer answered an unreadable undo: {error}")),
                    true,
                )
            }
        }
    }

    /// What the agent space reaches, as its own writer reads it.
    async fn access(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        call: &mut Call,
    ) -> Result<AgentAccess, String> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(ACCESS_ROUTE)
            .header(TRACE_HEADER, &call.trace)
            .body(Body::empty())
            .expect("static request");
        let response = self
            .writers
            .call(verified, bearer, session, verified.agent(), request)
            .await;
        call.steps.lap("access");
        call.steps
            .extend_within("access", Steps::of(response.headers()));
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        if !status.is_success() {
            // A refusal (the session's own grant revoked) is recorded as
            // one, with the writer's code; anything else failed.
            if writer_outcome(status) == Outcome::Refused {
                let code = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|body| body["code"].as_str().and_then(writer_code));
                call.end(
                    Outcome::Refused,
                    code.as_deref().unwrap_or("access_refused"),
                );
            } else {
                call.end(Outcome::Error, "access");
            }
            return Err(format!(
                "Could not read this agent's spaces from its own space ({status}): {}. Try \
                 again; if it persists, ask the user to reconnect.",
                String::from_utf8_lossy(&body)
            ));
        }
        serde_json::from_slice(&body).map_err(|error| {
            call.end(Outcome::Error, "access_unreadable");
            format!("The agent space answered unreadable grants: {error}")
        })
    }

    fn unauthorized(&self, reason: &str) -> Response<Body> {
        let mut response = plain(StatusCode::UNAUTHORIZED, reason);
        let challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
            self.origin.trim_end_matches('/')
        );
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }
}

/// Why a tool may not act on `space`, open to `access`, if it may not: a
/// reason to record, and the message to answer.
fn refusal(
    access: Option<&SpaceAccess>,
    space: &Did,
    write: bool,
) -> Option<(&'static str, String)> {
    let Some(access) = access else {
        return Some((
            "no_access",
            format!(
                "This connection has no access to {space}. Use a subject from tonk_list_spaces, \
             or ask the person to grant you this space."
            ),
        ));
    };
    if !access.read {
        return Some(if access.lapsed > 0 {
            (
                "lapsed",
                format!("Access to {space} has lapsed. Ask the person to grant it to you again."),
            )
        } else {
            ("no_read", format!("This connection cannot read {space}."))
        });
    }
    if write && !access.write {
        return Some((
            "read_only",
            format!(
                "This connection may only read {space}; tonk_evaluate is refused. Ask the user \
             for write access."
            ),
        ));
    }
    None
}

/// A space a write reaches: the grant it goes through (none for the
/// agent space), and what its card says.
struct Reached {
    granted: Option<SpaceAccess>,
    context: super::propose::Context,
}

/// A `POST` of `body` to the writer route `path`, carrying the call's
/// `trace` and the grants and address of `granted`, or `None` when they
/// do not make headers.
fn space_request(
    path: &str,
    granted: Option<&SpaceAccess>,
    content_type: &str,
    body: &str,
    trace: &str,
) -> Option<Request<Body>> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .header(TRACE_HEADER, trace);
    if let Some(granted) = granted {
        request = request.header(GRANTS_HEADER, granted.grants.join(","));
        if let Some(address) = &granted.address {
            request = request.header(ADDRESS_HEADER, address);
        }
    }
    request.body(Body::from(body.to_owned())).ok()
}

/// End `call` for a writer's failed answer of `status` and `body`: the
/// writer's own code where it gives one, the status otherwise.
fn writer_failed(call: &mut Call, status: StatusCode, body: &Value) {
    let kind = body["code"]
        .as_str()
        .and_then(writer_code)
        .unwrap_or_else(|| format!("http_{}", status.as_u16()));
    call.end(writer_outcome(status), &kind);
}

/// A writer's answer as JSON, or as a string when it is not.
fn parsed(body: &[u8]) -> Value {
    serde_json::from_slice::<Value>(body).unwrap_or_else(|_| json!(String::from_utf8_lossy(body)))
}

/// The reason a writer's refusal gives, for a person.
fn reason(value: &Value) -> String {
    value["error"]
        .as_str()
        .or_else(|| value["message"].as_str())
        .or_else(|| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// What `tonk_list_spaces` answers: the agent space first, marked
/// `home`, then each granted space in force.
fn list_spaces(access: &AgentAccess) -> Vec<Value> {
    let mut listed: Vec<(&String, &SpaceAccess)> = access
        .spaces
        .iter()
        // A space whose every grant lapsed is named by the tool error a
        // call on it gets, not listed.
        .filter(|(_, access)| access.read)
        .collect();
    listed.sort_by_key(|(_, access)| !access.agent);
    listed
        .into_iter()
        .map(|(space, access)| {
            let mut entry = json!({
                "subject": space,
                "access": if access.write { "write" } else { "read" },
            });
            if let Some(name) = &access.name {
                entry["name"] = json!(name);
            }
            if let Some(expires) = access.expires {
                entry["expires"] = json!(super::access::rfc3339(expires));
            }
            if access.agent {
                entry["home"] = json!(true);
                entry["about"] = json!(HOME);
            }
            entry
        })
        .collect()
}

/// The manual for this connection, in place of the shared agent manual,
/// which is written for a single attached space and hosts that render views.
const HOSTED: &str = include_str!("mcp/guide-hosted.md");

/// The server instructions: the hosted manual, naming where Tonk is open.
fn instructions(origin: &str) -> String {
    HOSTED.replace("{origin}", origin.trim_end_matches('/'))
}

/// Rows a query answers when the model names no `limit`; the answer says
/// how to read the next page.
const DEFAULT_LIMIT: u64 = 100;

/// A worker answer in Tonk notation, as `tonk eval` prints it: the
/// envelope, then each expression's matches as re-submittable notation,
/// then how to read the next page if there is one.
fn notation(body: &[u8], write: bool) -> Option<String> {
    let response: output::EvaluateResponse = serde_json::from_slice(body).ok()?;
    let mut answer = output::render(&response, output::Format::Notation, false).ok()?;
    let page: Value = serde_json::from_slice(body).ok()?;
    let mut note = |line: String| {
        if !answer.ends_with('\n') {
            answer.push('\n');
        }
        answer.push_str(&line);
    };
    // A read-back pasted into tonk_evaluate without its `!` evaluates as a
    // query and succeeds, so say plainly that it wrote nothing.
    if write && response.commits.claims == 0 {
        note(
            "# Nothing was written: this document asserts nothing. Only a `concept!:` \
             head (with `!`) or a command writes; without `!` a document only reads.\n"
                .to_owned(),
        );
    }
    if let Some(next) = page["page"]["next"].as_str() {
        note(format!(
            "# More rows: call tonk_query again with cursor: \"{next}\"\n"
        ));
    }
    Some(answer)
}

/// `answer` without the raw revision bytes the worker reports, which tell
/// the model nothing and cost it about 1.5 KB a call.
fn without_revisions(mut answer: Value) -> Value {
    if let Some(fields) = answer.as_object_mut() {
        fields.remove("revision_before");
        fields.remove("revision_after");
    }
    answer
}

/// The manuals as `tonk://guide/<topic>` resources.
fn guide_resources() -> Vec<Value> {
    guide::TOPICS
        .iter()
        .map(|topic| {
            json!({
                "uri": format!("tonk://guide/{topic}"),
                "name": topic,
                "description": guide::description(topic).unwrap_or_default(),
                "mimeType": "text/markdown",
            })
        })
        .collect()
}

/// The manual a `tonk://guide/<topic>` resource names.
fn read_guide(uri: &str) -> Option<Value> {
    let body = match uri.strip_prefix("tonk://guide/")? {
        "agent" => HOSTED,
        topic => guide::topic(topic)?,
    };
    Some(json!({
        "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": body }],
    }))
}

/// How a listing describes the agent's own space to the model.
const HOME: &str = "Your own space. Only you write here unless you share it: \
keep notes, drafts and anything you want to remember across conversations. \
The other spaces are the ones the person granted you, with the access shown.";

/// The offered tools, from the shared catalog, with `space` added to the
/// scoped ones.
pub fn tools() -> Vec<Value> {
    let catalog: Value = serde_json::from_str(CATALOG).expect("valid tool catalog");
    catalog["tools"]
        .as_array()
        .expect("catalog lists tools")
        .iter()
        .filter(|tool| TOOLS.contains(&tool["name"].as_str().unwrap_or_default()))
        .map(|tool| {
            let mut tool = tool.clone();
            let name = tool["name"].as_str().unwrap_or_default().to_owned();
            if name == "tonk_query" {
                tool["description"] = json!(
                    "Read a space with inline Tonk notation; never changes anything. Name the \
                     space by its exact subject from tonk_list_spaces. Query concept: for \
                     schema and read tonk_guide for notation. Select the fields you need and \
                     use exact saved entity URIs."
                );
            }
            if name == "tonk_evaluate" {
                tool["description"] = json!(
                    "Write to a space with inline Tonk notation: assertions and commands, run \
                     once, answered after the write is saved. Needs write access to the \
                     space (see tonk_list_spaces). Read tonk_guide (notation, views, events) \
                     before authoring unfamiliar schemas or views. If a response is lost, query \
                     before trying again; never replay automatically. With confirm: true \
                     nothing is written: the person sees what the change would do in a card \
                     and applies or declines it there. Use it for many changes at once, for \
                     changes others will see that are hard to undo, or when the person asked \
                     to confirm first."
                );
                tool["inputSchema"]["properties"]["confirm"] = json!({
                    "type": "boolean",
                    "default": false,
                    "description": "Propose the change for the person to confirm instead of \
                        writing it.",
                });
                tool["_meta"] = super::card::tool_meta();
            }
            if name == "tonk_list_spaces" {
                tool["description"] = json!(
                    "List the spaces you can use. The first, marked home, is your own space: \
                     it is yours to read and write, and it records the spaces the user granted \
                     you. Each other entry is a granted space with its name when the user gave \
                     one, its access (read or write) and when that access expires. Use \
                     the exact subject as the space argument of tonk_query and tonk_evaluate."
                );
            }
            if SCOPED.contains(&name.as_str()) {
                tool["inputSchema"]["properties"]["space"] = json!({
                    "type": "string",
                    "pattern": "^did:key:[A-Za-z0-9]+$",
                    "description": "Exact subject returned by tonk_list_spaces.",
                });
                if let Some(required) = tool["inputSchema"]["required"].as_array_mut() {
                    required.push(json!("space"));
                }
            }
            if name == "tonk_query" {
                tool["inputSchema"]["properties"]["limit"] = json!({
                    "type": "integer",
                    "minimum": 1,
                    "description": "Most rows to answer; a larger read is paged.",
                });
                tool["inputSchema"]["properties"]["cursor"] = json!({
                    "type": "string",
                    "description": "The `page.next` of the previous answer, to read the next page.",
                });
            }
            tool
        })
        .collect()
}

fn tool_result(result: Value, error: bool) -> Value {
    let text = match &result {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut answer = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": error,
    });
    // MCP requires structured content to be an object.
    if !error && result.is_object() {
        answer["structuredContent"] = result;
    }
    answer
}

fn rpc(status: StatusCode, value: Value) -> Response<Body> {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn plain(status: StatusCode, message: &str) -> Response<Body> {
    rpc(status, json!({ "error": message }))
}

fn accepted() -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::ACCEPTED;
    response
}

#[cfg(test)]
mod tests;
