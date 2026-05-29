//! Project values from a JS event into a `TransactRequest` JSON
//! body, driven by a concept descriptor.
//!
//! Walks the descriptor's `with:` map. For each attribute:
//!
//! - If the `the:` identifier sits under `dom.event` (read), follow
//!   the parsed path through the event object and coerce the
//!   resulting JS value into a dialog `Value` shape per `as:`. A
//!   path that fails to resolve (undefined/null step, type mismatch)
//!   aborts the whole transaction — a partial assertion is never
//!   posted.
//! - If the `the:` identifier sits under `dom.event.do` (action),
//!   call the named method on the event. Action attributes
//!   contribute no parameter to the assertion.
//! - Otherwise: the attribute can't be filled from a DOM event;
//!   it's silently omitted. The worker may reject the resulting
//!   claim if the field is required; that's a regular validation
//!   error.
//!
//! The output is a `serde_json::Value` shaped as a
//! `TransactRequest` body — one `claims` array with one
//! `assert` claim. Constructing JSON directly (rather than
//! materializing dialog's typed `Term<Any>` / `Parameters`)
//! avoids pulling the dialog-query type stack into the
//! main-thread renderer.
//!
//! The descriptor is assumed to represent a **transient**
//! concept. Event-derived facts are commands or intents, not
//! durable state; the wire wrapper that crosses to the worker
//! is always `{ "kind": "transient", "concept": <descriptor> }`
//! so the worker's induce loop buckets the assertion for sweep.

use js_sys::{Function, Reflect};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{Element, Event};

use super::path::{Classification, EventAction, EventPath, classify};

/// Build a `TransactRequest`-shaped JSON value for a single
/// `assert` of `concept_name` populated from `event`, using the
/// concept's `descriptor` schema.
///
/// `descriptor` is the dialog descriptor as returned by
/// `phase1_lookup`, pre-parsed once at delegate-install time. Its
/// top-level `with` field is the attribute map.
///
/// `this` is the entity URI the assertion's `this:` slot binds to.
/// It does not need to be in the descriptor (every concept has an
/// implicit `this`). For event-derived assertions it's the entity
/// being acted on — typically the view's rendered entity, surfaced
/// to the DOM via a `data-*` attribute the concept also reads.
///
/// Returns `Err` when the descriptor has no `with` field or a
/// `dom.event*` field's path didn't resolve. Action attributes
/// queue during the walk and only fire after every Read field has
/// resolved — a `preventDefault` never lands for a binding that
/// the caller is about to skip on fallthrough.
pub fn build_transact_body(
    descriptor: &Value,
    concept_name_or_uri: &str,
    this: &str,
    event: &Event,
    binding: &Element,
) -> Result<Value, ExtractError> {
    let with = descriptor
        .get("with")
        .and_then(Value::as_object)
        .ok_or(ExtractError::MissingWith)?;

    // Parameters always include `this` so the assertion has a
    // subject. The concept author shouldn't have to declare `this`
    // in the `with:` map — it's implicit on every concept.
    let mut parameters = serde_json::Map::new();
    parameters.insert("this".to_owned(), json!(this));

    let event_js: &JsValue = event.as_ref();
    let binding_js: &JsValue = binding.as_ref();

    // Actions queue up during the descriptor walk and only fire
    // after every Read field has resolved. If a field is unresolved
    // we bail with `UnresolvedField` and the caller falls through
    // to the next matching ancestor; we don't want preventDefault /
    // stopPropagation to land for a binding we're about to skip.
    let mut pending_actions: Vec<EventAction> = Vec::new();

    for (field_name, attr_value) in with {
        // Skip an explicitly-declared `this` slot — we just set it.
        if field_name == "this" {
            continue;
        }
        let Some(identifier) = attr_value.get("the").and_then(Value::as_str) else {
            // Malformed descriptor entry — skip the field.
            continue;
        };
        match classify(identifier) {
            Classification::Read(path) => {
                let as_type = attr_value
                    .get("as")
                    .and_then(Value::as_str)
                    .unwrap_or("Text");
                let value = read_path_and_coerce(event_js, binding_js, &path, as_type).ok_or_else(
                    || ExtractError::UnresolvedField {
                        field: field_name.clone(),
                        identifier: identifier.to_owned(),
                    },
                )?;
                parameters.insert(field_name.clone(), value);
            }
            Classification::Action(action) => {
                pending_actions.push(action);
            }
            Classification::Other => {
                // The field's the: doesn't point into the event,
                // so we have no way to populate it from a click.
                // Leave it absent; the worker decides whether the
                // resulting claim is valid.
            }
        }
    }

    // Body is known-good; commit the side effects on the event.
    for action in &pending_actions {
        apply_action(event_js, action);
    }

    // The wrapper around the dialog descriptor: tonk's
    // `ConceptDescriptor` enum, with `kind` + `concept`. We mark
    // event-derived assertions as transient (commands / intents,
    // not durable state). The `concept` body is dialog's plain
    // descriptor JSON — exactly what phase1 returned.
    let predicate = json!({
        "kind": "transient",
        "concept": descriptor.clone(),
    });

    let claim = json!({
        "op": "assert",
        "application": {
            "predicate": predicate,
            "parameters": parameters,
        },
    });

    // `concept_name_or_uri` is currently unused in the wire shape —
    // the predicate carries the full descriptor, which is what the
    // worker resolves against. We accept it for future tightening
    // (e.g. server-side name → entity check) and to make the call
    // site read naturally.
    let _ = concept_name_or_uri;

    Ok(json!({ "claims": [claim] }))
}

