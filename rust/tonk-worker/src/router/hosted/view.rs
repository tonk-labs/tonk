//! Inline views of a space in MCP Apps hosts (plan #1200, stage 7), served
//! by the space writer.
//!
//! `tonk_open_space` names a `ui://` relay page and returns, in `_meta` (which a
//! host hands the view and not the model), a view ticket: the session's
//! bearer, its `Mcp-Session-Id` and one space, sealed under a key only the
//! host holds and good for [`TICKET_LIFETIME`]. The relay presents it at
//! `/space-api/`, where the Host Worker opens it, verifies the bearer again
//! and hands the guest's request to the writer of that space and session.
//! So a view's edits are serialized with the agent's own writes and signed
//! as the same writer.
//!
//! The relay renews its ticket through `tonk_view_ticket`, an app-only
//! tool, so the bearer never leaves the host in the clear and a view
//! outlives no session: a ticket for a revoked or expired token opens, but
//! its bearer no longer verifies.
//!
//! The relay is the guest bridge `mcp/apps/ui/worker-space.html` already
//! speaks, with the guest loaded inline (`srcdoc`), since Claude frames no
//! origin outside its own list.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::mcp::Verified;

/// The relay page's resource.
pub const RELAY_URI: &str = "ui://tonk/space-view-v1.html";

/// The MIME type MCP Apps hosts render.
const RELAY_MIME: &str = "text/html;profile=mcp-app";

/// The relay page, with its origin left to fill in.
const RELAY: &str = include_str!("view/relay.html");

/// How long a view ticket is good for, in seconds.
pub const TICKET_LIFETIME: u64 = 10 * 60;

/// The context ticket keys are derived under, apart from the session MAC's.
const TICKET_KEY_CONTEXT: &str = "tonk 2026-10-29 hosted agent view ticket v1";

/// Bound into every ticket's seal, so nothing else sealed under the key
/// opens as one.
const TICKET_AAD: &[u8] = b"tonk view ticket v1";

/// Bytes of nonce before a ticket's ciphertext.
const NONCE_BYTES: usize = 12;

/// The model-visible tool that shows a space inline: a route of it, as the
/// hosted MCP server's `tonk_open_space` does, not the local peer's
/// `tonk_show` (a live query result).
pub const SHOW: &str = "tonk_open_space";

/// The app-only tool the relay renews its ticket with.
pub const RENEW: &str = "tonk_view_ticket";

/// The key view tickets are sealed under, derived from the host's secret.
#[derive(Clone)]
pub struct TicketKey([u8; 32]);

impl TicketKey {
    /// The ticket key for the host secret `secret`.
    pub fn derive(secret: &str) -> Self {
        Self(blake3::derive_key(TICKET_KEY_CONTEXT, secret.as_bytes()))
    }

    /// Seal `ticket`.
    pub fn seal(&self, ticket: &Ticket) -> String {
        let cipher = Aes256Gcm::new_from_slice(&self.0).expect("32-byte key");
        let nonce: [u8; NONCE_BYTES] = rand::random();
        let plain = serde_json::to_vec(ticket).expect("ticket serializes");
        let sealed = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plain,
                    aad: TICKET_AAD,
                },
            )
            .expect("sealing never fails for a valid key");
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&sealed);
        B64.encode(bytes)
    }

    /// Open `sealed`, refusing one that does not open under this key or
    /// whose time is up at `now` (Unix seconds).
    pub fn open(&self, sealed: &str, now: u64) -> Result<Ticket, String> {
        let bytes = B64
            .decode(sealed.trim())
            .map_err(|_| "malformed view ticket".to_owned())?;
        if bytes.len() <= NONCE_BYTES {
            return Err("malformed view ticket".into());
        }
        let (nonce, sealed) = bytes.split_at(NONCE_BYTES);
        let nonce: [u8; NONCE_BYTES] = nonce.try_into().expect("split at the nonce");
        let cipher = Aes256Gcm::new_from_slice(&self.0).expect("32-byte key");
        let plain = cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: sealed,
                    aad: TICKET_AAD,
                },
            )
            .map_err(|_| "view ticket does not open".to_owned())?;
        let ticket: Ticket =
            serde_json::from_slice(&plain).map_err(|_| "malformed view ticket".to_owned())?;
        if ticket.expires <= now {
            return Err("view ticket expired".into());
        }
        Ok(ticket)
    }
}

/// What a view ticket carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    /// The agent session token the view acts under, as the host sent it.
    #[serde(rename = "b")]
    pub bearer: String,
    /// The MCP session, which with the bearer names the writer.
    #[serde(rename = "m")]
    pub mcp_session: String,
    /// The one space the ticket reaches.
    #[serde(rename = "s")]
    pub space: String,
    /// When the ticket's time is up, in Unix seconds.
    #[serde(rename = "e")]
    pub expires: u64,
}

