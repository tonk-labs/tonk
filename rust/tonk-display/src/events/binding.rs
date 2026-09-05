//! Resolve an `on:<name>=<command>` binding against a live event.
//!
//! The counterpart of [`super::extract`] for the `event!:` form. Where
//! that walks the *command's* `dom.event.*` identifiers, this walks the
//! *event declaration's* `where:` map — so the command stays
//! domain-shaped and the platform knowledge lives in one artifact.
//!
//! Only the three source readers are here. The dispatch table, the
//! source grammar and the wire shape live in `tonk_template::event`, so
//! they are shared with any other host and tested natively.

use std::collections::BTreeMap;

use serde_json::Value;
use tonk_template::event::{Binding, Source};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, Event};

use super::extract::{ReadOutcome, coerce, read_path_and_coerce};
use super::path::EventPath;

/// The attribute the renderer stamps on a repeat row root, carrying the
/// row's subject. `{this}` reads it.
const SUBJECT_ATTRIBUTE: &str = "data-this";

/// Build the transact body for `binding` fired by `event` on `bound`.
///
/// `descriptor` is the command's dialog descriptor. Returns `None` when
/// a source failed to resolve, which the caller treats as "this binding
/// does not apply" and falls through to the next ancestor — the same
/// behaviour as the legacy path, so a typo on an inner binding cannot
/// swallow an outer one's click.
pub(super) fn build_body(
    binding: &Binding<'_>,
    descriptor: &Value,
    event: &Event,
    bound: &Element,
) -> Option<Value> {
    let with = descriptor.get("with").and_then(Value::as_object)?;
    let event_js: &JsValue = event.as_ref();
    let bound_js: &JsValue = bound.as_ref();

    let mut parameters: BTreeMap<String, Value> = BTreeMap::new();
    for (field, source) in &binding.event.sources {
        // A source for a field the command does not declare is dropped
        // rather than posted: `tonk_template::event::check` reports it
        // at lowering, and an extra parameter would be a wire error.
        let Some(entry) = with.get(field) else {
            continue;
        };
        let as_type = entry.get("as").and_then(Value::as_str).unwrap_or("Text");

        match read_source(source, event_js, bound_js, bound, as_type) {
            ReadOutcome::Value(value) => {
                parameters.insert(field.clone(), value);
            }
            // Present but empty — a blank control, or an attribute
            // rendered from an absent field. "Not provided": omit it
            // and still post, which is what `maybe:` on the command
            // makes safe to consume from a rule.
            ReadOutcome::Empty => {}
            ReadOutcome::Unresolved => return None,
        }
    }

    Some(tonk_template::event::transact_body(descriptor, parameters))
}

/// Read one source into an outcome.
fn read_source(
    source: &Source,
    event_js: &JsValue,
    bound_js: &JsValue,
    bound: &Element,
    as_type: &str,
) -> ReadOutcome {
    match source {
        // `{field}` is interpolation in the scope of the element the
        // event fired on. The renderer stamps the repeat subject as
        // `data-this` on every row root and writes interpolated values
        // into the DOM as it renders, so this reads back what the
        // render pass already wrote — never a second query.
        Source::Field(name) => read_field(name, bound, as_type),
        // `.path` walks the live event, spelled the way the platform
        // spells it. `currentTarget` means the bound element, not the
        // host the delegated listener sits on.
        Source::Property(segments) => read_path_and_coerce(
            event_js,
            bound_js,
            &EventPath {
                segments: segments.clone(),
            },
            as_type,
        ),
        Source::Literal(text) => match coerce(&JsValue::from_str(text), as_type) {
            Some(value) => ReadOutcome::Value(value),
            None => ReadOutcome::Unresolved,
        },
    }
}

