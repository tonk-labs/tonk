//! Template preprocessing — rewrite `on<event>=<concept>` attributes
//! on a template fragment to `data-on<event>=<concept>`.
//!
//! Why: a raw `onclick="increment"` in rendered HTML is an inline
//! JS handler. The browser sets the element's `onclick` property
//! to a function whose body is the attribute's string value;
//! identifiers like `increment` evaluate against the global scope
//! and either silently fail or throw `ReferenceError`. Either way,
//! the click never reaches our listener.
//!
//! Rewriting to `data-onclick` keeps the binding information on
//! the DOM (still readable from JS, still serializable) while
//! preventing the browser from trying to evaluate it. A single
//! delegation listener on the host inspects `data-on<event>` to
//! route fires.
//!
//! The rewrite is done once per renderer construction on the
//! template fragment; iteration rows clone the rewritten subtree
//! so every row inherits the data-prefixed form.

use std::collections::BTreeSet;

use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, Node};

/// Result of preprocessing — the rewritten fragment is mutated in
/// place, and the set of event types we registered bindings for
/// is returned to the caller so it can install the matching
/// delegation listeners.
#[derive(Debug, Default)]
pub struct Bindings {
    /// Event types ("click", "submit", "keydown", …) that appear
    /// as `on<event>` attributes anywhere in the fragment. Sorted,
    /// deduplicated.
    pub event_types: BTreeSet<String>,
    /// Concept names referenced by the bindings. Used by the
    /// renderer to resolve descriptors up front so the listener
    /// can build assertions without an async hop on each click.
    pub concept_names: BTreeSet<String>,
    /// Event declarations referenced by `on:<name>=<command>`
    /// bindings, as declaration names (`on/click`).
    ///
    /// These carry no event *type* — that lives on the declaration,
    /// which the renderer resolves alongside the command descriptors.
    /// So the listener set is derived from resolved data rather than
    /// from the attribute names, which is what lets two declarations
    /// read the same platform event differently.
    pub event_names: BTreeSet<String>,
}

/// Walk `fragment`, rewriting every `on<event>=<value>` attribute
/// to `data-on<event>=<value>` and collecting the (event-type,
/// concept-name) pairs that appeared.
pub fn preprocess(fragment: &DocumentFragment) -> Bindings {
    let mut bindings = Bindings::default();
    let node: &Node = fragment.as_ref();
    visit(node, &mut bindings);
    bindings
}

fn visit(node: &Node, bindings: &mut Bindings) {
    if let Some(el) = node.dyn_ref::<Element>() {
        rewrite_element(el, bindings);
    }
    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i) {
            visit(&child, bindings);
        }
    }
}

fn rewrite_element(el: &Element, bindings: &mut Bindings) {
    // Snapshot attribute (name, value) pairs first — mutating
    // attributes while iterating a live NamedNodeMap is asking
    // for trouble.
    let attrs = el.attributes();
    let mut pending: Vec<(String, String)> = Vec::new();
    for i in 0..attrs.length() {
        let Some(attr) = attrs.item(i) else {
            continue;
        };
        let name = attr.name();
        // The `on:<name>` form needs no rewrite: `on:click` is not one
        // of HTML's event-handler content attributes, so the browser
        // never tries to evaluate it as inline JS. It is left in place
        // and read back at dispatch time.
        if let Some(event_name) = tonk_template::event::event_name_for_attribute(&name) {
            let command = attr.value().trim().to_string();
            if !command.is_empty() {
                // The command still needs its descriptor resolved, the
                // same way the legacy form's does — only the field
                // *sources* moved to the declaration.
                bindings.concept_names.insert(command);
                bindings.event_names.insert(event_name);
            }
            continue;
        }
        if let Some(event_type) = strip_on_prefix(&name) {
            // Empty event type (just `on=`) is meaningless; skip.
            if !event_type.is_empty() {
                pending.push((event_type.to_owned(), attr.value()));
            }
        }
    }
    for (event_type, value) in pending {
        let on_attr = format!("on{event_type}");
        let data_attr = format!("data-on{event_type}");
        let _ = el.remove_attribute(&on_attr);
        let _ = el.set_attribute(&data_attr, &value);
        bindings.event_types.insert(event_type);
        bindings.concept_names.insert(value);
    }
}

