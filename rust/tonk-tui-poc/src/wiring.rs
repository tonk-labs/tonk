//! Reading a view's binding table from a file, in the spelling an
//! author writes rather than the shape the artifact stores.
//!
//! A real host decodes `xyz.tonk.view/bindings` straight off the view
//! (`tonk_template::bindings::Bindings::decode`) and resolves each
//! command name through the branch. This proof of concept has neither a
//! view nor a branch, so it reads the same two tables out of JSON.
//!
//! The declarations are spelled the way `event!:` spells them —
//! `"subject": "{this}"` — and parsed with the analyzer's own
//! `parse_source`, rather than round-tripping a serialized `Source`.
//! That keeps the demo files legible *and* keeps them honest: they go
//! through the same grammar a document does, so a change to it breaks
//! them.

use std::collections::BTreeMap;

use serde_json::Value;
use tonk_template::event::{EventDescriptor, event_descriptor, parse_source};

/// The two tables a session needs: declarations, and the commands they
/// post to.
#[derive(Debug, Default)]
pub struct Tables {
    /// Declaration name (`on/click`) -> descriptor.
    pub events: BTreeMap<String, EventDescriptor>,
    /// Command name -> its dialog descriptor.
    pub commands: BTreeMap<String, Value>,
}

/// Parse `{"events": {...}, "commands": {...}}`.
pub fn tables(json: &str) -> Result<Tables, String> {
    let root: Value = serde_json::from_str(json).map_err(|error| format!("bindings: {error}"))?;
    let mut tables = Tables::default();

    if let Some(events) = root.get("events").and_then(Value::as_object) {
        for (name, declaration) in events {
            tables
                .events
                .insert(name.clone(), descriptor(name, declaration)?);
        }
    }
    if let Some(commands) = root.get("commands").and_then(Value::as_object) {
        for (name, command) in commands {
            tables.commands.insert(name.clone(), command.clone());
        }
    }
    Ok(tables)
}

fn descriptor(name: &str, declaration: &Value) -> Result<EventDescriptor, String> {
    let sources = declaration
        .get("where")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(field, raw)| {
                    let text = raw.as_str().unwrap_or_default();
                    (field.clone(), parse_source(text))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    event_descriptor(
        declaration.get("type").and_then(Value::as_str),
        flag(declaration, "prevent-default"),
        flag(declaration, "stop-propagation"),
        sources,
    )
    .map_err(|error| format!("bindings: `{name}`: {error}"))
}

fn flag(declaration: &Value, name: &str) -> bool {
    declaration
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
