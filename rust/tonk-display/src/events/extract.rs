//! Project values from a JS event into a `TransactRequest` JSON
//! body, driven by a concept descriptor.
//!
//! Walks the descriptor's `with:` map. For each attribute:
//!
//! - If the `the:` identifier sits under `dom.event` (read), follow
//!   the parsed path through the event object and coerce the
//!   resulting JS value into a dialog `Value` shape per `as:`. A
//!   *present but empty* leaf (a blank `<wa-input>` reads back `null`,
//!   an empty text input reads `""`) is treated as "not provided" and
//!   the field is omitted — an optional input left blank must not abort
//!   the command. A path that genuinely fails to resolve (a missing
//!   intermediate step or a value that won't coerce to `as:`) aborts
//!   the whole transaction — a partial assertion is never posted.
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

/// A successfully-built transact body plus the diagnostics the
/// delegate surfaces. `blank_fields` names the `dom.event*` fields
/// whose leaf resolved but was empty (`""`/`null`) — omitted from
/// `body` rather than aborting it. The command still posts; a rule
/// premise naming an omitted field will silently match nothing, so
/// the delegate logs them.
#[derive(Debug)]
pub struct BuiltBody {
    /// The `TransactRequest`-shaped JSON wire body.
    pub body: Value,
    /// Field names omitted because their leaf read back blank.
    pub blank_fields: Vec<String>,
}

