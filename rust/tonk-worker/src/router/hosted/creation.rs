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
            "title": "Ask to create a Tonk space",
            "description": "Ask the person to create a space they own and grant this agent read and write access. They approve in the chat card.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": {"type":"string","minLength":1,"maxLength":100} },
                "required": ["name"], "additionalProperties": false
            },
            "annotations": {"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false},
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
            // It adds a space to the person's own account: an additive write
            // confined to Tonk, so not open-world.
            "annotations": {"readOnlyHint":false,"destructiveHint":false,"openWorldHint":false},
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

/// The bounded list stored in the creation writer's Durable Object. No
/// credentials are persisted here. The object's identity scopes the list to
/// the OAuth connection, independently of which MCP transport answers.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct PendingCreations(Vec<Pending>);

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    id: String,
    name: String,
    expires: u64,
}

/// A transition to run only after the pending list has been saved.
pub enum CreationDecision {
    /// Display the newly recorded proposal.
    Ask {
        /// One-use card identifier.
        id: String,
        /// Proposed display name.
        name: String,
    },
    /// A cancellation consumed the proposal.
    Cancel,
    /// Provision the name recorded by the proposal.
    Approve {
        /// Name from the pending record.
        name: String,
        /// Provisioning grant from the current verified connection.
        permission: String,
    },
}

impl PendingCreations {
    /// Record or consume a proposal. The caller serializes this update with
    /// other calls and commits it before returning a card or creating a space.
    pub fn decide(
        &mut self,
        request: CreateRequest,
        now: u64,
    ) -> Result<CreationDecision, &'static str> {
        self.0.retain(|p| p.expires > now);
        match request {
            CreateRequest::Propose { name } => {
                if name.trim().is_empty() || name.chars().count() > 100 {
                    return Err("Choose a name between 1 and 100 characters.");
                }
                let id = hex::encode(rand::random::<[u8; 16]>());
                if self.0.len() >= super::propose::MOST {
                    self.0.remove(0);
                }
                self.0.push(Pending {
                    id: id.clone(),
                    name: name.clone(),
                    expires: now + super::propose::LIFETIME,
                });
                Ok(CreationDecision::Ask { id, name })
            }
            CreateRequest::Answer {
                id,
                approve,
                permission,
            } => {
                let index = self
                    .0
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or("This creation request expired or was already answered.")?;
                let name = self.0.remove(index).name;
                if approve {
                    Ok(CreationDecision::Approve { name, permission })
                } else {
                    Ok(CreationDecision::Cancel)
                }
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_card_survives_reopening_and_consumption_survives_replay() {
        let mut cards = PendingCreations::default();
        let ask = cards
            .decide(
                CreateRequest::Propose {
                    name: "Research".into(),
                },
                1,
            )
            .unwrap();
        let CreationDecision::Ask { id, .. } = ask else {
            panic!("expected a card")
        };
        let answer = || CreateRequest::Answer {
            id: id.clone(),
            approve: true,
            permission: "current grant".into(),
        };
        let reopen = |cards: &PendingCreations| {
            serde_json::from_value::<PendingCreations>(serde_json::to_value(cards).unwrap())
                .unwrap()
        };
        assert!(
            reopen(&cards)
                .decide(answer(), 1 + super::super::propose::LIFETIME)
                .is_err()
        );
        let mut cards = reopen(&cards);
        assert!(
            matches!(cards.decide(answer(), 2), Ok(CreationDecision::Approve {name, ..}) if name == "Research")
        );
        assert!(reopen(&cards).decide(answer(), 3).is_err());
    }
}
