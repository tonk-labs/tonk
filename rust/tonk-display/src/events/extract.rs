//! Project values from a JS event into a `TransactRequest` JSON
//! body, driven by a concept descriptor.
//!
//! Walks the descriptor's `with:` map. For each attribute:
//!
//! - If the `the:` identifier sits under `dom.event` (read), follow
//!   the parsed path through the event object and coerce the
//!   resulting JS value into a dialog `Value` shape per `as:`.
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
use web_sys::Event;

use super::path::{Classification, EventAction, EventPath, classify};

/// Build a `TransactRequest`-shaped JSON value for a single
/// `assert` of `concept_name` populated from `event`, using the
/// concept's `descriptor_json` schema.
///
/// `descriptor_json` is the dialog descriptor as returned by
/// `phase1_lookup` — its top-level `with` field is the attribute
/// map.
///
/// `this` is the entity URI the assertion's `this:` slot binds to.
/// It does not need to be in the descriptor (every concept has an
/// implicit `this`). For event-derived assertions it's the entity
/// being acted on — typically the view's rendered entity, surfaced
/// to the DOM via a `data-*` attribute the concept also reads.
///
/// Returns `Err` if the descriptor JSON can't be parsed. Action
/// attributes are applied to the event as a side effect during the
/// walk; an action's failure (method not present on this event
/// type) is silently ignored.
pub fn build_transact_body(
    descriptor_json: &str,
    concept_name_or_uri: &str,
    this: &str,
    event: &Event,
) -> Result<Value, ExtractError> {
    let descriptor: Value = serde_json::from_str(descriptor_json).map_err(ExtractError::Parse)?;
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
                if let Some(value) = read_path_and_coerce(event_js, &path, as_type) {
                    parameters.insert(field_name.clone(), value);
                }
            }
            Classification::Action(action) => {
                apply_action(event_js, &action);
                // Actions contribute no parameter.
            }
            Classification::Other => {
                // The field's the: doesn't point into the event,
                // so we have no way to populate it from a click.
                // Leave it absent; the worker decides whether the
                // resulting claim is valid.
            }
        }
    }

    // The wrapper around the dialog descriptor: tonk's
    // `ConceptDescriptor` enum, with `kind` + `concept`. We mark
    // event-derived assertions as transient (commands / intents,
    // not durable state). The `concept` body is dialog's plain
    // descriptor JSON — exactly what phase1 returned.
    let predicate = json!({
        "kind": "transient",
        "concept": descriptor,
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
/// Returns `None` when:
/// - the path doesn't resolve (any step is `undefined`/`null`/missing);
/// - the value can't be coerced to the requested `as:` type.
fn read_path_and_coerce(event: &JsValue, path: &EventPath, as_type: &str) -> Option<Value> {
    let mut current = event.clone();
    for segment in &path.segments {
        let next = Reflect::get(&current, &JsValue::from_str(segment)).ok()?;
        if next.is_undefined() || next.is_null() {
            return None;
        }
        current = next;
    }
    coerce(&current, as_type)
}

/// JS value → JSON-term value per `as:` type. The accepted set
/// mirrors dialog's `Type` enum surface; anything else falls
/// through as `None` (skipping the field).
fn coerce(value: &JsValue, as_type: &str) -> Option<Value> {
    match as_type {
        "Text" | "String" | "text" | "string" => {
            value.as_string().map(Value::String)
        }
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
        "Boolean" | "boolean" => {
            value.as_bool().map(Value::Bool)
        }
        "UnsignedInt" | "SignedInt" | "Integer" | "integer" => {
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
        _ => None,
    }
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
    /// Descriptor JSON didn't parse.
    Parse(serde_json::Error),
    /// Descriptor parsed but has no `with` field — not a concept
    /// descriptor.
    MissingWith,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "descriptor JSON parse: {e}"),
            Self::MissingWith => write!(f, "descriptor missing `with` field"),
        }
    }
}

impl std::error::Error for ExtractError {}

#[cfg(test)]
mod tests {
    use super::*;
    use js_sys::Object;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Event, EventInit};

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
        Reflect::set(&descriptor, &JsValue::from_str("configurable"), &JsValue::TRUE).unwrap();
        Reflect::set(&descriptor, &JsValue::from_str("writable"), &JsValue::TRUE).unwrap();
        Reflect::set(&descriptor, &JsValue::from_str("enumerable"), &JsValue::TRUE).unwrap();
        let _ = Object::define_property(
            event_js.unchecked_ref::<Object>(),
            &JsValue::from_str("target"),
            &descriptor,
        );

        event
    }

    #[wasm_bindgen_test]
    fn it_builds_a_transact_body_from_event_target_dataset() {
        let descriptor = r#"{
            "with": {
                "counter": { "the": "dom.event.target.dataset/counter", "as": "Entity", "cardinality": "one" }
            }
        }"#;
        let event = synthetic_event(&[("counter", "did:key:zCounter")]);
        let body = build_transact_body(descriptor, "increment", "did:key:zCounter", &event)
            .expect("build_transact_body");
        let claims = body.get("claims").and_then(Value::as_array).expect("claims");
        assert_eq!(claims.len(), 1);
        let app = &claims[0]["application"];
        let parameters = &app["parameters"];
        assert_eq!(parameters["this"], json!("did:key:zCounter"));
        assert_eq!(parameters["counter"], json!("did:key:zCounter"));
        assert_eq!(claims[0]["op"], json!("assert"));
        assert_eq!(app["predicate"]["kind"], json!("transient"));
    }

    #[wasm_bindgen_test]
    fn it_reads_top_level_event_fields() {
        let descriptor = r#"{
            "with": {
                "kind": { "the": "dom.event/type", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let event = synthetic_event(&[]);
        let body = build_transact_body(descriptor, "noop", "did:key:zSubject", &event)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(params["kind"], json!("click"));
    }

    #[wasm_bindgen_test]
    fn it_omits_fields_whose_path_resolves_to_undefined() {
        let descriptor = r#"{
            "with": {
                "missing": { "the": "dom.event/pressure", "as": "Float", "cardinality": "one" }
            }
        }"#;
        let event = synthetic_event(&[]);
        let body = build_transact_body(descriptor, "noop", "did:key:zSubject", &event)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert!(params.get("missing").is_none());
        assert!(params.get("this").is_some());
    }

    #[wasm_bindgen_test]
    fn it_omits_non_dom_event_fields() {
        let descriptor = r#"{
            "with": {
                "name": { "the": "xyz.tonk.user/name", "as": "Text", "cardinality": "one" }
            }
        }"#;
        let event = synthetic_event(&[]);
        let body = build_transact_body(descriptor, "noop", "did:key:zSubject", &event)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert!(params.get("name").is_none());
    }

    #[wasm_bindgen_test]
    fn it_rejects_entity_coercion_when_value_isnt_uri() {
        let descriptor = r#"{
            "with": {
                "counter": { "the": "dom.event.target.dataset/counter", "as": "Entity", "cardinality": "one" }
            }
        }"#;
        let event = synthetic_event(&[("counter", "not-a-uri")]);
        let body = build_transact_body(descriptor, "increment", "did:key:zCounter", &event)
            .expect("build_transact_body");
        let params = &body["claims"][0]["application"]["parameters"];
        assert!(
            params.get("counter").is_none(),
            "non-URI entity value should be dropped, got {params:?}"
        );
    }
}
