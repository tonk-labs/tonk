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
            "description": "Show one view of a space in the chat, live, as the space renders \
                it: pass a route from tonk_outline as view, or a path within the space such as \
                /notebook/<entity> (entity route parameters must be actual entity URIs returned \
                by tonk_query, never titles or names). The whole space (path / with no view) is \
                not shown in the chat; the person gets a card that opens it in Tonk. The \
                person can change the space from the view wherever this connection may write \
                it; a space it may only read is shown read-only. The result's text says what \
                the view shows. Showing a route does not verify that it rendered.",
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
                    "size": {
                        "type": "string",
                        "enum": SIZES,
                        "default": "card",
                        "description": "How big the view is in the chat: strip (one line \
                            with a link, when there is nothing to look at), card (a short \
                            summary, 4 to 5 rows and at most 2 actions), carousel (3 to 8 \
                            like items side by side, 1 action each) or full (the whole \
                            view, full screen).",
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
                children, and a route that shows it alone. A list's node also has items: each \
                record it rendered, with its fields and a route that shows that one record as \
                it looks in the list, controls included. Without route the tree starts at the \
                space's home. Pass a node's or an item's route to tonk_open_space as view to \
                show just that part, for example one list from a larger page or one to-do \
                from a list.",
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
    let html = super::in_agent::splice(RELAY, origin)
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
    let reachable = view_reaches(&remote, origin);
    if !reachable && name != OPEN {
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
        return Some(
            open(
                arguments,
                &access,
                writers,
                verified,
                bearer,
                mcp_session,
                &space,
                Shown {
                    edit,
                    remote: &remote,
                    reachable,
                    origin,
                },
            )
            .await,
        );
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

/// How [`OPEN`] shows a route: what the connection may do there, where
/// the space syncs, and whether a view from `origin` reaches it.
struct Shown<'a> {
    edit: bool,
    remote: &'a str,
    reachable: bool,
    origin: &'a str,
}

/// The sizes a shown view may take inline, or full screen.
const SIZES: [&str; 4] = ["strip", "card", "carousel", "full"];

/// [`OPEN`]: what the relay shows of `space`, and the text version of it
/// the model reads.
///
/// One view, or a page by path, shows live (`"show": "live"`) at the
/// size asked for. The whole space (`/` with no view) is never shown in
/// the chat: it is a hand-off into Tonk (`"handoff"`). A strip, or a
/// space a view cannot reach from here, is one line with a link
/// (`"strip"`). Every answer carries the link into Tonk, so a host that
/// shows no card still offers it.
#[allow(clippy::too_many_arguments)]
async fn open<W: Writers>(
    arguments: &Value,
    access: &SpaceAccess,
    writers: &W,
    verified: &Verified,
    bearer: &str,
    mcp_session: &str,
    space: &Did,
    shown: Shown<'_>,
) -> Value {
    let path = arguments["path"].as_str().unwrap_or("/");
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.chars().any(|c| c == '\\' || c.is_control())
    {
        return error("Provide a path within the space, such as /.");
    }
    let size = match arguments["size"].as_str() {
        None => "card",
        Some(size) if SIZES.contains(&size) => size,
        Some(other) => {
            return error(&format!(
                "`{other}` is not a size. Use strip, card, carousel or full."
            ));
        }
    };
    let route = arguments["view"].as_str().map(str::trim).unwrap_or("");
    let view = if route.is_empty() {
        Value::Null
    } else if route.len() > 2048 {
        return error("Provide a view route from tonk_outline, up to 2 KB.");
    } else {
        match tonk_render::RenderRoute::parse(route) {
            // The home mounts on each peer's own replica; the path
            // shows it, as the outline's empty home route means.
            Ok(route) if super::super::view_tree::is_home(&route) => Value::Null,
            Ok(route) => json!({
                "model": route.model,
                "entity": route.entity,
                "facet": route.view,
            }),
            Err(problem) => {
                return error(&format!(
                    "`{route}` is not a view route ({problem}). Use a route from tonk_outline."
                ));
            }
        }
    };
    let path = if view.is_null() && !route.is_empty() {
        "/"
    } else {
        path
    };
    let whole = view.is_null() && path == "/";
    // Where the space opens in Tonk: on the deployment it syncs with.
    let home = url::Url::parse(shown.remote)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| shown.origin.trim_end_matches('/').to_owned());
    // A view opens where the browser routes it, `/entity@model!view`; a
    // page by its path.
    let link = if !view.is_null() {
        format!("{home}/space/{space}/{}", route_path(route))
    } else if path == "/" {
        format!("{home}/space/{space}")
    } else {
        format!("{home}/space/{space}{path}")
    };
    // What Tonk's line names, and the text version: the view's outline.
    // A view whose outline cannot be read is still shown; the line then
    // names the space alone.
    let outline = if view.is_null() || !shown.reachable || size == "strip" {
        None
    } else {
        read_outline(route, access, writers, verified, bearer, mcp_session, space)
            .await
            .ok()
    };
    let node = outline.as_ref().map(|outline| &outline["outline"]);
    let view_name = node
        .and_then(|node| node["name"].as_str())
        .map(str::to_owned);
    let show = if whole {
        "handoff"
    } else if !shown.reachable || size == "strip" {
        "strip"
    } else {
        "live"
    };
    let space_name = access.name.as_deref().unwrap_or("the space");
    let place = match &view_name {
        Some(name) => format!("{space_name} › {name}"),
        None if view.is_null() && path != "/" => format!("{space_name} {path}"),
        None if view.is_null() => space_name.to_owned(),
        None => format!("a view of {space_name}"),
    };
    let role = role(access);
    let mut text = match show {
        "handoff" => format!(
            "The whole of {space_name} is not shown in the chat: the person sees a card that \
             opens it in Tonk ({role}). To show part of it here, pass a view route from \
             tonk_outline."
        ),
        "strip" if !shown.reachable => format!(
            "{place} is hosted at {home}, and a view from {} cannot reach it, so the person \
             sees a link that opens it in their browser instead.",
            shown.origin.trim_end_matches('/')
        ),
        "strip" => format!("The person sees a link to {place} that opens it in their browser."),
        _ => format!(
            "Showing {place} {}, live ({role}): the person can look and {}. Rendering is \
             not verified by this response.",
            if size == "full" {
                "full screen".to_owned()
            } else {
                format!("inline, as a {size}")
            },
            if shown.edit {
                "change it there"
            } else {
                "not change it"
            },
        ),
    };
    if let Some(node) = node {
        let mut lines = String::new();
        describe(node, 0, &mut lines);
        if !lines.is_empty() {
            text.push_str("\nIt shows:\n");
            text.push_str(&lines);
        }
    }
    text.push_str(&format!("\nOpen in Tonk: {link}"));
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": {
            "subject": space.to_string(),
            "path": path,
            "view": view,
            "edit": shown.edit,
            "remote": shown.remote,
            "name": access.name,
            "viewName": view_name,
            "role": role,
            "show": show,
            "size": size,
            "link": link,
        },
        "isError": false,
    })
}

