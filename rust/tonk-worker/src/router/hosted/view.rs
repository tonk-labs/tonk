//! Views for `/mcp` hosts (MCP Apps), as plan #1200 stage 7 lays out.
//!
//! The OAuth token stays in the host; a view never sees it. The model
//! calls `tonk_open_space` to show a route of a space. The view (the
//! relay page) makes its own non-extractable key and calls
//! `tonk_view_access(space, audience)`, an app-only tool, with that key's
//! DID; the session signs a delegation to it for that one space, to edit
//! it wherever the connection may write, else to read it. The delegation is useless without
//! the view's private key, so it is no secret: it may sit in tool results,
//! conversation storage and logs. The view's peer then acts as its own
//! DID, against the provider's `/ucan/` and `live.rs`, with no Host Worker
//! in between.
//!
//! What a view may do never exceeds what the session may: each delegation
//! extends a grant in force (or, for the agent space, the powerline), so
//! revoking the session or a grant cuts the views under it too.

use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::time::{Duration, UNIX_EPOCH};
use dialog_ucan_core::{Container, DelegationBuilder, DelegationChain, Tag};
use dialog_varsig::Did;
use serde_json::{Value, json};

use axum::body::Body;
use axum::http::{Method, Request, Response};
use http_body_util::BodyExt as _;

use super::access::reaches;
use super::mcp::{
    ACCESS_ROUTE, ADDRESS_HEADER, AgentAccess, GRANTS_HEADER, SpaceAccess, Verified, Writers,
    mount_address,
};

/// The key view tickets are bound under, derived from the host's secret
/// under its own context, apart from the session id MAC. Unused by the
/// delegated views; kept so the seam's signature stays put.
#[derive(Clone)]
pub struct TicketKey(#[allow(dead_code)] [u8; 32]);

impl TicketKey {
    /// Derive the key from `secret`, the host's MCP secret.
    pub fn derive(secret: &str) -> Self {
        Self(blake3::derive_key(
            "tonk hosted view ticket v1",
            secret.as_bytes(),
        ))
    }
}

/// The model-visible tool that shows a route of a space, as the hosted
/// MCP server's `tonk_open_space` does (not the local peer's `tonk_show`,
/// a live query result).
pub const OPEN: &str = "tonk_open_space";

/// The app-only tool a view gets its delegation from.
pub const ACCESS: &str = "tonk_view_access";

/// The model-visible tool that answers the views a space renders, each
/// with the route [`OPEN`] shows it alone by.
pub const OUTLINE: &str = "tonk_outline";

/// The relay page's resource.
pub const RELAY_URI: &str = "ui://tonk/space-view-v2.html";

/// How long a view's delegation holds, in seconds. The view asks again
/// before it runs out, and on every load.
pub const ACCESS_LIFETIME: u64 = 60 * 60;

/// Tools appended to `tools/list`.
pub fn tools() -> Vec<Value> {
    let space = json!({
        "type": "string",
        "pattern": "^did:key:[A-Za-z0-9]+$",
        "description": "Exact subject returned by tonk_list_spaces.",
    });
    vec![
        json!({
            "name": OPEN,
            "title": "Open a Tonk space",
            "description": "Show a route of a space inline in the chat, live, as the space's own \
                page renders it. Use a path within the space, such as / or /notebook/<entity>; \
                entity route parameters must be actual entity URIs returned by tonk_query, never \
                titles or names. The person can change the space from the view wherever this \
                connection may write it; a space it may only read is shown read-only. Showing \
                a route does not verify that it rendered.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "space": space,
                    "path": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 4096,
                        "default": "/",
                        "description": "A path within the space.",
                    },
                    "view": {
                        "type": "string",
                        "maxLength": 2048,
                        "description": "One view of the space to show on its own, instead of \
                            the page at path: a route from tonk_outline (entity@model, \
                            entity@model!view, or a model for every instance).",
                    },
                },
                "required": ["space"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false },
            "_meta": {
                "ui": { "resourceUri": RELAY_URI },
                "openai/outputTemplate": RELAY_URI,
            },
        }),
        json!({
            "name": ACCESS,
            "description": "Delegate one space to a shown view's own key. Called by the view.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "space": space,
                    "audience": {
                        "type": "string",
                        "pattern": "^did:key:[A-Za-z0-9]+$",
                        "description": "The view's key.",
                    },
                },
                "required": ["space", "audience"],
                "additionalProperties": false,
            },
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
        json!({
            "name": OUTLINE,
            "title": "Outline a Tonk space's views",
            "description": "List the views a space renders, as a tree: each node names its \
                model and concept, its entity, how many records it shows, any error, its \
                children, and a route that shows it alone. Without route the tree starts at the \
                space's home. Pass a node's route to tonk_open_space as view to show just that \
                view, for example one list from a larger page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "space": space,
                    "route": {
                        "type": "string",
                        "maxLength": 2048,
                        "description": "A route from an earlier outline to start from.",
                    },
                },
                "required": ["space"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false },
        }),
    ]
}