/// Read a JS event's property path and turn the resulting JS value
/// into a JSON term value matching the field's `as:` type.
///
/// A leading `currentTarget` segment resolves to `binding` (the
/// element the concept was authored on — the closest ancestor of
/// `event.target` carrying the `data-on<event>` attribute), not to
/// the real `event.currentTarget`. The real one points at the host
/// where tonk-display installed its delegation listener, which is
/// an implementation detail the template author shouldn't see.
///
/// Returns `None` when:
/// - the path doesn't resolve (any step is `undefined`/`null`/missing);
/// - the value can't be coerced to the requested `as:` type.
fn read_path_and_coerce(
    event: &JsValue,
    binding: &JsValue,
    path: &EventPath,
    as_type: &str,
) -> Option<Value> {
    let mut segments = path.segments.iter();
    let first = segments.next()?;
    let mut current = if first == "currentTarget" {
        binding.clone()
    } else {
        let next = Reflect::get(event, &JsValue::from_str(first)).ok()?;
        if next.is_undefined() || next.is_null() {
            return None;
        }
        next
    };
    for segment in segments {
        let next = Reflect::get(&current, &JsValue::from_str(segment)).ok()?;
        if next.is_undefined() || next.is_null() {
            return None;
        }
        current = next;
    }
    coerce(&current, as_type)
}

/// JS value → JSON-term value per `as:` type. Covers every variant
/// of dialog's `Value` enum (`Bytes`, `Entity`, `Boolean`, `String`,
/// `UnsignedInt`, `SignedInt`, `Float`, `Record`, `Symbol`) so a
/// concept attribute declared with any supported `as:` can be
/// populated from a DOM event provided the JS path produces a
/// shape that fits.
fn coerce(value: &JsValue, as_type: &str) -> Option<Value> {
    match as_type {
        "Text" | "String" | "text" | "string" => value.as_string().map(Value::String),
        "Entity" | "entity" => {
            // Entities are URIs encoded as strings on the wire.
            let s = value.as_string()?;
            // Cheap sanity-check: a URI has a `:`. If a binding
            // accidentally points at a DOM node, this fails fast
            // rather than serializing `[object HTMLElement]`.
            if s.contains(':') {
                Some(Value::String(s))
            } else {
                None
            }
        }
        "Boolean" | "boolean" => value.as_bool().map(Value::Bool),
        "UnsignedInt" | "SignedInt" | "Integer" | "integer" | "unsigned-integer"
        | "signed-integer" => {
            let n = value.as_f64()?;
            if n.is_finite() && n.fract() == 0.0 {
                serde_json::Number::from_f64(n).map(Value::Number)
            } else {
                None
            }
        }
        "Float" | "float" | "Number" | "number" => {
            let n = value.as_f64()?;
            serde_json::Number::from_f64(n).map(Value::Number)
        }
        // Bytes / Record both deserialize from JSON arrays of u8.
        // Accept a JS `Uint8Array` or any array-like whose entries
        // are numbers in 0..=255.
        "Bytes" | "bytes" | "Record" | "record" => js_to_bytes_array(value),
        // Symbol is dialog's `Attribute` value — domain/name. On the
        // wire it deserializes as a JSON string, but server-side it
        // collides with Entity (Entity::try_from runs first on string
        // values). We emit the string verbatim and rely on the
        // concept's `as:` field tagging the slot's intended type. If
        // the consumer expects Symbol it has to use a non-URI form;
        // otherwise the deserializer picks Entity. Documented as a
        // gotcha rather than re-encoded here.
        "Symbol" | "symbol" | "Attribute" | "attribute" => value.as_string().map(Value::String),
        _ => None,
    }
}