/// `route` as a path segment run: what would end the path or break it is
/// escaped, and the route's own `/` and `@` stay as the browser routes them.
fn route_path(route: &str) -> String {
    route
        .chars()
        .map(|c| match c {
            '%' => "%25".to_owned(),
            '?' => "%3F".to_owned(),
            '#' => "%23".to_owned(),
            c if c.is_whitespace() || c.is_control() => {
                let mut bytes = [0; 4];
                c.encode_utf8(&mut bytes)
                    .bytes()
                    .map(|byte| format!("%{byte:02X}"))
                    .collect()
            }
            c => c.to_string(),
        })
        .collect()
}

/// Most characters of the text version of a view.
const DESCRIBE_LIMIT: usize = 4000;

/// Most records of one view the text version lists.
const DESCRIBE_ITEMS: usize = 20;

/// The text version of an outline `node`: one line for the view and each
/// view it nests, with how many records it shows, and one line for each
/// record it lists. Cut at [`DESCRIBE_LIMIT`]; the rest is in
/// `tonk_outline`.
fn describe(node: &Value, depth: usize, out: &mut String) {
    if out.len() >= DESCRIBE_LIMIT {
        return;
    }
    let indent = "  ".repeat(depth);
    let name = node["name"]
        .as_str()
        .or_else(|| node["model"].as_str())
        .unwrap_or("a view");
    let records = node["records"].as_u64().unwrap_or(0);
    let mut line = format!(
        "{indent}- {name}: {records} record{}",
        if records == 1 { "" } else { "s" }
    );
    if let Some(problem) = node["error"].as_str() {
        line.push_str(&format!(" (did not render: {problem})"));
    }
    push(out, &line);
    let items = node["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    for item in items.iter().take(DESCRIBE_ITEMS) {
        let fields = item["fields"]
            .as_object()
            .map(|fields| {
                fields
                    .iter()
                    .map(|(field, value)| match value {
                        Value::String(text) => format!("{field}: {text}"),
                        other => format!("{field}: {other}"),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let entity = item["entity"].as_str().unwrap_or("");
        push(
            out,
            &format!(
                "{indent}  - {}",
                if fields.is_empty() { entity } else { &fields }
            ),
        );
    }
    if items.len() > DESCRIBE_ITEMS {
        push(
            out,
            &format!("{indent}  - and {} more", items.len() - DESCRIBE_ITEMS),
        );
    }
    for child in node["children"].as_array().into_iter().flatten() {
        describe(child, depth + 1, out);
    }
}

fn push(out: &mut String, line: &str) {
    if out.len() + line.len() + 1 > DESCRIBE_LIMIT {
        if !out.ends_with("…\n") {
            out.push_str("…\n");
        }
        return;
    }
    out.push_str(line);
    out.push('\n');
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

/// The role Tonk's line names for a connection with `access`: `write`
/// wherever it may write, else `read`. Admin is not modelled for a
/// connection yet; it will be a third answer here.
fn role(access: &SpaceAccess) -> &'static str {
    if access.agent || access.write {
        "write"
    } else {
        "read"
    }
}

/// [`OUTLINE`]: the view tree of `space`, as its writer reads it.
async fn outline<W: Writers>(
    arguments: &Value,
    access: &SpaceAccess,
    writers: &W,
    verified: &Verified,
    bearer: &str,
    mcp_session: &str,
    space: &Did,
) -> Value {
    let route = arguments["route"].as_str().unwrap_or("");
    match read_outline(route, access, writers, verified, bearer, mcp_session, space).await {
        Ok(outline) => json!({
            "content": [{ "type": "text", "text": outline.to_string() }],
            "structuredContent": outline,
            "isError": false,
        }),
        Err(message) => error(&message),
    }
}

/// The view tree of `space` from `route` (the home when empty), as its
/// writer reads it. A granted space's request carries its grants and
/// address, as a query's does, so its writer mounts and pulls it the same
/// way.
async fn read_outline<W: Writers>(
    route: &str,
    access: &SpaceAccess,
    writers: &W,
    verified: &Verified,
    bearer: &str,
    mcp_session: &str,
    space: &Did,
) -> Result<Value, String> {
    let route = route.trim();
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
        return Err("The space's directory entry is malformed.".into());
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
        return Err(format!("Could not outline the space ({status}): {reason}"));
    }
    serde_json::from_slice::<Value>(&body)
        .map_err(|problem| format!("The space answered an unreadable outline: {problem}"))
}

fn error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

#[cfg(test)]
mod tests;