/// Returns `Some(rest)` when `name` looks like `on<rest>` — i.e.
/// starts with the literal `on`, lowercase. We deliberately don't
/// match `On*` / `ON*`: HTML attribute names are case-insensitive
/// but `getAttribute`/`attributes()` returns them lowercased,
/// so the wire form is always lowercase.
///
/// We also skip `on` standing alone (no event-type suffix) and
/// `on-something` (kebab-prefixed unrelated attribute). Only
/// `on<lowercase-letter>...` is treated as an event binding.
fn strip_on_prefix(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("on")?;
    // First char of the suffix must be ASCII alphabetic so we
    // don't catch `onset` (no — `onset` starts with `s`, fine —
    // think again: `onset` matches `on` prefix, leaving `set`).
    // The real risk is non-event attributes that happen to start
    // with `on`. We accept the ambiguity and treat any
    // `on<ascii-alpha>...` attribute as a candidate binding.
    let first = rest.chars().next()?;
    if first.is_ascii_alphabetic() {
        Some(rest)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::window;

    wasm_bindgen_test_configure!(run_in_browser);

    fn fragment_from_html(html: &str) -> DocumentFragment {
        let document = window().expect("window").document().expect("document");
        let template = document
            .create_element("template")
            .expect("create template")
            .dyn_into::<web_sys::HtmlTemplateElement>()
            .expect("HtmlTemplateElement");
        template.set_inner_html(html);
        template.content()
    }

    fn outer_html(fragment: &DocumentFragment) -> String {
        let document = window().expect("window").document().expect("document");
        let host = document.create_element("div").expect("create div");
        let _ = host.append_child(fragment.as_ref());
        host.inner_html()
    }

    /// Walk every element in `fragment` and report `true` if any
    /// carries an attribute named `attr`. Used by the rewrite
    /// tests to make a precise "attribute not present" assertion
    /// — string-contains on serialised HTML would match
    /// `data-onclick` as a substring of `onclick`, which is
    /// exactly the wrong direction.
    fn fragment_has_attribute(fragment: &DocumentFragment, attr: &str) -> bool {
        let elements = fragment
            .query_selector_all("*")
            .expect("query_selector_all");
        for i in 0..elements.length() {
            let Some(node) = elements.item(i) else {
                continue;
            };
            if let Some(el) = node.dyn_ref::<Element>()
                && el.has_attribute(attr)
            {
                return true;
            }
        }
        false
    }

    #[dialog_common::test]
    fn it_rewrites_onclick_to_data_onclick() {
        let fragment = fragment_from_html(r#"<button onclick="increment">+</button>"#);
        let bindings = preprocess(&fragment);
        assert!(
            fragment_has_attribute(&fragment, "data-onclick"),
            "rewritten markup should carry data-onclick; got {}",
            outer_html(&fragment)
        );
        assert!(
            !fragment_has_attribute(&fragment, "onclick"),
            "raw onclick should be gone; got {}",
            outer_html(&fragment)
        );
        assert!(bindings.event_types.contains("click"));
        assert!(bindings.concept_names.contains("increment"));
    }

    #[dialog_common::test]
    fn it_rewrites_arbitrary_event_types() {
        let fragment =
            fragment_from_html(r#"<form onsubmit="save"><input onkeydown="cancel"></form>"#);
        let bindings = preprocess(&fragment);
        assert!(fragment_has_attribute(&fragment, "data-onsubmit"));
        assert!(fragment_has_attribute(&fragment, "data-onkeydown"));
        assert!(!fragment_has_attribute(&fragment, "onsubmit"));
        assert!(!fragment_has_attribute(&fragment, "onkeydown"));
        assert!(bindings.event_types.contains("submit"));
        assert!(bindings.event_types.contains("keydown"));
        assert!(bindings.concept_names.contains("save"));
        assert!(bindings.concept_names.contains("cancel"));
    }

    #[dialog_common::test]
    fn it_recurses_into_nested_elements() {
        let fragment =
            fragment_from_html(r#"<div><span><button onclick="increment">+</button></span></div>"#);
        let bindings = preprocess(&fragment);
        let html = outer_html(&fragment);
        assert!(html.contains(r#"data-onclick="increment""#));
        assert!(bindings.event_types.contains("click"));
    }

    #[dialog_common::test]
    fn it_leaves_non_event_attributes_alone() {
        let fragment = fragment_from_html(
            r#"<button data-counter="abc" class="primary" onclick="increment">+</button>"#,
        );
        preprocess(&fragment);
        let html = outer_html(&fragment);
        assert!(html.contains(r#"data-counter="abc""#));
        assert!(html.contains(r#"class="primary""#));
        assert!(html.contains(r#"data-onclick="increment""#));
    }

    #[dialog_common::test]
    fn it_collects_distinct_event_types_across_elements() {
        let fragment = fragment_from_html(
            r#"<div><button onclick="a">A</button><button onclick="b">B</button><input onkeydown="c"></div>"#,
        );
        let bindings = preprocess(&fragment);
        assert_eq!(
            bindings.event_types.iter().cloned().collect::<Vec<_>>(),
            vec!["click", "keydown"]
        );
        assert_eq!(
            bindings.concept_names.iter().cloned().collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[dialog_common::test]
    fn it_ignores_bare_on_attribute() {
        // `on` alone isn't an event binding — no event-type suffix.
        let fragment = fragment_from_html(r#"<button on="oops">+</button>"#);
        let bindings = preprocess(&fragment);
        let html = outer_html(&fragment);
        assert!(html.contains(r#"on="oops""#), "got {html}");
        assert!(bindings.event_types.is_empty());
    }

    #[dialog_common::test]
    fn it_ignores_attributes_with_non_alpha_after_on() {
        // `on-something="x"` shouldn't be treated as an event;
        // the suffix doesn't start with an alphabetic char.
        let fragment = fragment_from_html(r#"<button on-something="x">+</button>"#);
        let bindings = preprocess(&fragment);
        assert!(bindings.event_types.is_empty());
    }
}
