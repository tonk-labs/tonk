//! Turning a terminal activation into a transact body.
//!
//! The terminal counterpart of `tonk-display/src/events/binding.rs`, and
//! deliberately the *only* new code the write path needs
//! (`plan/tui-views.md` §5.3). Everything else is already shared: the
//! binding table is the view's compiled `bindings` artifact, the
//! dispatch and the wire shape are `tonk_template::event`, and the
//! command is domain-shaped so it carries no platform knowledge to
//! translate.
//!
//! What differs between hosts is one `match` over [`Source`] — reading a
//! value out of a live DOM event versus out of the row the terminal has
//! focused. Three of the five variants need no translation at all:
//! `Literal` and `Entity` are constants, and `Reference` is unresolved
//! on both hosts by construction.
//!
//! `Field` is *easier* here than in a browser. The browser reads a
//! rendered `data-<name>` attribute back out of the DOM, because by
//! dispatch time the conclusion is gone; the terminal still has the
//! conclusion, so it reads the value directly and never round-trips
//! through a string.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use serde_json::Value;
use tonk_render::Conclusion;
use tonk_template::event::{EventDescriptor, Source};

/// What the host knows about the activated element.
///
/// `row` is the conclusion behind the focused repeat clone — the
/// terminal's equivalent of "the element's row". `attributes` are the
/// focused element's own attributes, which is what a
/// `.currentTarget.dataset.*` source reads.
#[derive(Debug, Clone)]
pub struct Activation<'a> {
    /// The conclusion the focused element was rendered from.
    pub row: &'a Conclusion,
    /// The focused element's attributes, keyed without any `data-`
    /// prefix — `dataset.remove` looks up `remove`.
    pub attributes: &'a BTreeMap<String, String>,
}

/// One source's outcome, mirroring the browser's `ReadOutcome`.
///
/// The three-way split is load-bearing and not cosmetic: a *blank*
/// control is "not provided" and the command still posts without that
/// field, while an unresolvable path means the binding does not apply
/// and the whole assertion is abandoned. Collapsing them would either
/// post half a command or swallow a real miss.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// A value to post.
    Value(Value),
    /// Present but empty — omit the field, still post.
    Empty,
    /// The path did not resolve — abandon the assertion.
    Unresolved,
}

/// Build the transact body for `descriptor` activated on `activation`,
/// against `command` (the command's dialog descriptor JSON).
///
/// `None` means a source failed to resolve, which the caller treats as
/// "this binding does not apply" — the same fall-through the browser
/// delegate performs so a typo on an inner binding cannot swallow an
/// outer one.
pub fn build_body(
    descriptor: &EventDescriptor,
    command: &Value,
    activation: &Activation<'_>,
) -> Option<Value> {
    let with = command.get("with").and_then(Value::as_object)?;
    let mut parameters: BTreeMap<String, Value> = BTreeMap::new();

    for (field, source) in &descriptor.sources {
        // A source for a field the command does not declare is dropped
        // rather than posted: the analyzer reports it at lowering
        // (`E_EVENT_COMMAND_MISMATCH`), and an extra parameter would be
        // a wire error. Same rule as the browser.
        let Some(entry) = with.get(field) else {
            continue;
        };
        let as_type = entry.get("as").and_then(Value::as_str).unwrap_or("Text");
        match read(source, activation, as_type) {
            Outcome::Value(value) => {
                parameters.insert(field.clone(), value);
            }
            Outcome::Empty => {}
            Outcome::Unresolved => return None,
        }
    }

    Some(tonk_template::event::transact_body(command, parameters))
}

/// Read one source in terminal terms.
fn read(source: &Source, activation: &Activation<'_>, as_type: &str) -> Outcome {
    match source {
        Source::Field(name) if name == "this" => coerce_str(&activation.row.this, as_type),
        Source::Field(name) => match activation.row.fields.get(name) {
            Some(value) => coerce_ipld(value, as_type),
            // A field the row does not carry is a genuine miss, not a
            // blank: the browser reaches the same verdict when no
            // element under the binding carries the attribute.
            None => Outcome::Unresolved,
        },
        Source::Property(segments) => read_property(segments, activation, as_type),
        Source::Literal(text) | Source::Entity(text) => coerce_str(text, as_type),
        // Left unresolved by the host's name lookup — reported there,
        // and treated as a binding that does not apply rather than
        // posting a name where an entity belongs.
        Source::Reference(_) => Outcome::Unresolved,
    }
}