/// Build a `TransactRequest`-shaped JSON value for a single
/// `assert` of `concept_name` populated from `event`, using the
/// concept's `descriptor` schema.
///
/// `descriptor` is the dialog descriptor as returned by
/// `phase1_lookup`, pre-parsed once at delegate-install time. Its
/// top-level `with` field is the attribute map.
///
/// The `this:` slot is left out of the wire body unless the
/// descriptor itself declares a `this` field that resolves from
/// the event. The worker derives an absent `this:` from
/// `(predicate, parameters)` so each event-derived assertion gets
/// a distinct, content-addressed subject entity rather than
/// colliding on a fixed sentinel.
///
/// Returns `Err` when the descriptor has no `with` field or a
/// `dom.event*` field's path didn't resolve. Action attributes
/// queue during the walk and only fire after every Read field has
/// resolved — a `preventDefault` never lands for a binding that
/// the caller is about to skip on fallthrough.
pub fn build_transact_body(
    descriptor: &Value,
    concept_name_or_uri: &str,
    event: &Event,
    binding: &Element,
) -> Result<BuiltBody, ExtractError> {
    let with = descriptor
        .get("with")
        .and_then(Value::as_object)
        .ok_or(ExtractError::MissingWith)?;

    let mut parameters = serde_json::Map::new();

    let event_js: &JsValue = event.as_ref();
    let binding_js: &JsValue = binding.as_ref();

    // Actions queue up during the descriptor walk and only fire
    // after every Read field has resolved. If a field is unresolved
    // we bail with `UnresolvedField` and the caller falls through
    // to the next matching ancestor; we don't want preventDefault /
    // stopPropagation to land for a binding we're about to skip.
    let mut pending_actions: Vec<EventAction> = Vec::new();
    let mut blank_fields: Vec<String> = Vec::new();

    for (field_name, attr_value) in with {
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
                match read_path_and_coerce(event_js, binding_js, &path, as_type) {
                    ReadOutcome::Value(value) => {
                        parameters.insert(field_name.clone(), value);
                    }
                    // The field exists on the form/event but is empty (a
                    // blank `<wa-input>` reads back `null`, an empty text
                    // input reads `""`). Treat it as "not provided" and
                    // omit the parameter rather than aborting the whole
                    // command — an optional input left blank (the
                    // `space/create` form's `remote`) must still fire the
                    // command (and its `preventDefault`), not fall through
                    // to a native form submit. The worker decides whether
                    // the missing parameter is acceptable. Recorded so the
                    // delegate can log what was omitted — a rule premise
                    // naming the field will otherwise mismatch silently.
                    ReadOutcome::Empty => blank_fields.push(field_name.clone()),
                    // The path itself didn't resolve (a missing property /
                    // type mismatch — a descriptor typo, not a blank
                    // input). That is a genuine binding failure: abort so
                    // the caller falls through to the next ancestor and we
                    // don't post a half-built transaction.
                    ReadOutcome::Unresolved => {
                        return Err(ExtractError::UnresolvedField {
                            field: field_name.clone(),
                            identifier: identifier.to_owned(),
                        });
                    }
                }
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

    // Body is known-good; commit the side effects.
    for action in &pending_actions {
        apply_action(event_js, binding_js, action);
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

    Ok(BuiltBody {
        body: json!({ "claims": [claim] }),
        blank_fields,
    })
}

/// The result of reading a `dom.event` path for one field.
pub(super) enum ReadOutcome {
    /// The path resolved and the leaf coerced to a term value.
    Value(Value),
    /// The path resolved to a *present but empty* leaf — a blank
    /// `<wa-input>` (`value === null`) or an empty text input
    /// (`value === ""`). The field is "not provided"; the caller omits
    /// the parameter without aborting the command.
    Empty,
    /// The path didn't resolve (a missing intermediate property, a
    /// missing leaf, or a value that wouldn't coerce to `as:`). A
    /// genuine binding failure — the caller aborts and falls through.
    Unresolved,
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
/// The leaf is the field's *value*, so a present-but-empty leaf
/// (`null` from a blank `<wa-input>`, or `""`) is reported as
/// [`ReadOutcome::Empty`] — "field left blank", not a hard failure —
/// while a missing intermediate step or an uncoercible value is
/// [`ReadOutcome::Unresolved`].
pub(super) fn read_path_and_coerce(
    event: &JsValue,
    binding: &JsValue,
    path: &EventPath,
    as_type: &str,
) -> ReadOutcome {
    if path.segments.is_empty() {
        return ReadOutcome::Unresolved;
    }
    let last_index = path.segments.len() - 1;
    let mut current = JsValue::UNDEFINED;
    for (index, segment) in path.segments.iter().enumerate() {
        let next = if index == 0 && segment == "currentTarget" {
            binding.clone()
        } else if index == 0 {
            match Reflect::get(event, &JsValue::from_str(segment)) {
                Ok(v) => v,
                Err(_) => return ReadOutcome::Unresolved,
            }
        } else {
            match Reflect::get(&current, &JsValue::from_str(segment)) {
                Ok(v) => v,
                Err(_) => return ReadOutcome::Unresolved,
            }
        };
        // An intermediate step that is null/undefined means the path
        // itself didn't resolve — the binding doesn't apply, fall through.
        if index < last_index && (next.is_undefined() || next.is_null()) {
            return ReadOutcome::Unresolved;
        }
        // At the leaf, distinguish "property absent" from "property
        // present but empty":
        // - `undefined` → the property does not exist on the object (e.g.
        //   a `dataset.subject` attribute the element doesn't carry). The
        //   binding can't be filled → `Unresolved` so the caller falls
        //   through to the next ancestor.
        // - `null` → the property exists but holds no value (a blank
        //   `<wa-input>` reads `value === null`). The field is "left
        //   blank" → `Empty`, omitted without aborting the command.
        if index == last_index {
            if next.is_undefined() {
                return ReadOutcome::Unresolved;
            }
            if next.is_null() {
                return ReadOutcome::Empty;
            }
        }
        current = next;
    }
    // The leaf resolved to a non-null value. An empty string is still a
    // blank field (omit); anything else coerces.
    if let Some(text) = current.as_string()
        && text.is_empty()
    {
        return ReadOutcome::Empty;
    }
    match coerce(&current, as_type) {
        Some(value) => ReadOutcome::Value(value),
        None => ReadOutcome::Unresolved,
    }
}

/// JS value → JSON-term value per `as:` type. Covers every variant
/// of dialog's `Value` enum (`Bytes`, `Entity`, `Boolean`, `String`,
/// `UnsignedInt`, `SignedInt`, `Float`, `Record`, `Symbol`) so a
/// concept attribute declared with any supported `as:` can be
/// populated from a DOM event provided the JS path produces a
/// shape that fits.
pub(super) fn coerce(value: &JsValue, as_type: &str) -> Option<Value> {
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
        | "signed-integer" | "UnsignedInteger" | "SignedInteger" => {
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
/// Invoke `action.method` on the object reached by walking
/// `action.path` from the event. An empty path targets the event
/// itself (`event.preventDefault()`); a leading `currentTarget`
/// resolves to `binding` (the authored element), mirroring the read
/// path, so `dom.event.current-target.do/blur` blurs the input the
/// concept was authored on. A path that doesn't resolve is a silent
/// no-op, like a read that misses.
fn apply_action(event: &JsValue, binding: &JsValue, action: &EventAction) {
    let mut target = event.clone();
    for (index, segment) in action.path.iter().enumerate() {
        target = if index == 0 && segment == "currentTarget" {
            binding.clone()
        } else {
            let Ok(next) = Reflect::get(&target, &JsValue::from_str(segment)) else {
                return;
            };
            if next.is_undefined() || next.is_null() {
                return;
            }
            next
        };
    }
    let Ok(method) = Reflect::get(&target, &JsValue::from_str(&action.method)) else {
        return;
    };
    if let Some(func) = method.dyn_ref::<Function>() {
        // `call0` sets `this` to `target`, so the method runs bound to
        // the object it was read from (the event or the element).
        let _ = func.call0(&target);
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
        let body = build_transact_body(&descriptor, "increment", &event, &binding)
            .expect("build_transact_body")
            .body;
        let claims = body
            .get("claims")
            .and_then(Value::as_array)
            .expect("claims");
        assert_eq!(claims.len(), 1);
        let app = &claims[0]["application"];
        let parameters = &app["parameters"];
        // `this:` is intentionally omitted on the wire; the worker
        // derives it from `(predicate, parameters)`.
        assert!(parameters.get("this").is_none());
        assert_eq!(parameters["counter"], json!("did:key:zCounter"));
        assert_eq!(claims[0]["op"], json!("assert"));
        assert_eq!(app["predicate"]["kind"], json!("transient"));
    }

    /// A field declared `as: unsigned-integer` whose descriptor `as`
    /// serializes to `UnsignedInteger` (the dialog `Value` variant
    /// name) must coerce a whole-number event property. Regression for
    /// the `create-sheet` command's `time` nonce, which silently failed
    /// to resolve — the coerce match listed `UnsignedInt` /
    /// `unsigned-integer` but not `UnsignedInteger`, so the whole
    /// binding fell through and no command fired.
    #[dialog_common::test]
    fn it_coerces_an_unsigned_integer_typed_field() {
        let event = synthetic_event(&[]);
        // Put an integer `time` directly on the event object so the
        // `dom.event/time` path resolves it.
        let event_js: &JsValue = event.as_ref();
        Reflect::set(
            event_js,
            &JsValue::from_str("time"),
            &JsValue::from_f64(1_780_982_815_290.0),
        )
        .unwrap();

        let descriptor = descriptor(
            r#"{
                "with": {
                    "time": { "the": "dom.event/time", "as": "UnsignedInteger", "cardinality": "one" }
                }
            }"#,
        );
        let binding = binding_element(&[]);
        let body = build_transact_body(&descriptor, "noop", &event, &binding)
            .expect("build_transact_body")
            .body;
        let params = &body["claims"][0]["application"]["parameters"];
        // The field resolved (the binding didn't fall through) and holds
        // the integer value. Compare as f64 since the coerced number is
        // built via `Number::from_f64`.
        assert_eq!(params["time"].as_f64(), Some(1_780_982_815_290.0));
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
        let body = build_transact_body(&descriptor, "noop", &event, &binding)
            .expect("build_transact_body")
            .body;
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
        let err = build_transact_body(&descriptor, "noop", &event, &binding)
            .expect_err("expected UnresolvedField");
        match err {
            ExtractError::UnresolvedField { field, identifier } => {
                assert_eq!(field, "missing");
                assert_eq!(identifier, "dom.event/pressure");
            }
            other => panic!("expected UnresolvedField, got {other:?}"),
        }
    }

    /// Put a property whose value is exactly JS `null` on the event,
    /// mirroring a blank `<wa-input>` (`input.value === null`). The
    /// helper sets `event.<name> = null` so a `dom.event/<name>` read
    /// reaches a present-but-null leaf.
    fn set_null_field(event: &Event, name: &str) {
        let event_js: &JsValue = event.as_ref();
        Reflect::set(event_js, &JsValue::from_str(name), &JsValue::NULL).unwrap();
    }

    /// True if `preventDefault` has been called on `event`.
    fn default_prevented(event: &Event) -> bool {
        event.default_prevented()
    }

    // A blank optional field — a `<wa-input>` left empty reads back
    // `null` — must be OMITTED, not abort the whole command. This is the
    // `space/create` form's `remote`: leaving it blank has to still fire
    // the command (creating a local-only space) AND run its
    // `preventDefault`, rather than failing to build and letting the form
    // submit natively (the `?name=` reload bug).
    #[dialog_common::test]
    fn it_omits_a_blank_field_and_still_fires_prevent_default() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "name":   { "the": "dom.event/type", "as": "Text", "cardinality": "one" },
                    "remote": { "the": "dom.event/remote", "as": "Text", "cardinality": "one" },
                    "prevent": { "the": "dom.event.do/prevent-default" }
                }
            }"#,
        );
        // A *cancelable* click so `preventDefault` actually flips
        // `defaultPrevented` (a non-cancelable event ignores it).
        let init = web_sys::EventInit::new();
        init.set_cancelable(true);
        let event = web_sys::Event::new_with_event_init_dict("click", &init).expect("event");
        // `remote` is present on the event but null (blank input).
        set_null_field(&event, "remote");
        let binding = binding_element(&[]);

        let built = build_transact_body(&descriptor, "space/create", &event, &binding)
            .expect("a blank optional field must not abort the command");
        // The omitted field is reported so the delegate can log it.
        assert_eq!(built.blank_fields, vec!["remote".to_string()]);
        let body = built.body;
        let params = &body["claims"][0]["application"]["parameters"];
        // `name` resolved (the event `type` is "click"); `remote` was
        // blank, so it is absent — not a reason to fail the build.
        assert_eq!(params["name"], json!("click"));
        assert!(
            params.get("remote").is_none(),
            "a blank field is omitted from the parameters",
        );
        // The queued `preventDefault` fired even though `remote` was
        // blank — so a real form submit would be stopped.
        assert!(
            default_prevented(&event),
            "preventDefault must fire for a command with a blank optional field",
        );
    }

    // An empty-string value (a plain `<input>` left empty reads `""`) is
    // likewise treated as "not provided" and omitted.
    #[dialog_common::test]
    fn it_omits_an_empty_string_field() {
        let descriptor = descriptor(
            r#"{
                "with": {
                    "remote": { "the": "dom.event/remote", "as": "Text", "cardinality": "one" }
                }
            }"#,
        );
        let event = synthetic_event(&[]);
        let event_js: &JsValue = event.as_ref();
        Reflect::set(
            event_js,
            &JsValue::from_str("remote"),
            &JsValue::from_str(""),
        )
        .unwrap();
        let binding = binding_element(&[]);
        let built = build_transact_body(&descriptor, "noop", &event, &binding)
            .expect("an empty-string field must not abort the command");
        assert_eq!(built.blank_fields, vec!["remote".to_string()]);
        let params = &built.body["claims"][0]["application"]["parameters"];
        assert!(
            params.get("remote").is_none(),
            "an empty-string field is omitted",
        );
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
        let body = build_transact_body(&descriptor, "noop", &event, &binding)
            .expect("build_transact_body")
            .body;
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
        let err = build_transact_body(&descriptor, "increment", &event, &binding)
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
        let body = build_transact_body(&descriptor, "toggle", &event, &binding)
            .expect("build_transact_body")
            .body;
        let params = &body["claims"][0]["application"]["parameters"];
        assert_eq!(params["todo"], json!("did:key:zTodo"));
    }
}
