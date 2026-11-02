//! Joining a space from the chat: the agent reads an invite link the
//! person pasted, and the person lets it in from Tonk, through them.
//!
//! - `tonk_join_space` (the model's) reads the link and changes nothing.
//!   Its result renders the join card, which carries the Tonk page that
//!   joins (`/settings/agent-join`, the link in its fragment).
//! - On that page the person's account claims the link, as any join
//!   does, and signs the agent's grant with its own key behind the
//!   passkey (`router::join_for_agent`). The grant runs
//!   `space -> ... -> link key -> account -> agent space`, so the person
//!   can revoke it, and an admin who removes the person removes it too.
//! - `tonk_join_status` (the card's, app-only) asks the agent space
//!   whether the grant has arrived, so the card can tell the model.
//!
//! Only the person's browser holds their keys, which is why the join
//! leaves the chat: the card runs on the host's origin and can reach
//! neither the account's key nor its passkey.

use axum::body::Body;
use axum::http::{Method, Request, header};
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tonk_invite::Invite;
use tonk_schema::prelude::DidExt as _;

use super::access::reaches;
use super::mcp::{ACCESS_ROUTE, AgentAccess, Verified, Writers};

/// The model-visible tool that reads an invite link and shows the card.
pub const JOIN: &str = "tonk_join_space";

/// The app-only tool the card asks with whether the agent has joined.
pub const STATUS: &str = "tonk_join_status";

/// The Tonk page that joins: the account claims the link and lets the
/// agent in. Its fragment carries the agent space's key and the link.
pub const JOIN_PAGE: &str = "/settings/agent-join";

/// The join card's resource.
pub const CARD_URI: &str = "ui://tonk/join-v1.html";

/// The agent space writer's route that reads a link (`POST`, a
/// [`LinkRequest`] as JSON, answered with a [`LinkAnswer`]).
pub const LINK_ROUTE: &str = "/api/access/link";

/// The MIME type MCP Apps hosts render.
const CARD_MIME: &str = "text/html;profile=mcp-app";

/// The join card.
const CARD: &str = include_str!("join/card.html");

/// The longest link accepted, in bytes.
const MAX_LINK: usize = 4096;

/// What a link offers the agent: the space, and whether it may write.
#[derive(Debug)]
pub struct Offer {
    /// The space the link opens.
    pub subject: Did,
    /// The space's name, as the invitation records it.
    pub name: Option<String>,
    /// Whether the link allows writing as well as reading.
    pub write: bool,
    /// Where the space syncs.
    pub remote: url::Url,
}

impl Offer {
    /// Read a parsed `invite` as what it offers an agent.
    ///
    /// An invite addressed to one person is offered too: the person
    /// claims it in Tonk, and Tonk refuses it there if it is not theirs.
    ///
    /// # Errors
    ///
    /// When the invite names no place to sync, has run out, or grants not
    /// even reading.
    pub fn of(invite: Invite) -> Result<Self, String> {
        let chain = &invite.chain;
        let now = Timestamp::now().to_unix();
        if chain.expiration().is_some_and(|at| at.to_unix() <= now) {
            return Err("This invitation has expired. Ask for a new link.".into());
        }
        let allows = |verb| {
            chain
                .proofs()
                .all(|delegation| reaches(&delegation.command().0, verb))
        };
        if !allows("get") {
            return Err("This invitation does not let anyone read the space.".into());
        }
        let remote = match tonk_invite::home_address(chain) {
            Ok(Some(remote)) => remote,
            _ => invite
                .remote_url
                .clone()
                .ok_or_else(|| "This invitation names no place the space syncs.".to_owned())?,
        };
        Ok(Self {
            subject: invite.subject().clone(),
            name: invite.space_name.clone(),
            write: allows("put"),
            remote,
        })
    }
}

/// Read `link`: expand a `/@/` shortcut, claim a ticket link's ticket, or
/// parse an `access=` link, and answer what it offers. Changes nothing.
///
/// # Errors
///
/// A sentence for the person, never one that repeats the link.
pub async fn read_link(link: &str) -> Result<Offer, String> {
    let link = link.trim();
    if link.is_empty() || link.len() > MAX_LINK {
        return Err("Paste the whole Tonk invite link.".into());
    }
    let parsed = url::Url::parse(link).map_err(|_| "That is not a Tonk invite link.".to_owned())?;
    let secure = match parsed.scheme() {
        "https" => parsed.host().is_some(),
        "http" => matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        _ => false,
    };
    if !secure {
        return Err("That is not a Tonk invite link.".into());
    }
    let link = if parsed.path().starts_with("/@/") {
        expand(link).await?
    } else {
        link.to_owned()
    };
    let invite = super::super::join::parse_invite(&link)
        .await
        .map_err(|failure| failure.kind().message().to_string())?;
    Offer::of(invite)
}

