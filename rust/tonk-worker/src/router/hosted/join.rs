//! Joining a space from the chat: the agent reads an invite link the
//! person pasted, and the person lets it join, with the access they
//! choose, in a card the host shows inline (MCP Apps).
//!
//! - `tonk_join_space` (the model's) reads the link and changes nothing.
//!   Its result renders the join card.
//! - `tonk_join_accept` (the card's, app-only) joins: the link's key
//!   delegates to the agent space only the scopes of the chosen access,
//!   and the agent space's writer retains the chains and a directory
//!   entry for the space, as the approval page does.
//!
//! The access holds because access is what the retained chains prove: a
//! read-only join holds only `/use/get/...` leaves, so the access service
//! refuses a write whatever the model asks for. The model cannot choose
//! the access itself, since the host does not offer it an app-only tool.
//! A host that ignored that could still never exceed what the link
//! allows, which is what any holder of the link may do.

use axum::body::Body;
use axum::http::{Method, Request, header};
use dialog_ucan_core::time::Timestamp;
use dialog_ucan_core::{Container, DelegationChain, Tag};
use dialog_varsig::Did;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tonk_invite::connection::{AgentAccess as Level, DEFAULT_GRANT_TTL_SECONDS};
use tonk_invite::{Invite, InviteAudience};

use super::access::{DirectoryEntry, reaches};
use super::mcp::{ACCESS_ROUTE, AgentAccess, Verified, Writers};

/// The model-visible tool that reads an invite link and shows the card.
pub const JOIN: &str = "tonk_join_space";

/// The app-only tool the card joins with.
pub const ACCEPT: &str = "tonk_join_accept";

/// The join card's resource.
pub const CARD_URI: &str = "ui://tonk/join-v1.html";

/// The agent space writer's route that reads a link, and joins with it
/// when asked (`POST`, a [`LinkRequest`] as JSON, answered with a
/// [`LinkAnswer`]).
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
    invite: Invite,
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
    /// # Errors
    ///
    /// When the invite is addressed to someone else (only an open invite
    /// can be redelegated to the agent), names no place to sync, has run
    /// out, or grants not even reading.
    pub fn of(invite: Invite) -> Result<Self, String> {
        if !matches!(invite.audience, InviteAudience::Open { .. }) {
            return Err(
                "This invitation is addressed to one person, so an agent cannot join with it. \
                 Ask for a share link instead."
                    .into(),
            );
        }
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
            invite,
        })
    }

    /// The chains that let `agent` reach the space with `access`: the
    /// link's key delegates each scope of `access` to `agent`, expiring
    /// when the link does or after 90 days, whichever is first.
    ///
    /// # Errors
    ///
    /// When `access` asks to write and the link allows only reading, or a
    /// delegation does not build.
    pub async fn grant(
        &self,
        agent: &Did,
        access: Level,
        now: Timestamp,
    ) -> Result<Vec<DelegationChain>, String> {
        if access == Level::ReadWrite && !self.write {
            return Err("This invitation lets an agent read the space, not change it.".into());
        }
        let signer = self
            .invite
            .signer()
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "This invitation cannot be passed on to an agent.".to_owned())?;
        let scopes = tonk_invite::connection::agent_grant_scopes(&self.subject, access);
        let lifetime = now.to_unix() + DEFAULT_GRANT_TTL_SECONDS;
        let until = self
            .invite
            .chain
            .expiration()
            .map_or(lifetime, |at| at.to_unix().min(lifetime));
        let expires = Timestamp::try_from(i128::from(until)).map_err(|error| error.to_string())?;
        let ancestors = vec![self.invite.chain.clone(); scopes.len()];
        let bundle = super::super::agent_connections::issue_to_recipient(
            agent,
            signer.into(),
            ancestors,
            &scopes,
            &self.remote,
            now,
            expires,
        )
        .await
        .map_err(|error| format!("The invitation could not be passed on: {error}"))?;
        Ok(bundle.chains().to_vec())
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

/// What a join keeps in the agent space.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GrantBody {
    /// The space.
    pub space: String,
    /// Its name, for the agent space's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where it syncs.
    pub address: String,
    /// Each chain `space -> ... -> agent space`, a base64url container.
    pub chains: Vec<String>,
}

impl GrantBody {
    /// The body for `chains` that reach the space `offer` names.
    pub fn new(offer: &Offer, chains: &[DelegationChain]) -> Result<Self, String> {
        let chains = chains
            .iter()
            .map(|chain| {
                let bytes = Container::from(chain)
                    .encode(Tag::Base64Url)
                    .map_err(|error| error.to_string())?;
                String::from_utf8(bytes).map_err(|error| error.to_string())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            space: offer.subject.to_string(),
            name: offer.name.clone(),
            address: offer.remote.to_string(),
            chains,
        })
    }