/// A dotted read off the "event".
///
/// A terminal has no DOM event object, so only the paths with a terminal
/// meaning resolve. `currentTarget` is the focused element — the same
/// rebinding the browser does, where it means the element the binding was
/// authored on rather than the delegation host.
///
/// Anything else is [`Outcome::Unresolved`] on purpose. A declaration
/// written against a browser-only path (`.currentTarget.files`) has no
/// terminal answer, and inventing one would post a command with a wrong
/// value rather than declining to fire.
fn read_property(segments: &[String], activation: &Activation<'_>, as_type: &str) -> Outcome {
    match segments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["currentTarget", "dataset", name] => match activation.attributes.get(*name) {
            Some(raw) if raw.is_empty() => Outcome::Empty,
            Some(raw) => coerce_str(raw, as_type),
            None => Outcome::Unresolved,
        },
        _ => Outcome::Unresolved,
    }
}

/// Coerce a rendered string to the field's `as:` type.
///
/// Mirrors `coerce` in `tonk-display/src/events/extract.rs`, including
/// the spellings it accepts and the entity sanity-check. The two are
/// counterparts; §12's command-parity test is what keeps them honest.
fn coerce_str(raw: &str, as_type: &str) -> Outcome {
    match as_type {
        "Text" | "String" | "text" | "string" => Outcome::Value(Value::String(raw.to_owned())),
        "Entity" | "entity" => {
            // A URI has a `:`. Failing fast here is what stops a stray
            // label being posted where an entity belongs.
            if raw.contains(':') {
                Outcome::Value(Value::String(raw.to_owned()))
            } else {
                Outcome::Unresolved
            }
        }
        "Boolean" | "boolean" => match raw {
            "true" => Outcome::Value(Value::Bool(true)),
            "false" => Outcome::Value(Value::Bool(false)),
            _ => Outcome::Unresolved,
        },
        "UnsignedInt" | "SignedInt" | "Integer" | "integer" | "unsigned-integer"
        | "signed-integer" | "UnsignedInteger" | "SignedInteger" => match raw.parse::<i64>() {
            Ok(number) => Outcome::Value(Value::Number(number.into())),
            Err(_) => Outcome::Unresolved,
        },
        "Float" | "float" | "Number" | "number" => match raw.parse::<f64>() {
            Ok(number) => match serde_json::Number::from_f64(number) {
                Some(number) => Outcome::Value(Value::Number(number)),
                None => Outcome::Unresolved,
            },
            Err(_) => Outcome::Unresolved,
        },
        "Symbol" | "symbol" | "Attribute" | "attribute" => {
            Outcome::Value(Value::String(raw.to_owned()))
        }
        _ => Outcome::Unresolved,
    }
}

/// Coerce a conclusion field, which arrives typed rather than as text.
///
/// The browser has to stringify through the DOM and parse back; here the
/// `Ipld` is the value the query produced, so a number stays a number.
/// Strings still go through [`coerce_str`] so both hosts apply the same
/// entity check to the same input.
fn coerce_ipld(value: &Ipld, as_type: &str) -> Outcome {
    match value {
        Ipld::String(text) if text.is_empty() => Outcome::Empty,
        Ipld::String(text) => coerce_str(text, as_type),
        Ipld::Null => Outcome::Empty,
        Ipld::Bool(flag) => match as_type {
            "Boolean" | "boolean" => Outcome::Value(Value::Bool(*flag)),
            _ => Outcome::Unresolved,
        },
        Ipld::Integer(number) => match i64::try_from(*number) {
            Ok(number) => coerce_str(&number.to_string(), as_type),
            Err(_) => Outcome::Unresolved,
        },
        Ipld::Float(number) => coerce_str(&number.to_string(), as_type),
        // A list is an iteration axis, not a scalar a command field can
        // carry; bytes and links have no source spelling that reaches
        // here.
        _ => Outcome::Unresolved,
    }
}