/// Convert a JS `Uint8Array` or array-of-numbers into a JSON array
/// of unsigned bytes — the wire shape `dialog_artifacts::Value`
/// deserializes as `Bytes` (or `Record`). Returns `None` if the
/// shape doesn't match.
fn js_to_bytes_array(value: &JsValue) -> Option<Value> {
    use js_sys::{Array, Uint8Array};
    // Prefer `Uint8Array` — it's the natural JS shape for byte data
    // (e.g. `FileReader.result`, `crypto.getRandomValues`).
    if let Some(buf) = value.dyn_ref::<Uint8Array>() {
        let bytes = buf.to_vec();
        let arr: Vec<Value> = bytes
            .into_iter()
            .map(|b| Value::Number(serde_json::Number::from(b)))
            .collect();
        return Some(Value::Array(arr));
    }
    // Fall back to a generic JS array; each entry must be a finite
    // integer in 0..=255.
    if let Some(arr) = value.dyn_ref::<Array>() {
        let len = arr.length() as usize;
        let mut out = Vec::with_capacity(len);
        for i in 0..arr.length() {
            let n = arr.get(i).as_f64()?;
            if !n.is_finite() || n.fract() != 0.0 || !(0.0..=255.0).contains(&n) {
                return None;
            }
            out.push(Value::Number(serde_json::Number::from(n as u8)));
        }
        return Some(Value::Array(out));
    }
    None
}

/// Apply a `dom.event.do` action to the event. Silently no-op when
/// the method isn't present on the runtime event object (e.g. an
/// older browser or a synthetic event lacking the method).
fn apply_action(event: &JsValue, action: &EventAction) {
    let Ok(method) = Reflect::get(event, &JsValue::from_str(&action.method)) else {
        return;
    };
    if let Some(func) = method.dyn_ref::<Function>() {
        let _ = func.call0(event);
    }
}

/// Errors building a transact body.
#[derive(Debug)]
pub enum ExtractError {
    /// Descriptor has no `with` field — not a concept descriptor.
    MissingWith,
    /// A `dom.event*` field's path resolved to `undefined`/`null`
    /// or didn't coerce to the declared `as:` type. We refuse to
    /// post a partial transaction; the click is a no-op.
    UnresolvedField {
        /// Concept field name from the descriptor `with:` map.
        field: String,
        /// The `the:` identifier whose path failed.
        identifier: String,
    },
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingWith => write!(f, "descriptor missing `with` field"),
            Self::UnresolvedField { field, identifier } => write!(
                f,
                "field `{field}` ({identifier}) did not resolve against the event",
            ),
        }
    }
}