/// The view tools, for `tools/list`.
pub fn tools() -> Vec<Value> {
    let space = json!({
        "type": "string",
        "pattern": "^did:key:[A-Za-z0-9]+$",
        "description": "Exact subject returned by tonk_list_spaces.",
    });
    vec![
        json!({
            "name": SHOW,
            "title": "Show a Tonk space",
            "description": "Show a route of a space inline in the chat, live: the user can read \
                and edit it there, and edits are written as this connection. Use a path within \
                the space, such as / or /notebook/<entity>; entity route parameters must be \
                actual entity URIs returned by tonk_query, never titles or names. Showing a \
                route does not verify that it rendered.",
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
            "name": RENEW,
            "description": "Renew a shown space's view ticket. Called by the view itself.",
            "inputSchema": {
                "type": "object",
                "properties": { "space": space },
                "required": ["space"],
                "additionalProperties": false,
            },
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
    ]
}

/// The relay page, for `resources/list`.
pub fn resources() -> Vec<Value> {
    vec![json!({
        "uri": RELAY_URI,
        "name": "tonk-space-view",
        "title": "Tonk space",
        "mimeType": RELAY_MIME,
    })]
}

/// The relay page for `origin`, the provider the host reached, for
/// `resources/read`.
pub fn read_resource(uri: &str, origin: &str) -> Option<Value> {
    if uri != RELAY_URI {
        return None;
    }
    let html = RELAY.replace("__TONK_MCP_ORIGIN__", &json!(origin).to_string());
    Some(json!({
        "contents": [{
            "uri": RELAY_URI,
            "mimeType": RELAY_MIME,
            "text": html,
            "_meta": {
                "ui": {
                    "csp": {
                        "connectDomains": [origin],
                        "resourceDomains": [origin],
                    },
                    "prefersBorder": true,
                },
                "openai/widgetCSP": {
                    "connect_domains": [origin],
                    "resource_domains": [origin],
                },
            },
        }],
    }))
}

/// Answer a call to a view tool, or `None` when `name` is not one.
pub async fn call(
    name: &str,
    arguments: &Value,
    verified: &Verified,
    bearer: &str,
    mcp_session: &str,
    _origin: &str,
    tickets: &TicketKey,
) -> Option<Value> {
    if name != SHOW && name != RENEW {
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
    let Some(access) = verified.spaces().get(&space) else {
        return Some(error("This connection does not reach that space."));
    };
    if !access.agent && !access.read {
        return Some(error(
            "This connection's access to that space has lapsed. Ask the user to grant it again.",
        ));
    }
    let now = Timestamp::now().to_unix();
    // A ticket outlives neither its lifetime nor the grants it rests on.
    let expires = access.expires.map_or(now + TICKET_LIFETIME, |until| {
        until.min(now + TICKET_LIFETIME)
    });
    let ticket = Ticket {
        bearer: bearer.trim().to_owned(),
        mcp_session: mcp_session.to_owned(),
        space: space.to_string(),
        expires,
    };
    let meta = json!({
        "ticket": {
            "token": tickets.seal(&ticket),
            "expiresAt": expires * 1000,
            "write": access.agent || access.write,
        },
    });
    if name == RENEW {
        return Some(json!({
            "content": [{ "type": "text", "text": "Renewed." }],
            "isError": false,
            "_meta": meta,
        }));
    }
    let path = arguments["path"].as_str().unwrap_or("/");
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.chars().any(|c| c == '\\' || c.is_control())
    {
        return Some(error("Provide a path within the space, such as /."));
    }
    Some(json!({
        "content": [{
            "type": "text",
            "text": "Showing the space inline. Rendering is not verified by this response.",
        }],
        "structuredContent": { "subject": space.to_string(), "path": path },
        "isError": false,
        "_meta": meta,
    }))
}

fn error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(expires: u64) -> Ticket {
        Ticket {
            bearer: "bearer".into(),
            mcp_session: "session".into(),
            space: "did:key:z6Mkspace".into(),
            expires,
        }
    }

    #[test]
    fn it_opens_a_ticket_it_sealed() {
        let key = TicketKey::derive("secret");
        let sealed = key.seal(&ticket(100));
        assert_eq!(key.open(&sealed, 99), Ok(ticket(100)));
    }

    #[test]
    fn it_refuses_an_expired_ticket() {
        let key = TicketKey::derive("secret");
        let sealed = key.seal(&ticket(100));
        assert_eq!(key.open(&sealed, 100), Err("view ticket expired".into()));
    }

    #[test]
    fn it_refuses_a_ticket_sealed_under_another_secret() {
        let sealed = TicketKey::derive("other").seal(&ticket(100));
        assert!(TicketKey::derive("secret").open(&sealed, 0).is_err());
    }

    #[test]
    fn it_refuses_a_tampered_ticket() {
        let key = TicketKey::derive("secret");
        let mut bytes = B64.decode(key.seal(&ticket(100))).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(key.open(&B64.encode(bytes), 0).is_err());
    }

    #[test]
    fn it_does_not_seal_the_bearer_in_the_clear() {
        let sealed = TicketKey::derive("secret").seal(&ticket(100));
        let bytes = B64.decode(sealed).unwrap();
        assert!(!bytes.windows(6).any(|window| window == b"bearer"));
    }

    #[test]
    fn it_fills_the_relay_with_the_origin() {
        let read = read_resource(RELAY_URI, "https://tonk.example").unwrap();
        let text = read["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("const origin = \"https://tonk.example\""));
        assert!(!text.contains("__TONK_MCP_ORIGIN__"));
        assert_eq!(
            read["contents"][0]["_meta"]["ui"]["csp"]["connectDomains"][0],
            "https://tonk.example"
        );
        assert!(read_resource("ui://tonk/other.html", "https://tonk.example").is_none());
    }

    #[test]
    fn it_points_the_show_tool_at_the_relay_and_hides_renewal_from_the_model() {
        let tools = tools();
        assert_eq!(tools[0]["_meta"]["ui"]["resourceUri"], RELAY_URI);
        assert_eq!(tools[1]["name"], RENEW);
        assert_eq!(tools[1]["_meta"]["ui"]["visibility"][0], "app");
    }
}
