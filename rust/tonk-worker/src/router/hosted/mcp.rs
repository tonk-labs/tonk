//! `/mcp`: the hosted agent's MCP endpoint, Streamable HTTP without SSE.
//!
//! A request carries an agent session token (`tonk-identity`'s
//! [`AgentSessionToken`]) as its Bearer. The front decodes it, checks the
//! grants it carries (each a chain from a space to the agent space), and
//! answers JSON-RPC over `POST`: `initialize`, `ping`, `tools/list` and
//! `tools/call`. A missing or bad token is answered `401` with the
//! protected resource metadata the OAuth client discovers the
//! authorization server from.
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
//! - each space a grant names: readable when a grant reaches
//!   `/use/get/...`, writable when one also reaches `/use/put/...`.
//!   `tonk_evaluate` is refused for a space without put grants, and the
//!   access service refuses a put such a chain carries anyway.
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

use std::collections::BTreeMap;

use axum::body::Body;
use axum::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_credentials::DidKeyResolver;
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tonk_identity::agent::{AgentSessionToken, decode_agent_session_token};

use super::WriterSession;

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

/// What one space is open to through the token.
#[derive(Debug, Clone)]
pub struct SpaceAccess {
    /// Whether this is the agent space itself.
    pub agent: bool,
    /// Whether a grant in force reaches `/use/get/...`.
    pub read: bool,
    /// Whether a grant in force also reaches `/use/put/...`.
    pub write: bool,
    /// The grants for this space that are in force, to hand the writer.
    pub grants: Vec<DelegationChain>,
    /// The earliest expiry among those grants, in Unix seconds.
    pub expires: Option<u64>,
    /// Grants for this space that have expired or are not yet valid.
    pub lapsed: usize,
}

/// A token that decoded and whose grants verified.
pub struct Verified {
    token: AgentSessionToken,
    hash: [u8; 32],
    spaces: BTreeMap<Did, SpaceAccess>,
}

impl Verified {
    /// The decoded token.
    pub fn token(&self) -> &AgentSessionToken {
        &self.token
    }

    /// The spaces the token reaches, the agent space among them.
    pub fn spaces(&self) -> &BTreeMap<Did, SpaceAccess> {
        &self.spaces
    }

    /// What the writer of `space` for the MCP session `mcp_session` acts
    /// with: the session's seed and powerline, and the grants in force
    /// for that space.
    pub fn writer_session(&self, space: &Did, mcp_session: &str) -> WriterSession {
        WriterSession {
            seed: self.token.seed().clone(),
            powerline: self.token.powerline().clone(),
            mcp_session: mcp_session.to_owned(),
            grants: self
                .spaces
                .get(space)
                .map(|access| access.grants.clone())
                .unwrap_or_default(),
        }
    }
}

/// Decode `bearer` and check what it carries. Every refusal is a reason
/// to answer `401`.
///
/// The token's own delegations are checked by
/// [`decode_agent_session_token`]. Each grant must start at its subject,
/// end at the agent space, and be signed throughout; a grant outside its
/// time bounds is kept as lapsed rather than refused, so a tool can name
/// the space whose access ran out instead of the host starting over.
pub async fn verify(bearer: &str) -> Result<Verified, String> {
    let token = decode_agent_session_token(bearer)
        .await
        .map_err(|error| error.to_string())?;
    let now = Timestamp::now();
    let agent = token.agent().clone();
    let mut spaces = BTreeMap::new();
    spaces.insert(
        agent.clone(),
        SpaceAccess {
            agent: true,
            read: true,
            write: true,
            grants: Vec::new(),
            expires: None,
            lapsed: 0,
        },
    );
    for grant in token.grants() {
        if grant.audience() != &agent {
            return Err("a grant is not addressed to the agent space".into());
        }
        let Some(subject) = grant.subject().cloned() else {
            return Err("a grant names no space".into());
        };
        if grant.issuer() != &subject {
            return Err(format!("a grant for {subject} is not issued by it"));
        }
        let mut audience: Option<&Did> = None;
        for delegation in grant.proofs() {
            // Each link is issued by whoever the one before it delegated to.
            if audience.is_some_and(|audience| delegation.issuer() != audience) {
                return Err(format!("a grant for {subject} is not one chain"));
            }
            audience = Some(delegation.audience());
            // A subject-open link (the account's powerline to a device) is
            // in every grant an account mints; it narrows nothing, and the
            // chain's other links pin the space.
            if delegation.subject() != &UcanSubject::Specific(subject.clone())
                && delegation.subject() != &UcanSubject::Any
            {
                return Err(format!("a grant for {subject} changes subject"));
            }
            delegation
                .verify_signature(&DidKeyResolver)
                .await
                .map_err(|error| format!("a grant for {subject} is not signed: {error}"))?;
        }
        let access = spaces.entry(subject).or_insert_with(|| SpaceAccess {
            agent: false,
            read: false,
            write: false,
            grants: Vec::new(),
            expires: None,
            lapsed: 0,
        });
        if access.agent {
            continue;
        }
        let in_force = grant.expiration().is_none_or(|at| at > now)
            && grant.not_before().is_none_or(|at| at <= now);
        if !in_force {
            access.lapsed += 1;
            continue;
        }
        let command = grant
            .proofs()
            .last()
            .map(|leaf| leaf.command().0.clone())
            .unwrap_or_default();
        access.read |= reaches(&command, "get");
        access.write |= reaches(&command, "put");
        if let Some(at) = grant.expiration() {
            let at = at.to_unix();
            access.expires = Some(access.expires.map_or(at, |held| held.min(at)));
        }
        access.grants.push(grant.clone());
    }
    // A space is writable only if it is readable too: a write pulls first.
    for access in spaces.values_mut() {
        access.write &= access.read;
    }
    Ok(Verified {
        hash: *blake3::hash(bearer.trim().as_bytes()).as_bytes(),
        token,
        spaces,
    })
}