/// The MIME type MCP Apps hosts render.
const RELAY_MIME: &str = "text/html;profile=mcp-app";

/// The relay page, with its origin left to fill in.
const RELAY: &str = include_str!("view/relay.html");

/// Resources served by `resources/list`.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": RELAY_URI,
        "name": "tonk-space-view",
        "title": "Tonk space",
        "mimeType": RELAY_MIME,
    })]
}

/// The `resources/read` result for `uri`, or `None` when there is no such
/// resource: the relay page for `origin`, the provider the host reached,
/// which the page loads the view peer and the guest runtime from and
/// whose `/ucan/` the peer syncs with.
pub fn read_resource(uri: &str, origin: &str) -> Option<Value> {
    if uri != RELAY_URI {
        return None;
    }
    let origin = origin.trim_end_matches('/');
    let connect = connect_domains(origin);
    let html = RELAY
        .replace("__TONK_MCP_ORIGIN__", &json!(origin).to_string())
        .replace("__TONK_VIEW_FRAME__", "false");
    Some(json!({
        "contents": [{
            "uri": RELAY_URI,
            "mimeType": RELAY_MIME,
            "text": html,
            "_meta": {
                "ui": {
                    "csp": {
                        "connectDomains": connect,
                        "resourceDomains": [origin],
                        // Route A: a host that honours it frames the
                        // provider's page, whose worker is the peer.
                        "frameDomains": [origin],
                    },
                    "prefersBorder": true,
                },
                "openai/widgetCSP": {
                    "connect_domains": connect,
                    "resource_domains": [origin],
                    "frame_domains": [origin],
                },
            },
        }],
    }))
}

/// Tonk's own deployments, where most spaces live. A view may sync with
/// these wherever it is served from. A space hosted anywhere else is
/// refused until views can follow each grant's location.
const TONK_ORIGINS: [&str; 3] = [
    "https://tonk.foundation",
    "https://tonk.network",
    "https://tonk.spot",
];

/// The origins a view's peer may sync with: those its policy names in
/// `connectDomains`. The provider's own, then Tonk's deployments.
fn sync_origins(origin: &str) -> Vec<String> {
    let origin = origin.trim_end_matches('/');
    std::iter::once(origin)
        .chain(TONK_ORIGINS.into_iter().filter(|known| *known != origin))
        .map(str::to_owned)
        .collect()
}

/// The view's `connectDomains`: each origin it syncs with, and its
/// `live.rs` socket, which a policy listing only the https origin
/// refuses.
fn connect_domains(origin: &str) -> Vec<String> {
    sync_origins(origin)
        .into_iter()
        .flat_map(|origin| {
            let socket = origin
                .strip_prefix("https://")
                .map(|host| format!("wss://{host}"))
                .or_else(|| {
                    origin
                        .strip_prefix("http://")
                        .map(|host| format!("ws://{host}"))
                });
            std::iter::once(origin).chain(socket)
        })
        .collect()
}

/// Whether a view served from `origin` may sync with `remote`, a
/// space's `/ucan/` address. The host enforces the view's policy, so a
/// view of a space elsewhere would fail on its first request.
fn view_reaches(remote: &str, origin: &str) -> bool {
    let Ok(remote) = url::Url::parse(remote) else {
        return false;
    };
    let remote = remote.origin().ascii_serialization();
    sync_origins(origin).contains(&remote)
}

/// The agent space writer's route answering the grants in force for one
/// space (`?space=<did>`), as [`chains_answer`] encodes them.
pub const CHAINS_ROUTE: &str = "/api/access/chains";

/// The [`CHAINS_ROUTE`] answer for `chains`: `{"chains": [...]}`, each a
/// base64url container.
pub fn chains_answer(chains: &[DelegationChain]) -> Result<Value, String> {
    Ok(json!({ "chains": encode(chains)? }))
}

fn encode(chains: &[DelegationChain]) -> Result<Vec<String>, String> {
    chains
        .iter()
        .map(|chain| {
            let encoded = Container::from(chain)
                .encode(Tag::Base64Url)
                .map_err(|error| error.to_string())?;
            String::from_utf8(encoded).map_err(|error| error.to_string())
        })
        .collect()
}