/// Where a shortcut lands, with the link's fragment (its key) put back:
/// the fragment never leaves this host.
async fn expand(link: &str) -> Result<String, String> {
    let failed = || "Could not open the shortened link. Ask for the full one.".to_owned();
    let mut sent = url::Url::parse(link).map_err(|_| failed())?;
    sent.set_fragment(None);
    let landed = landing(sent.as_str()).await.ok_or_else(failed)?;
    tonk_invite::shortcut::resolve_location(link, &landed).map_err(|_| failed())
}

/// Where a `HEAD` of `url` lands once its redirects are followed.
#[cfg(not(target_arch = "wasm32"))]
async fn landing(url: &str) -> Option<String> {
    let response = reqwest::Client::new().head(url).send().await.ok()?;
    Some(response.url().to_string())
}

/// Where a `HEAD` of `url` lands once its redirects are followed, by the
/// global `fetch` (a Workers isolate has no window or service worker).
#[cfg(target_arch = "wasm32")]
async fn landing(url: &str) -> Option<String> {
    use wasm_bindgen::JsCast as _;
    let init = web_sys::RequestInit::new();
    init.set_method("HEAD");
    let request = web_sys::Request::new_with_str_and_init(url, &init).ok()?;
    let global = js_sys::global();
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &"fetch".into())
        .ok()?
        .dyn_into()
        .ok()?;
    let promise: js_sys::Promise = fetch.call1(&global, &request).ok()?.dyn_into().ok()?;
    let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    Some(response.url())
}

/// Tools appended to `tools/list`.
pub fn tools() -> Vec<Value> {
    let link = json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_LINK,
        "description": "The whole Tonk invite link the person pasted, including the part after #.",
    });
    vec![
        json!({
            "name": JOIN,
            "title": "Join a Tonk space",
            "description": "Ask the person to let you join a Tonk space from an invite link \
                they gave you. Shows a card in the chat that opens Tonk, where the person \
                joins the space and lets you in through them, with the access they choose \
                (read, or read and write). You join as this connection's agent, not as them. \
                This call joins nothing. When they are done, the card tells you, and the \
                space is in tonk_list_spaces. If the host shows no card, give the person \
                the join page in this result's structured content (open).",
            "inputSchema": {
                "type": "object",
                "properties": { "link": link },
                "required": ["link"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": true },
            "_meta": {
                "ui": { "resourceUri": CARD_URI },
                "openai/outputTemplate": CARD_URI,
                "openai/widgetAccessible": true,
            },
        }),
        json!({
            "name": STATUS,
            "description": "Whether the agent has joined a space yet. Called by the card.",
            "inputSchema": {
                "type": "object",
                "properties": { "space": { "type": "string", "maxLength": 256 } },
                "required": ["space"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true },
            "_meta": {
                "ui": { "visibility": ["app"] },
                "openai/visibility": "private",
            },
        }),
    ]
}

/// The Tonk page at `origin` where the person lets `agent` in with
/// `link`. The link stays in the fragment, which no server is sent.
pub fn join_page(origin: &str, agent: &Did, link: &str) -> String {
    let fragment = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("agent", agent.repo_key())
        .append_pair("link", link.trim())
        .finish();
    format!("{}{JOIN_PAGE}#{fragment}", origin.trim_end_matches('/'))
}

/// Resources served by `resources/list`.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": CARD_URI,
        "name": "tonk-join",
        "title": "Join a Tonk space",
        "mimeType": CARD_MIME,
    })]
}

/// The `resources/read` result for `uri`, or `None` when it is not the
/// card. The card loads nothing: it talks only to the host.
pub fn read_resource(uri: &str) -> Option<Value> {
    if uri != CARD_URI {
        return None;
    }
    Some(json!({
        "contents": [{
            "uri": CARD_URI,
            "mimeType": CARD_MIME,
            "text": CARD,
            "_meta": {
                "ui": { "csp": { "connectDomains": [], "resourceDomains": [] }, "prefersBorder": true },
                "openai/widgetCSP": { "connect_domains": [], "resource_domains": [] },
            },
        }],
    }))
}

