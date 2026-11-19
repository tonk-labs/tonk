//! Chat confirmation for creating an account-owned space.
use super::join::{answer, error, refusal};
use super::mcp::{Verified, Writers};
use axum::body::Body;
use axum::http::{Method, Request, header};
use serde_json::{Value, json};

/// Propose a user-owned space.
pub const CREATE: &str = "tonk_create_space";
/// Answer the creation card.
pub const CONFIRM: &str = "tonk_create_space_confirm";
/// Internal route on the agent's session writer.
pub const CREATE_ROUTE: &str = "/api/access/create-space";

pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": CREATE,
            "title": "Create a Tonk space",
            "description": "Ask the person to create a space they own and grant this agent read and write access. They approve in the chat card.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": {"type":"string","minLength":1,"maxLength":100} },
                "required": ["name"], "additionalProperties": false
            },
            "annotations": {"readOnlyHint":true,"openWorldHint":false},
            "_meta": super::card::tool_meta()
        }),
        json!({
            "name": CONFIRM,
            "title": "Answer space creation request",
            "inputSchema": {
                "type": "object",
                "properties": {"id":{"type":"string"},"approve":{"type":"boolean"}},
                "required":["id","approve"], "additionalProperties":false
            },
            "annotations": {"readOnlyHint":false,"destructiveHint":false,"openWorldHint":true},
            "_meta": {"ui":{"visibility":["app"]},"openai/visibility":"private"}
        }),
    ]
}

/// The proposal and its one-use answer share the session's writer.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "action")]
pub enum CreateRequest {
    /// Hold the proposed display name.
    Propose {
        /// The proposed display name.
        name: String,
    },
    /// Consume an earlier proposal and, when approved, provision it.
    Answer {
        /// One-use proposal identifier.
        id: String,
        /// The choice made on the card.
        approve: bool,
        /// The account-signed provisioning grant from this connection.
        permission: String,
    },
}

pub(super) async fn call<W: Writers>(
    tool: &str,
    args: &Value,
    verified: &Verified,
    writers: &W,
    bearer: &str,
) -> Value {
    let Some(permission) = verified.token().grants().iter().find(|grant| {
        grant.ability() == "/provider/create-space" && grant.audience() == verified.agent()
    }) else {
        return error("Reconnect this agent in Tonk to request a new space.");
    };
    let body = if tool == CREATE {
        let Some(name) = args["name"].as_str() else {
            return error("Give the new space a name.");
        };
        CreateRequest::Propose {
            name: name.trim().into(),
        }
    } else {
        let (Some(id), Some(approve)) = (args["id"].as_str(), args["approve"].as_bool()) else {
            return error("Answer the creation card.");
        };
        let Ok(encoded) = permission.to_bytes() else {
            return error("The creation permission could not be read.");
        };
        CreateRequest::Answer {
            id: id.into(),
            approve,
            permission: hex::encode(encoded),
        }
    };
    let request = Request::builder()
        .method(Method::POST)
        .uri(CREATE_ROUTE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&body).expect("creation request serializes"),
        ))
        .expect("static request");
    // Chat hosts can open a new MCP transport when the person answers a card.
    // The writer key also includes the OAuth connection's random seed, so only
    // that connection can consume its one-use proposal.
    let response = writers
        .call(
            verified,
            bearer,
            "tonk/space-creation",
            verified.agent(),
            request,
        )
        .await;
    if !response.status().is_success() {
        return error(&refusal(response).await);
    }
    answer::<Value>(response)
        .await
        .unwrap_or_else(|message| error(&message))
}