/// The result of the view tool `name`, or `None` when it is not one.
///
/// What a space is open to is the agent space's to say, so both tools ask
/// its writer through `writers`: [`ACCESS_ROUTE`] for whether the space
/// may be read or edited, and [`CHAINS_ROUTE`] for the grants a view's
/// delegation of a granted space extends.
#[allow(clippy::too_many_arguments)]
pub async fn call<W: Writers>(
    name: &str,
    arguments: &Value,
    verified: &Verified,
    writers: &W,
    bearer: &str,
    mcp_session: &str,
    origin: &str,
    _tickets: &TicketKey,
) -> Option<Value> {
    if name != OPEN && name != ACCESS && name != OUTLINE {
        return None;
    }
    let space = match arguments["space"].as_str().map(str::parse::<Did>) {
        Some(Ok(space)) => space,
        _ => {
            return Some(error(
                "Provide the exact space subject from tonk_list_spaces.",
            ));
        }
    };
    let agent = verified.agent().clone();
    let ask = |request: Request<Body>| writers.call(verified, bearer, mcp_session, &agent, request);
    let access = if space == agent {
        SpaceAccess {
            agent: true,
            read: true,
            write: true,
            ..SpaceAccess::default()
        }
    } else {
        let access: AgentAccess = match answer(ask(get(ACCESS_ROUTE)).await).await {
            Ok(access) => access,
            Err(message) => return Some(error(&message)),
        };
        match access.get(&space) {
            Some(access) => access.clone(),
            None => return Some(error("This connection does not reach that space.")),
        }
    };
    if !access.agent && !access.read {
        return Some(error(
            "This connection's access to that space has lapsed. Ask the user to grant it again.",
        ));
    }
    if name == OUTLINE {
        return Some(
            outline(
                arguments,
                &access,
                writers,
                verified,
                bearer,
                mcp_session,
                &space,
            )
            .await,
        );
    }
    // A view edits wherever the connection may write, and only reads a
    // space granted read.
    let edit = access.agent || access.write;
    let remote = mount_address(
        access.address.as_deref(),
        &format!("{}/ucan/", origin.trim_end_matches('/')),
    );
    if !view_reaches(&remote, origin) {
        let at = url::Url::parse(&remote)
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or(remote.clone());
        return Some(error(&format!(
            "This space is hosted at {at}, and a view from {} cannot reach it yet, so it \
             cannot be shown inline. Ask the person to open it in Tonk at {at}.",
            origin.trim_end_matches('/')
        )));
    }
    if name == OPEN {
        let path = arguments["path"].as_str().unwrap_or("/");
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.chars().any(|c| c == '\\' || c.is_control())
        {
            return Some(error("Provide a path within the space, such as /."));
        }
        let view = match arguments["view"].as_str().map(str::trim) {
            None | Some("") => Value::Null,
            Some(route) if route.len() > 2048 => {
                return Some(error("Provide a view route from tonk_outline, up to 2 KB."));
            }
            Some(route) => match tonk_render::RenderRoute::parse(route) {
                // The home mounts on each peer's own replica; the path
                // shows it, as the outline's empty home route means.
                Ok(route) if super::super::view_tree::is_home(&route) => Value::Null,
                Ok(route) => json!({
                    "model": route.model,
                    "entity": route.entity,
                    "facet": route.view,
                }),
                Err(problem) => {
                    return Some(error(&format!(
                        "`{route}` is not a view route ({problem}). Use a route from tonk_outline."
                    )));
                }
            },
        };
        let text = if view.is_null() {
            "Showing the space inline. Rendering is not verified by this response."
        } else {
            "Showing that view inline. Rendering is not verified by this response."
        };
        return Some(json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": {
                "subject": space.to_string(),
                "path": path,
                "view": view,
                "edit": edit,
                "remote": remote,
            },
            "isError": false,
        }));
    }
    let audience = match arguments["audience"].as_str().map(str::parse::<Did>) {
        Some(Ok(audience)) => audience,
        _ => return Some(error("Provide the view key's did:key.")),
    };
    let grants = if access.agent {
        Vec::new()
    } else {
        let uri = format!("{CHAINS_ROUTE}?space={space}");
        let chains: Value = match answer(ask(get(&uri)).await).await {
            Ok(chains) => chains,
            Err(message) => return Some(error(&message)),
        };
        let decoded = chains["chains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|encoded| {
                let container = Container::decode(encoded.as_bytes()).map_err(|e| e.to_string())?;
                DelegationChain::try_from(container).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, String>>();
        match decoded {
            Ok(grants) => grants,
            Err(message) => {
                return Some(error(&format!(
                    "The agent space answered an unreadable grant: {message}"
                )));
            }
        }
    };
    let expires = Timestamp::now().to_unix() + ACCESS_LIFETIME;
    match delegate(
        verified,
        &space,
        access.agent,
        &grants,
        &audience,
        edit,
        expires,
    )
    .await
    {
        Ok(chains) => Some(json!({
            "content": [{ "type": "text", "text": "Delegated." }],
            "structuredContent": {
                "space": space.to_string(),
                "audience": audience.to_string(),
                "edit": edit,
                "expiresAt": expires * 1000,
                "remote": remote,
                "delegations": chains,
            },
            "isError": false,
        })),
        Err(message) => Some(error(&message)),
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("a static request")
}

/// The JSON a writer answered, or why there is none.
async fn answer<T: serde::de::DeserializeOwned>(response: Response<Body>) -> Result<T, String> {
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Could not read this agent's grants from its own space ({status}): {}. Try again; \
             if it persists, ask the user to reconnect.",
            String::from_utf8_lossy(&body)
        ));
    }
    serde_json::from_slice(&body)
        .map_err(|error| format!("The agent space answered unreadable grants: {error}"))
}

