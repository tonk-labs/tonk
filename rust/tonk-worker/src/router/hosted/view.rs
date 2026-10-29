//! Views for `/mcp` hosts (MCP Apps), as plan #1200 stage 7 lays out.
//!
//! The OAuth token stays in the host; a view never sees it. The model
//! calls `tonk_open_space` to show a route of a space. The view (the
//! relay page) makes its own non-extractable key and calls
//! `tonk_view_access(space, audience)`, an app-only tool, with that key's
//! DID; the session signs a delegation to it for that one space, read-only
//! unless the view was opened to edit. The delegation is useless without
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

use super::mcp::{SpaceAccess, Verified};

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
                titles or names. Set edit to let the user change the space from the view (only \
                for a space this connection may write); otherwise the view is read-only. \
                Showing a route does not verify that it rendered.",
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
                    "edit": {
                        "type": "boolean",
                        "default": false,
                        "description": "Let the user edit the space from the view.",
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
                    "edit": { "type": "boolean", "default": false },
                },
                "required": ["space", "audience"],
                "additionalProperties": false,
            },
            "_meta": { "ui": { "visibility": ["app"] } },
        }),
    ]
}

/// Resources served by `resources/list`.
pub fn resources() -> Vec<Value> {
    Vec::new()
}

/// The `resources/read` result for `uri`, or `None` when there is no such
/// resource.
pub fn read_resource(_uri: &str, _origin: &str) -> Option<Value> {
    None
}

/// The result of the view tool `name`, or `None` when it is not one.
pub async fn call(
    name: &str,
    arguments: &Value,
    verified: &Verified,
    _bearer: &str,
    _mcp_session: &str,
    origin: &str,
    _tickets: &TicketKey,
) -> Option<Value> {
    if name != OPEN && name != ACCESS {
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
    let edit = arguments["edit"].as_bool().unwrap_or(false);
    if edit && !access.agent && !access.write {
        return Some(error(
            "This connection may only read that space, so its view cannot edit.",
        ));
    }
    let remote = format!("{}/ucan/", origin.trim_end_matches('/'));
    if name == OPEN {
        let path = arguments["path"].as_str().unwrap_or("/");
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.chars().any(|c| c == '\\' || c.is_control())
        {
            return Some(error("Provide a path within the space, such as /."));
        }
        return Some(json!({
            "content": [{
                "type": "text",
                "text": "Showing the space inline. Rendering is not verified by this response.",
            }],
            "structuredContent": {
                "subject": space.to_string(),
                "path": path,
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
    let expires = Timestamp::now().to_unix() + ACCESS_LIFETIME;
    match delegate(verified, &space, access, &audience, edit, expires).await {
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

/// The session's delegations of `space` to `audience`, each a chain from
/// the space to the audience, encoded as base64url containers.
///
/// For a granted space: each grant in force that reaches what the view
/// may do, extended by the powerline and a last hop from the session to
/// the audience. For the agent space: the powerline and that hop. The last
/// hop names the space and, for a read-only view, narrows the command to
/// `/use/get`, so the view can sync and read and never publish.
async fn delegate(
    verified: &Verified,
    space: &Did,
    access: &SpaceAccess,
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
    if access.agent {
        let command = if edit { vec!["use".into()] } else { read() };
        bases.push((command, powerline.clone()));
    } else {
        for grant in &access.grants {
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
        let chain = base.push(leaf).map_err(|error| error.to_string())?;
        let encoded = Container::from(&chain)
            .encode(Tag::Base64Url)
            .map_err(|error| error.to_string())?;
        chains.push(String::from_utf8(encoded).map_err(|error| error.to_string())?);
    }
    Ok(chains)
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

/// Whether `command` covers some `/use/{verb}/...` invocation.
fn reaches(command: &[String], verb: &str) -> bool {
    command
        .iter()
        .zip(["use", verb])
        .all(|(segment, wanted)| segment == wanted)
}

fn error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

#[cfg(test)]
mod tests;
