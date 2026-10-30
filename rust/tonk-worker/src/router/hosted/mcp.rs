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

#[path = "../../../../tonk-cli/src/output.rs"]
#[allow(dead_code)]
mod output;

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

/// Header on a request to a granted space's writer: where the agent
/// space's directory says the space syncs, when it says.
pub const ADDRESS_HEADER: &str = "x-tonk-space-address";

/// Where a writer mounts a space: the address the agent space's
/// directory names (`named`), only when its origin is `configured`'s, the
/// access service the host is configured with; `configured` otherwise.
///
/// Anyone who can write the agent space can write its directory, the
/// agent included, so the address is not trusted to send the writer's
/// invocations anywhere else.
pub fn mount_address(named: Option<&str>, configured: &str) -> String {
    let origin = |address: &str| url::Url::parse(address).ok().map(|url| url.origin());
    match (named, origin(configured)) {
        (Some(named), Some(trusted))
            if origin(named).is_some_and(|origin| origin == trusted && trusted.is_tuple()) =>
        {
            named.to_owned()
        }
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
    /// `POST /api/query` (a read), `POST /api/evaluate` (a write), or
    /// `GET` [`ACCESS_ROUTE`] (to the agent space's writer). A request to
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
}

impl<W: Writers> McpEndpoint<'_, W> {
    /// Answer one request to `/mcp`.
    pub async fn serve(&self, request: Request<Body>) -> Response<Body> {
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
                tools.extend(super::view::tools());
                Ok(json!({ "tools": tools }))
            }
            "resources/list" => {
                let mut resources = guide_resources();
                resources.extend(super::view::resources());
                Ok(json!({ "resources": resources }))
            }
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => {
                let uri = message["params"]["uri"].as_str().unwrap_or_default();
                read_guide(uri)
                    .or_else(|| super::view::read_resource(uri, self.origin))
                    .ok_or_else(
                        || json!({ "code": -32602, "message": format!("no resource {uri}") }),
                    )
            }
            "tools/call" => Ok(self
                .call_tool(&verified, &bearer, &session, &message["params"])
                .await),
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

    async fn call_tool(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
        params: &Value,
    ) -> Value {
        let name = params["name"].as_str().unwrap_or_default();
        let arguments = &params["arguments"];
        match name {
            "tonk_list_spaces" => match self.access(verified, bearer, session).await {
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
                    (_, None) => tool_result(json!("Unknown guide topic."), true),
                }
            }
            "tonk_query" | "tonk_evaluate" => {
                let write = name == "tonk_evaluate";
                let Some(space) = arguments["space"].as_str() else {
                    return tool_result(
                        json!("`space` is required: an exact subject from tonk_list_spaces."),
                        true,
                    );
                };
                let Some(document) = arguments["document"]
                    .as_str()
                    .filter(|document| !document.is_empty())
                else {
                    return tool_result(json!("`document` is required."), true);
                };
                let Ok(space) = space.parse::<Did>() else {
                    return tool_result(json!(format!("`{space}` is not a space DID.")), true);
                };
                // The agent space is reached through the powerline alone;
                // any other space through what the agent space holds.
                let granted = if &space == verified.agent() {
                    None
                } else {
                    let access = match self.access(verified, bearer, session).await {
                        Ok(access) => access,
                        Err(error) => return tool_result(json!(error), true),
                    };
                    if let Some(refusal) = refusal(access.get(&space), &space, write) {
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
                    .header(header::CONTENT_TYPE, "text/plain");
                if let Some(granted) = &granted {
                    request = request.header(GRANTS_HEADER, granted.grants.join(","));
                    if let Some(address) = &granted.address {
                        request = request.header(ADDRESS_HEADER, address);
                    }
                }
                let Ok(request) = request.body(Body::from(document.to_owned())) else {
                    return tool_result(json!("The space's directory entry is malformed."), true);
                };
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
                let text = if status.is_success() {
                    notation(&body, write)
                } else {
                    None
                };
                let body = match serde_json::from_slice::<Value>(&body) {
                    Ok(value) => value,
                    Err(_) => json!(String::from_utf8_lossy(&body)),
                };
                let mut answer = tool_result(without_revisions(body), !status.is_success());
                // The model reads notation; programs read the JSON beside it.
                if let Some(text) = text {
                    answer["content"] = json!([{ "type": "text", "text": text }]);
                }
                answer
            }
            _ => match super::view::call(
                name,
                arguments,
                verified,
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

    /// What the agent space reaches, as its own writer reads it.
    async fn access(
        &self,
        verified: &Verified,
        bearer: &str,
        session: &str,
    ) -> Result<AgentAccess, String> {
        let request = Request::builder()
            .method(Method::GET)
            .uri(ACCESS_ROUTE)
            .body(Body::empty())
            .expect("static request");
        let response = self
            .writers
            .call(verified, bearer, session, verified.agent(), request)
            .await;
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "Could not read this agent's spaces from its own space ({status}): {}. Try \
                 again; if it persists, ask the user to reconnect.",
                String::from_utf8_lossy(&body)
            ));
        }
        serde_json::from_slice(&body)
            .map_err(|error| format!("The agent space answered unreadable grants: {error}"))
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

/// Why a tool may not act on `space`, open to `access`, if it may not.
fn refusal(access: Option<&SpaceAccess>, space: &Did, write: bool) -> Option<String> {
    let Some(access) = access else {
        return Some(format!(
            "This connection has no access to {space}. Use a subject from tonk_list_spaces, \
             or ask the person to grant you this space."
        ));
    };
    if !access.read {
        return Some(if access.lapsed > 0 {
            format!("Access to {space} has lapsed. Ask the person to grant it to you again.")
        } else {
            format!("This connection cannot read {space}.")
        });
    }
    if write && !access.write {
        return Some(format!(
            "This connection may only read {space}; tonk_evaluate is refused. Ask the user \
             for read and write access."
        ));
    }
    None
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
                "access": if access.write { "read-write" } else { "read" },
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
                     once, answered after the write is saved. Needs read-write access to the \
                     space (see tonk_list_spaces). Read tonk_guide (notation, views, events) \
                     before authoring unfamiliar schemas or views. If a response is lost, query \
                     before trying again; never replay automatically."
                );
            }
            if name == "tonk_list_spaces" {
                tool["description"] = json!(
                    "List the spaces you can use. The first, marked home, is your own space: \
                     it is yours to read and write, and it records the spaces the user granted \
                     you. Each other entry is a granted space with its name when the user gave \
                     one, its access (read or read-write) and when that access expires. Use \
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
