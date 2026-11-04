//! Undoing one change from an MCP App: what a write's result carries for
//! the card, and the app-only tool the card calls with it.
//!
//! A `tonk_evaluate` that commits answers, beside the notation the model
//! reads, `_meta[`[`CHANGE_META`]`]`: the space and the revision the write
//! made, encoded as `change/undo` names one. Hosts hand `_meta` to the
//! app and not to the model. The card calls [`UNDO`] with the two, and
//! the session's writer of that space undoes the change between its pull
//! and its push ([`UNDO_ROUTE`]), answering once the undo has landed. The
//! outcome is also recorded in the space as a `tonk:undo`, so a view
//! repainting from sync shows it without a message from the card.
//!
//! The receipt strip of such a write ends with undo ([`offer`]), and the
//! tool answers the card that takes its place ([`result`]).

use dialog_varsig::Did;
use serde_json::{Value, json};

use super::super::card::Card;
use super::super::propose::Context;

/// The app-only tool a card undoes a change with.
pub const UNDO: &str = "tonk_undo";

/// The writer route that undoes a change: `POST`, `{ "revision": … }`,
/// answered with an [`UndoReport`](crate::UndoReport).
pub const UNDO_ROUTE: &str = "/api/undo";

/// The `_meta` key a write's result carries its change under.
pub const CHANGE_META: &str = "tonk/change";

/// The tool's `tools/list` entry: visible to the app alone.
pub fn tool() -> Value {
    json!({
        "name": UNDO,
        "description": "Undo one change a write made, named by the revision its result \
            carried. Called by the view.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "space": {
                    "type": "string",
                    "pattern": "^did:key:[A-Za-z0-9]+$",
                    "description": "Exact subject returned by tonk_list_spaces.",
                },
                "revision": {
                    "type": "string",
                    "pattern": "^[1-9A-HJ-NP-Za-km-z]+$",
                    "maxLength": 256,
                    "description": "The revision a write's result carried in _meta.",
                },
            },
            "required": ["space", "revision"],
            "additionalProperties": false,
        },
        "annotations": { "destructiveHint": true, "openWorldHint": false },
        "_meta": { "ui": { "visibility": ["app"] } },
    })
}

/// What a write's result carries for the card: the space and the revision
/// its commit made, or `None` when `answer` (the worker's evaluate answer)
/// committed nothing.
pub fn change_meta(space: &Did, answer: &Value) -> Option<Value> {
    if answer["commits"]["claims"].as_u64().unwrap_or(0) == 0
        || answer["revision_after"] == answer["revision_before"]
    {
        return None;
    }
    let after: dialog_artifacts::Revision =
        serde_json::from_value(answer["revision_after"].clone()).ok()?;
    Some(json!({
        "space": space.to_string(),
        "revision": tonk_schema::encode_revision(&after.version()),
    }))
}

/// The body asking a writer to undo `revision`, `POST`ed to
/// [`UNDO_ROUTE`].
pub fn request(revision: &str) -> String {
    json!({ "revision": revision }).to_string()
}

/// A write's receipt `card`, ending with undo when the write made a
/// `change` (its [`change_meta`], which the card calls [`UNDO`] with).
pub fn offer(card: Card, change: Option<&Value>) -> Card {
    match change {
        Some(_) => card.undo(UNDO),
        None => card,
    }
}

/// What stands once `reverted` is undone in `space`, as the strip says
/// it: "undone · Hiking boots is no longer in Family trip". When the one
/// entity has no name, or nothing is listed, it says the space is as it
/// was.
pub fn stands(reverted: &[crate::router::undo::Reverted], space: &str) -> String {
    let all = |was: &str| reverted.iter().all(|reverted| reverted.was == was);
    match reverted {
        [] | [crate::router::undo::Reverted { name: None, .. }] => {
            format!("undone · {space} is as it was before")
        }
        [one] => {
            let name = one.name.as_deref().unwrap_or_default();
            match one.was.as_str() {
                "added" => format!("undone · {name} is no longer in {space}"),
                "removed" => format!("undone · {name} is back in {space}"),
                _ => format!("undone · {name} is back as it was"),
            }
        }
        many => {
            let count = many.len();
            if all("added") {
                format!("undone · {count} items are no longer in {space}")
            } else if all("removed") {
                format!("undone · {count} items are back in {space}")
            } else {
                format!("undone · {count} items are back as they were")
            }
        }
    }
}

/// The tool's result for a writer's answer `report`, with the card the
/// host draws in place of the receipt: a strip once the change is undone,
/// a tell when it is refused. A refusal is an outcome, not a tool error,
/// so the card reads it either way.
pub fn result(report: &crate::UndoReport, context: &Context) -> Value {
    let space = context.place.label();
    let (text, card) = match &report.reason {
        None => (
            format!(
                "The person undid the change in {space} (revision {}).",
                report.revision
            ),
            Card::strip(
                Some(context.place.clone()),
                context.role,
                stands(&report.reverted, space),
                context.agent.clone(),
            ),
        ),
        Some(reason) => (
            format!(
                "The person tried to undo a change in {space}, but it was not undone: {reason}"
            ),
            Card::tell(
                Some(context.place.clone()),
                context.role,
                "not undone",
                format!("I couldn’t undo this in {space}, so nothing changed."),
                context.agent.clone(),
            )
            .fine(reason.clone()),
        ),
    };
    let mut structured = serde_json::to_value(report).expect("a report serializes");
    structured["card"] = card.to_value();
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": structured,
        "isError": false,
    })
}