    /// The chains and directory entry, checked: every chain reaches
    /// `agent`, and grants the space named and no other.
    ///
    /// # Errors
    ///
    /// When a chain does not decode, is addressed elsewhere, or grants
    /// another space or the agent space itself.
    pub(crate) fn open(
        &self,
        agent: &Did,
    ) -> Result<(Vec<DelegationChain>, DirectoryEntry), String> {
        let space: Did = self
            .space
            .parse()
            .map_err(|_| "the space is not a DID".to_owned())?;
        if &space == agent {
            return Err("the agent space is not granted to itself".into());
        }
        if self.chains.is_empty() {
            return Err("no grants".into());
        }
        let mut chains = Vec::new();
        for encoded in &self.chains {
            let container = Container::decode(encoded.as_bytes())
                .map_err(|error| format!("a grant does not decode: {error}"))?;
            let chain = DelegationChain::try_from(container)
                .map_err(|error| format!("a grant is not a chain: {error}"))?;
            if chain.audience() != agent || chain.subject() != Some(&space) {
                return Err("a grant is not this space's to the agent".into());
            }
            chains.push(chain);
        }
        Ok((
            chains,
            DirectoryEntry {
                space,
                name: self.name.clone(),
                address: Some(self.address.clone()),
            },
        ))
    }
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
                they gave you. Shows a card in the chat where they choose your access (read, \
                or read and write) and join; you are the one who joins, as this connection's \
                agent, not as them. This call joins nothing. When they join, the card tells \
                you, and the space is in tonk_list_spaces. If the host shows no card, ask \
                the person to grant you the space in Tonk instead.",
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
            "name": ACCEPT,
            "description": "Join the space with the access the person chose. Called by the card.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "link": link,
                    "access": { "type": "string", "enum": ["read", "write"] },
                },
                "required": ["link", "access"],
                "additionalProperties": false,
            },
            "_meta": {
                "ui": { "visibility": ["app"] },
                "openai/visibility": "private",
            },
        }),
    ]
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
/// card. The card talks only to the host; from `origin`, the provider the
/// host reached, it loads nothing but Tonk's fonts.
pub fn read_resource(uri: &str, origin: &str) -> Option<Value> {
    if uri != CARD_URI {
        return None;
    }
    let origin = origin.trim_end_matches('/');
    Some(json!({
        "contents": [{
            "uri": CARD_URI,
            "mimeType": CARD_MIME,
            "text": super::in_agent::splice(CARD, origin),
            "_meta": {
                "ui": {
                    "csp": { "connectDomains": [], "resourceDomains": [origin] },
                    "prefersBorder": true,
                },
                "openai/widgetCSP": { "connect_domains": [], "resource_domains": [origin] },
            },
        }],
    }))
}

/// The result of the join tool `name`, or `None` when it is not one.
///
/// The link is read, and joined, by the agent space's own writer
/// ([`LINK_ROUTE`]): it reaches the access service the way every write
/// does, which a front serving a request the access service forwarded
/// cannot (its claim answered `404`).
pub async fn call<W: Writers>(
    name: &str,
    arguments: &Value,
    verified: &Verified,
    writers: &W,
    bearer: &str,
    mcp_session: &str,
) -> Option<Value> {
    if name != JOIN && name != ACCEPT {
        return None;
    }
    let Some(link) = arguments["link"].as_str() else {
        return Some(error("Provide the whole invite link as link."));
    };
    let access = if name == ACCEPT {
        match arguments["access"].as_str() {
            Some(access @ ("read" | "write")) => Some(access.to_owned()),
            _ => return Some(error("Choose read or write access.")),
        }
    } else {
        None
    };
    let agent = verified.agent().clone();
    let ask = |request: Request<Body>| writers.call(verified, bearer, mcp_session, &agent, request);
    let body = serde_json::to_string(&LinkRequest {
        link: link.to_owned(),
        access,
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
    if let Some(granted) = &offer.granted {
        let words = if granted == "write" {
            "read and write"
        } else {
            "read"
        };
        return Some(json!({
            "content": [{
                "type": "text",
                "text": format!("Joined {label} with {words} access. Use space {}.", offer.space),
            }],
            "structuredContent": {
                "space": offer.space,
                "name": offer.name,
                "access": granted,
            },
            "isError": false,
        }));
    }
    let held = match answer::<AgentAccess>(ask(get(ACCESS_ROUTE)).await).await {
        Ok(access) => access,
        Err(message) => return Some(error(&message)),
    };
    let joined = offer
        .space
        .parse::<Did>()
        .ok()
        .and_then(|space| held.get(&space).cloned())
        .filter(|access| access.read)
        .map(|access| if access.write { "write" } else { "read" });
    let me = held.get(&agent).and_then(|own| own.name.clone());
    let text = match joined {
        Some("read") if offer.write => format!(
            "You already have read access to {label}, space {}. The card lets the person \
             give you read and write access too; you do not have it until they do.",
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
            "Showing the invitation to {label}. You have not joined: the person chooses \
             your access in the card and joins from there. When they do, use space {}.",
            offer.space
        ),
    };
    Some(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": {
            "link": link,
            "space": offer.space,
            "name": offer.name,
            "write": offer.write,
            "joined": joined,
            "agent": { "did": agent.to_string(), "name": me },
        },
        "isError": false,
    }))
}

/// What a front asks the agent space's writer about a link.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LinkRequest {
    /// The link, as the person pasted it.
    pub link: String,
    /// `read` or `write` to join with that access; absent to only read
    /// what the link offers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
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
    /// The access just granted, when the request asked to join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted: Option<String>,
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
