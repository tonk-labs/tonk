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

use axum::body::Body;
use axum::http::{Method, Request, header};
use dialog_varsig::Did;
use serde_json::{Value, json};

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

/// The request asking a writer to undo `revision`.
pub fn request(revision: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(UNDO_ROUTE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "revision": revision }).to_string()))
        .expect("static request")
}

/// The tool's result for a writer's answer `report`: a refusal is an
/// outcome, not a tool error, so the card reads it either way.
pub fn result(report: &crate::UndoReport) -> Value {
    let text = match &report.reason {
        None => "Undone.".to_owned(),
        Some(reason) => format!("Not undone: {reason}"),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": report,
        "isError": false,
    })
}