/// Whether a grant whose leaf command is `command` covers some
/// `/use/{verb}/...` invocation.
fn reaches(command: &[String], verb: &str) -> bool {
    let wanted = ["use", verb];
    command
        .iter()
        .zip(wanted)
        .all(|(segment, wanted)| segment == wanted)
}

/// Where the front sends a space's reads and writes.
#[allow(async_fn_in_trait)]
pub trait Writers {
    /// Serve `request` on the writer of `space` for `mcp_session`.
    ///
    /// `request` is one of the writer's routes relative to the space:
    /// `POST /api/query` (a read) or `POST /api/evaluate` (a write).
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
                        "instructions": guide::AGENT,
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
            "resources/list" => Ok(json!({ "resources": super::view::resources() })),
            "resources/read" => {
                let uri = message["params"]["uri"].as_str().unwrap_or_default();
                super::view::read_resource(uri, self.origin).ok_or_else(
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
            "tonk_list_spaces" => tool_result(json!({ "spaces": list_spaces(verified) }), false),
            "tonk_guide" => {
                let topic = arguments["topic"].as_str().unwrap_or("");
                let body = if topic == "context" {
                    Some(guide::AGENT)
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
                if let Some(refusal) = refusal(verified, &space, write) {
                    return tool_result(json!(refusal), true);
                }
                let path = if write {
                    "/api/evaluate".to_owned()
                } else {
                    // A page of a large read: the writer caps reads and
                    // answers the next cursor.
                    let mut page = url::form_urlencoded::Serializer::new(String::new());
                    if let Some(limit) = arguments["limit"].as_u64() {
                        page.append_pair("limit", &limit.to_string());
                    }
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
                let request = Request::builder()
                    .method(Method::POST)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(document.to_owned()))
                    .expect("static request");
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
                let body = match serde_json::from_slice::<Value>(&body) {
                    Ok(value) => value,
                    Err(_) => json!(String::from_utf8_lossy(&body)),
                };
                tool_result(body, !status.is_success())
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

/// Why a tool may not act on `space`, if it may not.
fn refusal(verified: &Verified, space: &Did, write: bool) -> Option<String> {
    let Some(access) = verified.spaces.get(space) else {
        return Some(format!(
            "This connection has no access to {space}. Use a subject from tonk_list_spaces; \
             the user grants more spaces by reconnecting."
        ));
    };
    if !access.read {
        return Some(if access.lapsed > 0 {
            format!(
                "Access to {space} has lapsed. Ask the user to renew it for this agent, then \
                 reconnect."
            )
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

fn list_spaces(verified: &Verified) -> Vec<Value> {
    verified
        .spaces
        .iter()
        // A space whose every grant lapsed is named by the tool error a
        // call on it gets, not listed.
        .filter(|(_, access)| access.read)
        .map(|(space, access)| {
            let mut entry = json!({
                "subject": space.to_string(),
                "access": if access.write { "read-write" } else { "read" },
            });
            if access.agent {
                entry["agent_space"] = json!(true);
            }
            if let Some(expires) = access.expires {
                entry["expires"] = json!(expires);
            }
            entry
        })
        .collect()
}

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
            if name == "tonk_list_spaces" {
                tool["description"] = json!(
                    "List the spaces this connection reaches: the agent's own space and each \
                     space the user granted, with its access level (read or read-write). Use \
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