impl std::error::Error for ExtractError {}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::Object;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Event, EventInit, window};

    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a click `Event` and attach a synthetic `target`
    /// object so `event.target.dataset.<name>` resolves under the
    /// path resolver. Returning the `Event` after stamping a
    /// custom `target` so `Reflect::get(event, "target")` finds
    /// it during the walk.
    fn synthetic_event(dataset: &[(&str, &str)]) -> Event {
        let init = EventInit::new();
        let event = Event::new_with_event_init_dict("click", &init).expect("Event::new");

        // Build a fake target: { dataset: { name: value, ... } }
        let target = Object::new();
        let ds = Object::new();
        for (k, v) in dataset {
            Reflect::set(&ds, &JsValue::from_str(k), &JsValue::from_str(v))
                .expect("set dataset entry");
        }
        Reflect::set(&target, &JsValue::from_str("dataset"), &ds).expect("set dataset");

        // The default `event.target` is readonly on real events.
        // Define an own property on the Event JS object so
        // `Reflect::get` returns our synthetic target during the
        // walk. (We can't actually shadow the real getter on a
        // browser-native Event, but defining the own property
        // works because `Reflect::get` checks own props first.)
        let event_js: &JsValue = event.as_ref();
        let descriptor = Object::new();
        Reflect::set(&descriptor, &JsValue::from_str("value"), &target).unwrap();
        Reflect::set(
            &descriptor,
            &JsValue::from_str("configurable"),
            &JsValue::TRUE,
        )
        .unwrap();
        Reflect::set(&descriptor, &JsValue::from_str("writable"), &JsValue::TRUE).unwrap();
        Reflect::set(
            &descriptor,
            &JsValue::from_str("enumerable"),
            &JsValue::TRUE,
        )
        .unwrap();
        let _ = Object::define_property(
            event_js.unchecked_ref::<Object>(),
            &JsValue::from_str("target"),
            &descriptor,
        );

        event
    }

    /// Build a `<div>` with the given `data-*` attributes — the
    /// "binding element," i.e. the closest ancestor of the click
    /// target that carries the `data-on<event>` attribute the
    /// renderer rewrote events to. `currentTarget` segments
    /// resolve to this in the projection path resolver.
    fn binding_element(data: &[(&str, &str)]) -> Element {
        let document = window().expect("window").document().expect("document");
        let el = document.create_element("div").expect("create div");
        for (k, v) in data {
            el.set_attribute(k, v).expect("set attribute");
        }
        el
    }

    fn descriptor(json_text: &str) -> Value {
        serde_json::from_str(json_text).expect("descriptor parses")
    }

    #[dialog_common::test]
    fn it_builds_a_transact_body_from_current_target_dataset() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let binding = binding_element(&[("data-counter", "did:key:zCounter")]);
        let body = build_transact_body(
            &descriptor,
            "increment",
            "did:key:zCounter",
            &event,
            &binding,
        )
        .expect("build_transact_body");
        let claims = body
            .get("claims")
            .and_then(Value::as_array)
            .expect("claims");
        assert_eq!(claims.len(), 1);
        let app = &claims[0]["application"];
        let parameters = &app["parameters"];
        assert_eq!(parameters["this"], json!("did:key:zCounter"));
        assert_eq!(parameters["counter"], json!("did:key:zCounter"));
        assert_eq!(claims[0]["op"], json!("assert"));
        assert_eq!(app["predicate"]["kind"], json!("transient"));
    }

    #[dialog_common::test]
    fn it_reads_top_level_event_fields() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "kind": { "the": "dom.event/type", "as": "Text", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let binding = binding_element(&[]);
        let body = build_transact_body(&descriptor, "noop", "did:key:zSubject", &event, &binding)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(params["kind"], json!("click"));
    }

    #[dialog_common::test]
    fn it_errors_when_a_required_field_does_not_resolve() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "missing": { "the": "dom.event/pressure", "as": "Float", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let binding = binding_element(&[]);
        let err = build_transact_body(&descriptor, "noop", "did:key:zSubject", &event, &binding)
            .expect_err("expected UnresolvedField");
        match err {
            ExtractError::UnresolvedField { field, identifier } => {
                assert_eq!(field, "missing");
                assert_eq!(identifier, "dom.event/pressure");
            }
            other => panic!("expected UnresolvedField, got {other:?}"),
        }
    }

    #[dialog_common::test]
    fn it_omits_non_dom_event_fields() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "name": { "the": "xyz.tonk.user/name", "as": "Text", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let binding = binding_element(&[]);
        let body = build_transact_body(&descriptor, "noop", "did:key:zSubject", &event, &binding)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert!(params.get("name").is_none());
    }

    #[dialog_common::test]
    fn it_errors_when_entity_coercion_fails() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "counter": { "the": "dom.event.current-target.dataset/counter", "as": "Entity", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let binding = binding_element(&[("data-counter", "not-a-uri")]);
        let err = build_transact_body(
            &descriptor,
            "increment",
            "did:key:zCounter",
            &event,
            &binding,
        )
        .expect_err("expected UnresolvedField");
        assert!(
            matches!(err, ExtractError::UnresolvedField { ref field, .. } if field == "counter"),
            "got {err:?}",
        );
    }

    #[dialog_common::test]
    fn it_reads_binding_dataset_when_event_currenttarget_lacks_it() {
        // Sanity check: the descriptor uses `current-target` but the
        // real `event.currentTarget` (the host where delegation was
        // installed) carries no data-* attrs — only the binding
        // element does. The projector must read from `binding`.
        let descriptor = descriptor(
            r#"{
                "with": {
                    "todo": { "the": "dom.event.current-target.dataset/todo", "as": "Entity", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]); // no target.dataset.todo either
        let binding = binding_element(&[("data-todo", "did:key:zTodo")]);
        let body = build_transact_body(&descriptor, "toggle", "did:key:zTodo", &event, &binding)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(params["todo"], json!("did:key:zTodo"));
    }
}