/// Read a `{field}` against the bound element's row.
///
/// `{this}` is the enclosing repeat subject. Any other `{name}` reads
/// the rendered `data-<name>` attribute, so a field the template did
/// not surface is a genuine miss rather than a silent blank.
fn read_field(name: &str, bound: &Element, as_type: &str) -> ReadOutcome {
    let attribute = if name == "this" {
        SUBJECT_ATTRIBUTE.to_string()
    } else {
        format!("data-{}", name.replace(['/', '.'], "-"))
    };
    let selector = format!("[{attribute}]");
    let raw = match bound
        .closest(&selector)
        .ok()
        .flatten()
        .and_then(|holder| holder.get_attribute(&attribute))
    {
        Some(raw) => raw,
        // `data-this` is stamped on repeat row roots only: a
        // single-entity view has one conclusion and no element to
        // clone, so there is no row to carry it. Fall back to the
        // owning display's `entity`, which is the same subject by
        // definition — otherwise `{this}` would work in a directory
        // and silently fail in a detail view.
        None if name == "this" => match host_entity(bound) {
            Some(entity) => entity,
            None => return ReadOutcome::Unresolved,
        },
        None => return ReadOutcome::Unresolved,
    };
    if raw.is_empty() {
        return ReadOutcome::Empty;
    }
    match coerce(&JsValue::from_str(&raw), as_type) {
        Some(value) => ReadOutcome::Value(value),
        None => ReadOutcome::Unresolved,
    }
}

/// The subject of the nearest `<tonk-display entity=...>` ancestor.
fn host_entity(element: &Element) -> Option<String> {
    let display = element.closest("tonk-display[entity]").ok().flatten()?;
    display
        .get_attribute("entity")
        .filter(|entity| !entity.is_empty())
}

/// Every `on:`-prefixed attribute on `element`, as `(name, value)`.
pub(super) fn on_attributes(element: &Element) -> Vec<(String, String)> {
    let attrs = element.attributes();
    let mut out = Vec::new();
    for index in 0..attrs.length() {
        let Some(attr) = attrs.item(index) else {
            continue;
        };
        let name = attr.name();
        if name.starts_with(tonk_template::event::ON_PREFIX) {
            out.push((name, attr.value()));
        }
    }
    out
}

/// True when `element` carries any `on:` binding attribute.
pub(super) fn has_on_attribute(element: &Element) -> bool {
    !on_attributes(element).is_empty()
}

/// Walk up from `event.target` trying each `on:`-bound ancestor until
/// one produces a body, bounded by `host`.
pub(super) fn resolve_binding(
    event: &Event,
    event_type: &str,
    table: &tonk_template::event::EventTable,
    descriptors: &super::delegate::Descriptors,
    host: &Element,
) -> Option<Value> {
    let target = event.target()?.dyn_ref::<Element>()?.clone();
    let mut cursor = Some(target);
    while let Some(current) = cursor {
        let bound = nearest_bound(&current)?;
        // A binding outside the host belongs to another view.
        if !host.contains(Some(bound.unchecked_ref())) {
            return None;
        }
        if let Some(binding) = table.resolve(event_type, on_attributes(&bound))
            && let Some(descriptor) = descriptors.get(&binding.command)
            && let Some(body) = build_body(&binding, descriptor, event, &bound)
        {
            apply_side_effects(&binding, event);
            return Some(body);
        }
        cursor = bound.parent_element();
    }
    None
}

/// The nearest ancestor of `element` (inclusive) carrying an `on:`
/// attribute. There is no CSS selector for "any attribute with this
/// prefix", so this walks rather than using `closest`.
fn nearest_bound(element: &Element) -> Option<Element> {
    let mut cursor = Some(element.clone());
    while let Some(current) = cursor {
        if has_on_attribute(&current) {
            return Some(current);
        }
        cursor = current.parent_element();
    }
    None
}

/// Apply the declaration's side effects, and only for the binding that
/// won — a losing binding must not suppress the platform's default.
///
/// These live on the event declaration rather than in the command's
/// field map, which is what keeps the command rule-consumable: a
/// `dom.event.do/*` field stores no value, so a rule premise over a
/// command declaring one matches zero rows however successfully the
/// command transacts.
fn apply_side_effects(binding: &Binding<'_>, event: &Event) {
    if binding.event.prevent_default {
        event.prevent_default();
    }
    if binding.event.stop_propagation {
        event.stop_propagation();
    }
}