/// The result of the join tool `name`, or `None` when it is not one.
///
/// The link is read by the agent space's own writer ([`LINK_ROUTE`]): it
/// reaches the access service the way every write does, which a front
/// serving a request the access service forwarded cannot (its claim
/// answered `404`).
#[allow(clippy::too_many_arguments)]
pub async fn call<W: Writers>(
    name: &str,
    arguments: &Value,
    verified: &Verified,
    writers: &W,
    bearer: &str,
    mcp_session: &str,
    origin: &str,
) -> Option<Value> {
    if name != JOIN && name != STATUS {
        return None;
    }
    let agent = verified.agent().clone();
    let ask = |request: Request<Body>| writers.call(verified, bearer, mcp_session, &agent, request);
    let held = || async { answer::<AgentAccess>(ask(get(ACCESS_ROUTE)).await).await };
    if name == STATUS {
        let Some(space) = arguments["space"]
            .as_str()
            .and_then(|space| space.parse::<Did>().ok())
        else {
            return Some(error("Name the space as space."));
        };
        let held = match held().await {
            Ok(access) => access,
            Err(message) => return Some(error(&message)),
        };
        let joined = joined(&held, &space);
        return Some(json!({
            "content": [{
                "type": "text",
                "text": match joined {
                    Some(access) => format!("Joined space {space} with {access} access."),
                    None => format!("Not joined to space {space} yet."),
                },
            }],
            "structuredContent": { "space": space.to_string(), "joined": joined },
            "isError": false,
        }));
    }
    let Some(link) = arguments["link"].as_str() else {
        return Some(error("Provide the whole invite link as link."));
    };
    let body = serde_json::to_string(&LinkRequest {
        link: link.to_owned(),
    })
    .expect("a link request serializes");
    let request = Request::builder()
        .method(Method::POST)
        .uri(LINK_ROUTE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("static request");
    let response = ask(request).await;
    let offer: LinkAnswer = if response.status().is_success() {
        match answer(response).await {
            Ok(offer) => offer,
            Err(message) => return Some(error(&message)),
        }
    } else {
        // The writer's refusal is a sentence for the person.
        return Some(error(&refusal(response).await));
    };
    let label = offer
        .name
        .clone()
        .unwrap_or_else(|| "this space".to_owned());
    let held = match held().await {
        Ok(access) => access,
        Err(message) => return Some(error(&message)),
    };
    let joined = offer
        .space
        .parse::<Did>()
        .ok()
        .and_then(|space| joined(&held, &space));
    let me = held.get(&agent).and_then(|own| own.name.clone());
    let text = match joined {
        Some("read") if offer.write => format!(
            "You already have read access to {label}, space {}. The card lets the person \
             give you read and write access too, in Tonk; you do not have it until they do.",
            offer.space
        ),
        Some(access) => format!(
            "You already have {} access to {label}, space {}. Nothing to ask.",
            if access == "write" {
                "read and write"
            } else {
                "read"
            },
            offer.space
        ),
        None => format!(
            "Showing the invitation to {label}. You have not joined: the card opens Tonk, \
             where the person joins the space and lets you in through them, with the access \
             they choose. When they do, use space {}.",
            offer.space
        ),
    };
    Some(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": {
            "space": offer.space,
            "name": offer.name,
            "write": offer.write,
            "joined": joined,
            "open": join_page(origin, &agent, link),
            "agent": { "did": agent.to_string(), "name": me },
        },
        "isError": false,
    }))
}

/// The access the agent holds to `space`: `read`, `write` for read and
/// write, or none.
fn joined(held: &AgentAccess, space: &Did) -> Option<&'static str> {
    held.get(space)
        .filter(|access| access.read)
        .map(|access| if access.write { "write" } else { "read" })
}

/// What a front asks the agent space's writer about a link.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LinkRequest {
    /// The link, as the person pasted it.
    pub link: String,
}

/// What the agent space's writer answers about a link.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LinkAnswer {
    /// The space the link opens.
    pub space: String,
    /// Its name, as the invitation records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether the link allows writing.
    pub write: bool,
}

/// The reason a writer gives for refusing, as its `error`.
async fn refusal(response: axum::http::Response<Body>) -> String {
    let body = response
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "The agent's own space did not answer. Try again.".to_owned())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("static request")
}

async fn answer<T: serde::de::DeserializeOwned>(
    response: axum::http::Response<Body>,
) -> Result<T, String> {
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
            .and_then(|value| value["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| String::from_utf8_lossy(&body).into_owned());
        return Err(format!("the agent's own space answered {status}: {reason}"));
    }
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

fn error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

#[cfg(test)]
mod tests;