/// The session's delegations of `space` to `audience`, each a chain from
/// the space to the audience, encoded as base64url containers.
///
/// For a granted space: each of `grants` (the agent space's grants in
/// force for it, `space -> ... -> agent`) that reaches what the view may
/// do, extended by the powerline and a last hop from the session to the
/// audience. For the agent space: the powerline and that hop. The last
/// hop names the space and, for a read-only view, narrows the command to
/// `/use/get`, so the view can sync and read and never publish.
async fn delegate(
    verified: &Verified,
    space: &Did,
    agent: bool,
    grants: &[DelegationChain],
    audience: &Did,
    edit: bool,
    expires: u64,
) -> Result<Vec<String>, String> {
    let token = verified.token();
    let session = Ed25519Signer::import(&**token.seed())
        .await
        .map_err(|error| format!("bad session key: {error}"))?;
    let powerline = token.powerline();
    let mut bases = Vec::new();
    if agent {
        let command = if edit { vec!["use".into()] } else { read() };
        bases.push((command, powerline.clone()));
    } else {
        for grant in grants {
            let command = command_of(grant);
            let command = match (edit, reaches(&command, "get")) {
                (true, _) => command,
                (false, true) if command.len() >= 2 => command,
                (false, true) => read(),
                (false, false) => continue,
            };
            let mut base = grant.clone();
            for delegation in powerline.proofs() {
                base = base
                    .push(delegation.clone())
                    .map_err(|error| error.to_string())?;
            }
            bases.push((command, base));
        }
    }
    if bases.is_empty() {
        return Err("No grant in force lets a view read that space.".into());
    }
    let mut chains = Vec::new();
    for (command, base) in bases {
        let leaf = DelegationBuilder::new()
            .issuer(Signer::from(session.clone()))
            .audience(audience)
            .subject(UcanSubject::Specific(space.clone()))
            .command(command)
            .expiration(
                Timestamp::new(UNIX_EPOCH + Duration::from_secs(expires))
                    .map_err(|error| error.to_string())?,
            )
            .try_build()
            .await
            .map_err(|error| format!("could not sign the view's delegation: {error}"))?;
        chains.push(base.push(leaf).map_err(|error| error.to_string())?);
    }
    encode(&chains)
}

fn read() -> Vec<String> {
    vec!["use".into(), "get".into()]
}

/// A chain's command, as segments.
fn command_of(chain: &DelegationChain) -> Vec<String> {
    chain
        .ability()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

/// [`OUTLINE`]: the view tree of `space`, as its writer reads it. A
/// granted space's request carries its grants and address, as a query's
/// does, so its writer mounts and pulls it the same way.
async fn outline<W: Writers>(
    arguments: &Value,
    access: &SpaceAccess,
    writers: &W,
    verified: &Verified,
    bearer: &str,
    mcp_session: &str,
    space: &Did,
) -> Value {
    let route = arguments["route"].as_str().unwrap_or("").trim();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if !route.is_empty() {
        query.append_pair("route", route);
    }
    let query = query.finish();
    let uri = if query.is_empty() {
        "/api/outline".to_owned()
    } else {
        format!("/api/outline?{query}")
    };
    let mut request = Request::builder().method(Method::GET).uri(uri);
    if !access.agent {
        request = request.header(GRANTS_HEADER, access.grants.join(","));
        if let Some(address) = &access.address {
            request = request.header(ADDRESS_HEADER, address);
        }
    }
    let Ok(request) = request.body(Body::empty()) else {
        return error("The space's directory entry is malformed.");
    };
    let response = writers
        .call(verified, bearer, mcp_session, space, request)
        .await;
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    if !status.is_success() {
        let reason = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|body| body["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
        return error(&format!("Could not outline the space ({status}): {reason}"));
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(outline) => json!({
            "content": [{ "type": "text", "text": outline.to_string() }],
            "structuredContent": outline,
            "isError": false,
        }),
        Err(problem) => error(&format!(
            "The space answered an unreadable outline: {problem}"
        )),
    }
}

fn error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

#[cfg(test)]
mod tests;
