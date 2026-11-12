//! The descriptor a `concept:`, `command:` or `rule:` result stores, as
//! the notation that would write it.
//!
//! A schema query returns its descriptor as a stringified JSON `source` (or
//! a rule's `definition`) with dialog's type names and variable terms. A
//! notebook cell draws it as a `concept!:` / `command!:` / `rule!:` record,
//! so these expand it into a field map with types spelled by their built-in
//! anchors (`text`, `natural:64`) and variables as `?name`.

use dialog_artifacts::ValueDataType;
use serde_json::{Map, Value};
use tonk_notation::type_anchor;

use crate::response::QueryResult;

/// Block label of a `concept:` query — its results render as `concept!:`.
pub(crate) const CONCEPT_LABEL: &str = "concept";
/// Block label of a `command:` query — `command!:` (transient concept).
pub(crate) const COMMAND_LABEL: &str = "command";
/// Block label of a `rule:` query — `rule!:`.
pub(crate) const RULE_LABEL: &str = "rule";

/// An attribute type, as stored, the way it is typed in notation: by
/// its built-in anchor.
fn type_name_to_notation(stored: &str) -> &str {
    ValueDataType::named(stored)
        .map(|kind| type_anchor(kind.uri()))
        .unwrap_or(stored)
}

/// Rewrite every `as` value in a descriptor tree to its notation surface form.
fn notation_normalize(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "as"
                    && let Value::String(s) = child
                {
                    *s = type_name_to_notation(s).to_owned();
                } else {
                    notation_normalize(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                notation_normalize(item);
            }
        }
        _ => {}
    }
}

/// Rewrite every variable term in a rule descriptor tree to its `?name` form.
fn rule_normalize_terms(value: &mut Value) {
    if let Value::Object(map) = value {
        if map.len() == 1
            && let Some(inner) = map.get("?")
        {
            let name = inner.get("name").and_then(Value::as_str).map(str::to_owned);
            *value = match name {
                Some(name) => Value::String(format!("?{name}")),
                None => Value::String("?".to_owned()),
            };
            return;
        }
        for child in map.values_mut() {
            rule_normalize_terms(child);
        }
    } else if let Value::Array(items) = value {
        for item in items {
            rule_normalize_terms(item);
        }
    }
}

/// Extract a concept result's descriptor as an object map (parsed from the
/// stringified `source` field, with `as` discriminants normalized).
pub(crate) fn concept_descriptor(
    result: &QueryResult,
    show_transient: bool,
) -> Option<Map<String, Value>> {
    let value = result.fields.get("source")?.clone();
    let map = match value {
        Value::Object(map) => map,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut value = Value::Object(map);
    notation_normalize(&mut value);
    let Value::Object(mut map) = value else {
        unreachable!("value was constructed as an object")
    };
    if show_transient && matches!(result.fields.get("transient"), Some(Value::Bool(true))) {
        map.insert("transient".to_owned(), Value::Bool(true));
    }
    Some(map)
}

/// Expand a `rule:` result's `definition` field into the `rule!:` field layout.
pub(crate) fn rule_definition(result: &QueryResult) -> Option<Map<String, Value>> {
    let value = result.fields.get("definition")?.clone();
    let outer = match value {
        Value::Object(map) => map,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
        _ => return None,
    };
    let mut rule = match outer.get("rule") {
        Some(Value::Object(map)) => map.clone(),
        _ => return None,
    };
    // The embedded descriptor carries the head in its polarity's own
    // field now (`retract!` for retract rules), so the swap below is
    // a no-op on current data; it remains for rows written by older
    // releases, whose descriptors always used `assert!`.
    let retract = matches!(
        outer.get("polarity"),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("retract")
    );
    if retract && let Some(head) = rule.remove("assert!") {
        rule.insert("retract!".to_owned(), head);
    }
    let mut value = Value::Object(rule);
    rule_normalize_terms(&mut value);
    notation_normalize(&mut value);
    match value {
        Value::Object(map) => Some(map),
        _ => unreachable!("value was constructed as an object"),
    }
}
