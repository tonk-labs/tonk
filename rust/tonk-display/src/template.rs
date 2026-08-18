//! Snapshot the author-supplied row template, extract a binding
//! plan, and apply that plan to a clone for each rendered row.
//!
//! The target-agnostic planner (segment parsing, the
//! `Binding`/`PlanNode`/`BindingPlan` types, path arithmetic, and
//! per-row string substitution) lives in [`tonk_template`] and is
//! re-exported here so existing `crate::template::*` paths keep
//! resolving. This module adds the browser-only half: walking a
//! real `DocumentFragment` to collect bindings, and applying the
//! plan to a clone via the DOM.

pub use tonk_template::*;

#[cfg(target_arch = "wasm32")]
mod dom {
    use std::collections::BTreeMap;

    use indexmap::IndexMap;
    use ipld_core::ipld::Ipld;
    use web_sys::{DocumentFragment, Element, HtmlTemplateElement, Node, window};

    use tonk_host::error::{ErrorDetail, ErrorKind};
    use tonk_template::{Binding, BindingKind, BindingPlan, Segment, has_field, parse_segments};
    use wasm_bindgen::JsCast;

    /// Move the row template's content out of the host element
    /// into a [`DocumentFragment`] and return the fragment plus
    /// the rendering container (where rows get appended).
    ///
    /// Two cases:
    /// 1. Host contains a `<template>` — its `.content` is the
    ///    fragment, the host itself is the container, and
    ///    everything else stays put as static chrome.
    /// 2. No `<template>` — the first non-whitespace child element
    ///    is moved into a fresh fragment; the host is the
    ///    container. Subsequent siblings are also moved (so the
    ///    template can have multiple roots).
    ///
    /// Returns `Err` only when the host has no usable row template
    /// at all.
    pub fn snapshot_template(host: &Element) -> Result<Snapshot, ErrorDetail> {
        let document = window()
            .and_then(|w| w.document())
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no document"))?;

        // Look for a <template> child anywhere in the host.
        if let Some(tpl) = find_template(host) {
            let template = tpl
                .dyn_ref::<HtmlTemplateElement>()
                .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "template cast failed"))?;
            let fragment = template.content();
            // Container is the parent of the <template> so the
            // chrome (e.g. <tbody>) that wraps the template stays
            // intact. The template element itself is removed —
            // it's an instruction, not visible chrome.
            let parent = template
                .parent_element()
                .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "template has no parent"))?;
            let _ = parent.remove_child(template);
            return Ok(Snapshot {
                fragment,
                container: parent,
            });
        }

        // No <template> — move the host's child nodes into a fresh
        // fragment. Keep element nodes and *content-bearing* text nodes
        // (so a bare `{name}` text template renders), dropping only
        // whitespace-only text (the indentation between elements) and
        // comments — neither is part of the row.
        let fragment: DocumentFragment = document.create_document_fragment();
        let mut node = host.first_child();
        while let Some(current) = node {
            let next = current.next_sibling();
            let keep = match current.node_type() {
                Node::ELEMENT_NODE => true,
                Node::TEXT_NODE => current
                    .text_content()
                    .is_some_and(|text| !text.trim().is_empty()),
                _ => false,
            };
            let _ = host.remove_child(&current);
            if keep {
                let _ = fragment.append_child(&current);
            }
            node = next;
        }
        if !fragment.has_child_nodes() {
            return Err(ErrorDetail::new(
                ErrorKind::Descriptor,
                "view has no row template",
            ));
        }
        Ok(Snapshot {
            fragment,
            container: host.clone(),
        })
    }

    /// Result of [`snapshot_template`] — the inert template
    /// fragment plus the element that rendered rows append into.
    pub struct Snapshot {
        /// The template body, with `{…}` placeholders intact.
        pub fragment: DocumentFragment,
        /// Where cloned rows get appended.
        pub container: Element,
    }

    /// Find the host's own row `<template>`: the first `<template>`
    /// in tree order that is **not** inside a nested template-owning
    /// component. A `<tonk-view>` display body can embed nested
    /// `<tonk-display>`s, each owning its own `<template>`; a plain
    /// `query_selector_all("template")` would return the first
    /// nested template and `snapshot_template` would then strip it,
    /// breaking that component. The walk stops at component
    /// boundaries so each element only ever claims a template it
    /// actually owns.
    fn find_template(host: &Element) -> Option<Element> {
        let mut child = host.first_element_child();
        while let Some(el) = child {
            if el.local_name() == "template" {
                return Some(el);
            }
            if !is_template_owning_component(&el)
                && let Some(found) = find_template(&el)
            {
                return Some(found);
            }
            child = el.next_element_sibling();
        }
        None
    }

    /// Custom elements that snapshot their own `<template>` child in
    /// their `connected_callback`. Their templates belong to them,
    /// so an ancestor's template search must skip their subtrees.
    ///
    /// Keep this list in sync with the template-snapshotting custom
    /// elements that actually register (`tonk-display` / `tonk-view`
    /// in the `tonk-display` crate). A template-owning element missing
    /// from this list would have its template stolen by an ancestor —
    /// the exact bug this guard prevents.
    fn is_template_owning_component(el: &Element) -> bool {
        matches!(el.local_name().as_str(), "tonk-display" | "tonk-view")
    }

    /// Walk a fragment, replace any text node containing
    /// `{field}` with a sequence of split text nodes (so each
    /// `Field` segment lives in its own targetable node), and
    /// build the [`BindingPlan`] of paths-to-bound-nodes.
    ///
    /// Mutates `fragment` in place.
    pub fn extract_plan(fragment: &DocumentFragment) -> BindingPlan {
        extract_plan_with_scalars(fragment, &std::collections::BTreeSet::new())
    }

    /// Like [`extract_plan`], but threads `scalar_fields` (the model concept's
    /// `cardinality: one` field names) into the planner so an optional scalar
    /// field never becomes an iteration root — and so its host element is not
    /// dropped when the value is absent. Callers without the descriptor pass an
    /// empty set (identical to [`extract_plan`]).
    pub fn extract_plan_with_scalars(
        fragment: &DocumentFragment,
        scalar_fields: &std::collections::BTreeSet<String>,
    ) -> BindingPlan {
        let bindings = collect_bindings(fragment);
        // Fold flat bindings into the iteration-aware plan tree, then
        // split it around the per-conclusion repeat element. Iteration
        // roots inside the body are discovered by the LCA of every
        // binding referencing the same field. The fold and split are
        // pure (operate on the collected `bindings` vec), which is why
        // they live outside the wasm-only `dom` module and get
        // unit-tested natively.
        let repeat_root = super::this_repeat_root(&bindings);
        super::split_plan_with_scalars(bindings, repeat_root, scalar_fields)
    }

    /// Walk a fragment, split interpolated text nodes into per-segment
    /// text nodes (mutating `fragment` in place), and return the flat
    /// list of bindings. Used by [`extract_plan`].
    fn collect_bindings(fragment: &DocumentFragment) -> Vec<Binding> {
        let mut bindings: Vec<Binding> = Vec::new();
        // (Path, NodeKind) snapshot — collect first, mutate after,
        // because mutating during walk invalidates the iterator.
        let mut text_targets: Vec<(Vec<usize>, String)> = Vec::new();
        let mut attr_targets: Vec<(Vec<usize>, IndexMap<String, String>)> = Vec::new();
        walk(
            fragment.unchecked_ref::<Node>(),
            &mut Vec::new(),
            &mut |path, node| match node.node_type() {
                Node::TEXT_NODE => {
                    let raw = node.text_content().unwrap_or_default();
                    if raw.contains('{') {
                        text_targets.push((path.to_vec(), raw));
                    }
                }
                Node::ELEMENT_NODE => {
                    if let Some(el) = node.dyn_ref::<Element>() {
                        let attrs = el.attributes();
                        let mut interpolated: IndexMap<String, String> = IndexMap::new();
                        for i in 0..attrs.length() {
                            if let Some(attr) = attrs.item(i) {
                                let name = attr.name();
                                let value = attr.value();
                                if value.contains('{') {
                                    interpolated.insert(name, value);
                                }
                            }
                        }
                        if !interpolated.is_empty() {
                            attr_targets.push((path.to_vec(), interpolated));
                        }
                    }
                }
                _ => {}
            },
        );

        // For each text target, split the original text node into
        // a sequence of single-segment text nodes so the rendered
        // value of one field doesn't trample its neighbours.
        let document = window().and_then(|w| w.document());
        for (path, raw) in text_targets {
            let segments = parse_segments(&raw);
            if !has_field(&segments) {
                continue;
            }
            let Some(node) = navigate(fragment.unchecked_ref::<Node>(), &path) else {
                continue;
            };
            let Some(parent) = node.parent_node() else {
                continue;
            };
            let document = match &document {
                Some(d) => d,
                None => continue,
            };
            // Build replacement nodes in document order, also
            // recording the new sub-path of each Field node so we
            // can target it on a clone.
            let mut new_nodes: Vec<(Node, Option<usize>)> = Vec::new();
            for seg in &segments {
                match seg {
                    Segment::Text(t) => {
                        let n: Node = document.create_text_node(t).into();
                        new_nodes.push((n, None));
                    }
                    Segment::Field(_) => {
                        let n: Node = document.create_text_node("").into();
                        new_nodes.push((n, Some(new_nodes.len())));
                    }
                }
            }
            // Replace the original node with the new sequence.
            let mut last_inserted: Node = node.clone();
            for (n, _) in &new_nodes {
                let _ = parent.insert_before(n, last_inserted.next_sibling().as_ref());
                last_inserted = n.clone();
            }
            let _ = parent.remove_child(&node);

            // Now compute paths of the new Field nodes and emit
            // bindings. The new nodes were inserted at the
            // original sibling index `path.last()`, so their paths
            // are `path[..-1] + [original_idx + i]`.
            let original_idx = *path.last().unwrap_or(&0);
            let prefix = &path[..path.len().saturating_sub(1)];
            for (i, seg) in segments.iter().enumerate() {
                if let Segment::Field(_) = seg {
                    let mut new_path = prefix.to_vec();
                    new_path.push(original_idx + i);
                    bindings.push(Binding {
                        path: new_path,
                        kind: BindingKind::Text {
                            segments: vec![seg.clone()],
                        },
                    });
                }
            }
        }

        for (path, attrs) in attr_targets {
            for (name, value) in attrs {
                let segments = parse_segments(&value);
                if !has_field(&segments) {
                    continue;
                }
                // `html:foo={x}` is the explicit "force attribute"
                // escape hatch — reads as the HTML-attribute
                // namespace. Strip the prefix so the renderer writes
                // the real name; remember the intent. The prefixed
                // attribute (left over from the parsed template) is
                // removed from the fragment so cloned rows don't
                // carry the literal `html:foo="{x}"` placeholder.
                let (attr_name, force_attribute) = match name.strip_prefix("html:") {
                    Some(stripped) => {
                        if let Some(node) = navigate(fragment.unchecked_ref::<Node>(), &path)
                            && let Some(el) = node.dyn_ref::<Element>()
                        {
                            let _ = el.remove_attribute(&name);
                        }
                        (stripped.to_string(), true)
                    }
                    None => (name, false),
                };
                bindings.push(Binding {
                    path: path.clone(),
                    kind: BindingKind::Attribute {
                        attr_name,
                        segments,
                        force_attribute,
                    },
                });
            }
        }

        bindings
    }

    /// Walk a node and its descendants, calling `visit(path, node)`
    /// for each. `path` is the sequence of child-indices from the
    /// caller's starting node down to the visited node.
    fn walk(node: &Node, path: &mut Vec<usize>, visit: &mut impl FnMut(&[usize], &Node)) {
        let children = node.child_nodes();
        for i in 0..children.length() {
            if let Some(child) = children.item(i) {
                path.push(i as usize);
                visit(path, &child);
                // Don't descend into `<style>` / `<script>`: their
                // text is CSS/JS, where `{ … }` are real braces, not
                // template `{field}` placeholders. Walking in would
                // mangle a stylesheet's rule blocks into bindings.
                if !is_raw_text_element(&child) {
                    walk(&child, path, visit);
                }
                path.pop();
            }
        }
    }

    /// `true` for elements whose content is verbatim (CSS/JS), not
    /// template markup: `<style>` and `<script>`.
    fn is_raw_text_element(node: &Node) -> bool {
        node.dyn_ref::<Element>().is_some_and(|el| {
            let tag = el.tag_name().to_ascii_lowercase();
            tag == "style" || tag == "script"
        })
    }

    /// Follow a child-index path from `root` and return the node
    /// at the end, or `None` if any step is missing.
    pub fn navigate(root: &Node, path: &[usize]) -> Option<Node> {
        let mut current = root.clone();
        for &idx in path {
            let children = current.child_nodes();
            current = children.item(idx as u32)?;
        }
        Some(current)
    }

    /// Apply an attribute-form binding to the element identified by
    /// `binding.path` under `scope_root`, dispatching between
    /// property and attribute per the rules documented on
    /// [`BindingKind::Attribute`].
    ///
    /// `rendered` is the already-stringified value (computed for
    /// cache-equality checks by the caller); single-field bindings
    /// also receive the underlying [`Ipld`] value so the
    /// dispatcher can choose typed property assignment for
    /// non-strings without re-parsing.
    pub fn apply_attribute_binding(
        scope_root: &Node,
        binding: &Binding,
        rendered: &str,
        single_field_value: Option<&Ipld>,
        has_absent_field: bool,
    ) {
        let BindingKind::Attribute {
            attr_name,
            force_attribute,
            segments,
            ..
        } = &binding.kind
        else {
            return;
        };
        let Some(target) = navigate(scope_root, &binding.path) else {
            return;
        };
        let Some(el) = target.dyn_ref::<Element>() else {
            return;
        };

        // A lone `{field}` binding (not `this`) whose field is absent
        // resolves to no value: omit the attribute entirely rather than
        // setting it to the empty string. `active={dom.host/data-active}`
        // with no `data-active` on the host must leave `<tonk-sheet-binder>`
        // with no `active` attribute at all, not `active=""` — the latter
        // is a real "" selection, not "unset". A present-but-empty value
        // still writes `""`; only a missing field clears the attribute.
        let absent_single_field = single_field_value.is_none()
            && matches!(segments.as_slice(), [Segment::Field(name)] if name != "this");

        // A MULTI-segment binding (e.g. `with="main@{id}"`) with a genuinely
        // absent `{field}` component (`has_absent_field`, computed by the
        // caller against the shadow + row fields) is only PARTIALLY resolved:
        // the value it stands for hasn't arrived yet. Writing it now
        // substitutes the absent field to nothing, turning `main@{id}` into
        // the misleading `main@` — a value that no longer contains a `{…}`
        // placeholder, so `<tonk-site>` (and any consumer that skips
        // unresolved templates) mistakes it for a resolved-but-MALFORMED
        // value and errors permanently instead of waiting. Leave the
        // attribute untouched (its prior `{…}` placeholder survives) until a
        // frame carrying the field lands. The lone-single-field case is
        // handled separately above (it clears the attribute instead).
        if !absent_single_field && has_absent_field {
            return;
        }

        if *force_attribute {
            if absent_single_field {
                let _ = el.remove_attribute(attr_name);
            } else {
                write_forced_attribute(el, attr_name, rendered, single_field_value);
            }
            return;
        }

        if absent_single_field {
            let _ = el.remove_attribute(attr_name);
            return;
        }

        // Typed property path: a single `{field}` binding whose
        // value is anything other than an Ipld string assigns the
        // typed value as a property via Reflect.set. Strings (and
        // multi-segment bindings, which always stringify) fall
        // through to the name-in-element dispatch below.
        if let Some(value) = single_field_value
            && !matches!(value, Ipld::String(_))
        {
            let js_value = ipld_to_js_value(value);
            let key = js_sys::JsString::from(attr_name.as_str()).into();
            let _ = js_sys::Reflect::set(el.as_ref(), &key, &js_value);
            return;
        }

        // String value (single-field) or multi-segment binding —
        // the result is the rendered string. Use a property when
        // the name exists on the element, otherwise setAttribute.
        let key = js_sys::JsString::from(attr_name.as_str()).into();
        let is_property = js_sys::Reflect::has(el.as_ref(), &key).unwrap_or(false);
        if is_property {
            let value: wasm_bindgen::JsValue = js_sys::JsString::from(rendered).into();
            let _ = js_sys::Reflect::set(el.as_ref(), &key, &value);
        } else {
            let _ = el.set_attribute(attr_name, rendered);
        }
    }

    /// Force-attribute path: `setAttribute` semantics, but with
    /// HTML's bool-attribute convention when the underlying value
    /// is a JSON bool — `true` adds the empty attribute, `false`
    /// removes it.
    fn write_forced_attribute(
        el: &Element,
        attr_name: &str,
        rendered: &str,
        single_field_value: Option<&Ipld>,
    ) {
        if let Some(Ipld::Bool(b)) = single_field_value {
            if *b {
                let _ = el.set_attribute(attr_name, "");
            } else {
                let _ = el.remove_attribute(attr_name);
            }
            return;
        }
        let _ = el.set_attribute(attr_name, rendered);
    }

    /// Convert an Ipld value to a typed `JsValue` for property
    /// assignment. Strings are handled by the caller (the property
    /// path is only used for non-strings); lists/maps pass through
    /// serde-wasm-bindgen so they reach the DOM as real JS
    /// arrays/objects rather than JSON-stringified blobs.
    fn ipld_to_js_value(value: &Ipld) -> wasm_bindgen::JsValue {
        use wasm_bindgen::JsValue;
        match value {
            Ipld::Bool(b) => JsValue::from_bool(*b),
            Ipld::Integer(n) => JsValue::from_f64(*n as f64),
            Ipld::Float(f) => JsValue::from_f64(*f),
            Ipld::String(s) => JsValue::from_str(s),
            Ipld::Null => JsValue::NULL,
            other => serde_wasm_bindgen::to_value(other).unwrap_or(JsValue::NULL),
        }
    }

    /// Resolve a binding's single `{field}` reference to its
    /// underlying Ipld value. Returns `None` when the binding has
    /// multiple segments or any literal text — in that case the
    /// rendered string is the only well-defined value, and typed
    /// dispatch isn't possible.
    pub fn single_field_value<'a>(
        binding: &Binding,
        row_this: &'a str,
        row_fields: &'a BTreeMap<String, Ipld>,
        shadow: &'a BTreeMap<String, Ipld>,
    ) -> Option<Ipld> {
        let segments = match &binding.kind {
            BindingKind::Text { segments } => segments,
            BindingKind::Attribute { segments, .. } => segments,
        };
        let [Segment::Field(name)] = segments.as_slice() else {
            return None;
        };
        if name == "this" {
            return Some(Ipld::String(row_this.to_string()));
        }
        shadow.get(name).or_else(|| row_fields.get(name)).cloned()
    }

    #[cfg(test)]
    mod find_template_tests {
        use super::{extract_plan, find_template, snapshot_template};
        #[cfg(target_arch = "wasm32")]
        use wasm_bindgen_test::wasm_bindgen_test_configure;
        use web_sys::{Element, window};
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_test_configure!(run_in_browser);

        fn host_with(inner_html: &str) -> Element {
            let document = window().expect("window").document().expect("document");
            let host: Element = document.create_element("div").expect("create div");
            host.set_inner_html(inner_html);
            host
        }

        // A display view can embed nested template-owning components,
        // each owning its own `<template>`. A host's template snapshot
        // must not adopt a nested component's template as its own —
        // doing so strips that component of its row template.
        #[dialog_common::test]
        fn it_skips_a_template_nested_in_a_component() {
            let host = host_with(
                "<tonk-display entity=\"did:key:zBook\"><ul><template><li>{title}</li></template></ul></tonk-display>",
            );
            assert!(find_template(&host).is_none());
        }

        // Tree order would pick the nested component's template first;
        // the search must skip it and reach the host's own template.
        #[dialog_common::test]
        fn it_prefers_an_own_template_over_one_inside_a_nested_component() {
            let host = host_with(
                "<tonk-display entity=\"did:key:zBook\"><template data-which=\"nested\"><li>{b}</li></template></tonk-display><template data-which=\"own\"><p>{a}</p></template>",
            );
            let found = find_template(&host).expect("a usable own template");
            assert_eq!(found.get_attribute("data-which").as_deref(), Some("own"));
        }

        // The walk recurses through plain wrappers, so an own template
        // several non-component levels deep is still found.
        #[dialog_common::test]
        fn it_finds_an_own_template_nested_in_plain_wrappers() {
            let host = host_with(
                "<section><div><ul><template><li>{a}</li></template></ul></div></section>",
            );
            assert!(find_template(&host).is_some());
        }

        // The own template lives inside a plain wrapper that comes
        // *before* a component sibling — recursion into the wrapper
        // must win over (and precede) the skipped component subtree.
        #[dialog_common::test]
        fn it_finds_an_own_template_in_a_wrapper_preceding_a_component() {
            let host = host_with(
                "<div><template data-which=\"own\"><p>{a}</p></template></div><tonk-display entity=\"did:key:zBook\"><template data-which=\"nested\"><li>{b}</li></template></tonk-display>",
            );
            let found = find_template(&host).expect("own template in the leading wrapper");
            assert_eq!(found.get_attribute("data-which").as_deref(), Some("own"));
        }

        // The boundary applies to every template-owning component: a
        // template inside a nested `tonk-display` or `tonk-view` belongs
        // to that component and must be skipped.
        #[dialog_common::test]
        fn it_skips_templates_nested_in_other_owning_components() {
            for tag in ["tonk-display", "tonk-view"] {
                let host = host_with(&format!("<{tag}><template><p>{{a}}</p></template></{tag}>"));
                assert!(
                    find_template(&host).is_none(),
                    "template nested in <{tag}> should be skipped",
                );
            }
        }

        // A `<style>` block's CSS uses `{ … }` for rule bodies, which
        // must not be parsed as template `{field}` bindings. The
        // extractor skips into `<style>`, so the stylesheet survives
        // verbatim and contributes no bindings.
        #[dialog_common::test]
        fn it_does_not_treat_style_braces_as_bindings() {
            let host = host_with(
                "<div><style>.sheet { display: grid; color: #131313; }</style>\
                 <span>{title}</span></div>",
            );
            let snapshot = snapshot_template(&host).expect("snapshot");
            let plan = extract_plan(&snapshot.fragment);
            // Only the `{title}` text binding — nothing from the CSS.
            // `{title}` is a subject field with no `{this}`, so the
            // fragment root repeats and the binding lives in the repeat
            // body; chrome is empty.
            assert!(
                plan.chrome.is_empty(),
                "expected no chrome, got {:?}",
                plan.chrome,
            );
            assert_eq!(
                plan.repeat.body.len(),
                1,
                "expected one plan node (the span's title), got {:?}",
                plan.repeat.body,
            );
            // The stylesheet text is untouched.
            let style = snapshot
                .fragment
                .query_selector("style")
                .ok()
                .flatten()
                .expect("style element preserved");
            assert!(
                style
                    .text_content()
                    .unwrap_or_default()
                    .contains("display: grid"),
                "stylesheet content should survive verbatim",
            );
        }

        // A view whose whole template is a bare `{field}` text node (no
        // wrapping element) must still snapshot and bind — the no-template
        // path keeps content-bearing text nodes, dropping only whitespace.
        #[dialog_common::test]
        fn it_snapshots_a_bare_text_node_template() {
            let host = host_with("{name}\n");
            let snapshot = snapshot_template(&host).expect("bare text node snapshots");
            assert!(
                snapshot.fragment.has_child_nodes(),
                "the `{{name}}` text node should survive the snapshot",
            );
            let plan = extract_plan(&snapshot.fragment);
            // `{name}` is a subject field with no `{this}`, so the fragment
            // root repeats and the single text binding lives in the body.
            assert_eq!(
                plan.repeat.body.len(),
                1,
                "expected one binding (the name text), got {:?}",
                plan.repeat.body,
            );
        }

        // Whitespace-only text between elements is still dropped, so a
        // template's indentation never becomes a stray empty row binding.
        #[dialog_common::test]
        fn it_drops_whitespace_only_text_between_elements() {
            let host = host_with("\n  <span>{title}</span>\n  ");
            let snapshot = snapshot_template(&host).expect("snapshot");
            // Only the `<span>` survives; the two whitespace text nodes go.
            assert_eq!(
                snapshot.fragment.child_nodes().length(),
                1,
                "whitespace-only text nodes should be dropped",
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{
    Snapshot, apply_attribute_binding, extract_plan, extract_plan_with_scalars, navigate,
    single_field_value, snapshot_template,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_plain_text_as_one_segment() {
        assert_eq!(parse_segments("hello"), vec![Segment::Text("hello".into())]);
    }

    #[test]
    fn it_parses_a_single_field_reference() {
        assert_eq!(
            parse_segments("{name}"),
            vec![Segment::Field("name".into())]
        );
    }

    #[test]
    fn it_parses_text_field_text() {
        assert_eq!(
            parse_segments("Hello {name}!"),
            vec![
                Segment::Text("Hello ".into()),
                Segment::Field("name".into()),
                Segment::Text("!".into()),
            ],
        );
    }

    #[test]
    fn it_parses_two_adjacent_fields() {
        assert_eq!(
            parse_segments("{first}{last}"),
            vec![
                Segment::Field("first".into()),
                Segment::Field("last".into()),
            ],
        );
    }

    #[test]
    fn it_parses_multiple_fields_with_separators() {
        assert_eq!(
            parse_segments("{name} is {age}"),
            vec![
                Segment::Field("name".into()),
                Segment::Text(" is ".into()),
                Segment::Field("age".into()),
            ],
        );
    }

    #[test]
    fn it_treats_unterminated_brace_as_literal() {
        assert_eq!(
            parse_segments("oops {name"),
            vec![Segment::Text("oops {name".into())],
        );
    }

    #[test]
    fn it_returns_empty_for_empty_input() {
        assert!(parse_segments("").is_empty());
    }

    #[test]
    fn it_detects_field_segments() {
        assert!(!has_field(&parse_segments("plain text")));
        assert!(has_field(&parse_segments("hello {name}")));
    }

    // --- LCA + plan-tree tests -------------------------------------------

    /// Helper: build a text binding referencing one field at the
    /// given path. The renderer treats a single-field text binding
    /// as `[Field(name)]`, mirroring what the splitting pass in
    /// `extract_plan` produces.
    fn text_binding(path: &[usize], field: &str) -> Binding {
        Binding {
            path: path.to_vec(),
            kind: BindingKind::Text {
                segments: vec![Segment::Field(field.into())],
            },
        }
    }

    /// Helper: build an attribute binding with one `{field}`
    /// placeholder in its value, at the given element path.
    fn attr_binding(path: &[usize], attr: &str, field: &str) -> Binding {
        Binding {
            path: path.to_vec(),
            kind: BindingKind::Attribute {
                attr_name: attr.into(),
                segments: vec![Segment::Field(field.into())],
                force_attribute: false,
            },
        }
    }

    #[test]
    fn it_returns_empty_lca_for_disjoint_paths() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2], vec![1, 2, 3]]);
        assert!(lca.is_empty());
    }

    #[test]
    fn it_returns_full_path_lca_when_paths_share_a_prefix() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2], vec![0, 1, 5, 7]]);
        assert_eq!(lca, vec![0, 1]);
    }

    #[test]
    fn it_returns_the_path_itself_for_a_single_path() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2]]);
        assert_eq!(lca, vec![0, 1, 2]);
    }

    #[test]
    fn it_returns_empty_lca_for_no_paths() {
        assert!(longest_common_path_prefix(&[]).is_empty());
    }

    #[test]
    fn it_lifts_a_single_text_placeholder_to_its_host_element() {
        // <p>{name}</p> — text binding at path [0, 0]. Its host
        // element is the <p> at [0]. The iteration root is the
        // <p>, not the text node, so empty / multi-valued data
        // can repeat the <p> wholesale.
        let nodes = build_plan_nodes(vec![text_binding(&[0, 0], "name")]);
        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "name");
                assert_eq!(path, &vec![0]);
                assert_eq!(body.len(), 1);
                match &body[0] {
                    PlanNode::Binding(b) => assert_eq!(b.path, vec![0]),
                    _ => panic!("expected Binding, got {:?}", body[0]),
                }
            }
            other => panic!("expected single Iteration, got {other:?}"),
        }
    }

    #[dialog_common::test]
    fn it_does_not_make_a_dom_host_attribute_an_iteration_root() {
        // `<tonk-sheet-binder active={dom.host/data-active}>` — an
        // attribute that copies a scalar off the outer host, not a
        // subject field. It must stay a flat `Binding`, never an
        // `Iteration`: otherwise an absent `data-active` (zero values)
        // would clone the element zero times and drop it entirely.
        let nodes = build_plan_nodes(vec![attr_binding(&[0], "active", "dom.host/data-active")]);
        match &nodes[..] {
            [PlanNode::Binding(b)] => {
                assert_eq!(b.path, vec![0]);
            }
            other => panic!("expected a single flat Binding, got {other:?}"),
        }
    }

    #[test]
    fn it_creates_independent_iteration_roots_for_sibling_placeholders() {
        // Mirrors the todo-list case:
        //   <p>You have {item} items.</p>  text at [0, 0], host <p> = [0]
        //   <li>{item}</li>                text at [1, 0], host <li> = [1]
        // LCA of host paths [0] and [1] is [] — no shared inner
        // ancestor — so each becomes its own iteration root and
        // both elements repeat independently.
        let nodes = build_plan_nodes(vec![
            text_binding(&[0, 0], "item"),
            text_binding(&[1, 0], "item"),
        ]);

        let iters: Vec<_> = nodes
            .iter()
            .filter_map(|n| match n {
                PlanNode::Iteration { field, path, .. } => Some((field.clone(), path.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            iters,
            vec![("item".to_owned(), vec![0]), ("item".to_owned(), vec![1]),],
            "expected two independent iteration roots, got {iters:?}",
        );
    }

    #[test]
    fn it_groups_multiple_occurrences_of_same_field_under_their_lca() {
        // <dl for={title}>          path [0]    attribute
        //   <dt>{title}</dt>        path [0, 0, 0]  text
        //   <dd>{title}</dd>        path [0, 1, 0]  text
        // </dl>
        // LCA = [0] — the <dl>. One iteration root.
        let nodes = build_plan_nodes(vec![
            attr_binding(&[0], "for", "title"),
            text_binding(&[0, 0, 0], "title"),
            text_binding(&[0, 1, 0], "title"),
        ]);

        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "title");
                assert_eq!(path, &vec![0]);
                // Three bindings inside, each with path
                // re-rooted relative to the <dl>.
                assert_eq!(body.len(), 3);
                let rerooted_paths: Vec<_> = body
                    .iter()
                    .filter_map(|n| match n {
                        PlanNode::Binding(b) => Some(b.path.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(rerooted_paths.contains(&Vec::<usize>::new()));
                assert!(rerooted_paths.contains(&vec![0, 0]));
                assert!(rerooted_paths.contains(&vec![1, 0]));
            }
            other => panic!("expected single Iteration, got {other:?}"),
        }
    }

    #[test]
    fn it_splits_same_field_across_disjoint_sibling_sections() {
        // <div class=workspace>            path [0]
        //   <div class=canvas>             path [0, 0]
        //     <div subtree>{sheet}</div>   path [0, 0, 0, 0]
        //   <div class=tabs>               path [0, 1]
        //     <div subtree>{sheet}</div>   path [0, 1, 0, 0]
        // Both occurrences reference `sheet` and their LCA is the
        // outer `.workspace` ([0]) — but nothing is bound ON [0], so
        // collapsing there would clone the whole workspace per sheet.
        // Expect TWO roots, one per sibling section, NOT one at [0].
        let nodes = build_plan_nodes(vec![
            text_binding(&[0, 0, 0, 0], "sheet"),
            text_binding(&[0, 1, 0, 0], "sheet"),
        ]);

        let roots: Vec<_> = nodes
            .iter()
            .filter_map(|n| match n {
                PlanNode::Iteration { field, path, .. } => Some((field.clone(), path.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            roots,
            vec![
                ("sheet".to_owned(), vec![0, 0, 0]),
                ("sheet".to_owned(), vec![0, 1, 0]),
            ],
            "two sibling sections iterating the same field must get \
             separate roots, not one at the shared ancestor",
        );
    }

    #[test]
    fn it_nests_iteration_roots_for_distinct_fields() {
        // <ul for={list}>           path [0]    attr
        //   <li for={item}>         path [0, 0] attr
        //     <span>{item}</span>   path [0, 0, 0, 0] text
        //   </li>
        // </ul>
        let nodes = build_plan_nodes(vec![
            attr_binding(&[0], "for", "list"),
            attr_binding(&[0, 0], "for", "item"),
            text_binding(&[0, 0, 0, 0], "item"),
        ]);

        // Outermost: list-iteration at [0]. Its body should
        // contain an item-iteration at relative [0] holding the
        // marker + the text binding.
        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "list");
                assert_eq!(path, &vec![0]);
                // Inside <ul>: one marker binding on the <ul>
                // itself (`for={list}`) at relative path [], plus
                // a nested item iteration at relative path [0].
                let inner_iter = body
                    .iter()
                    .find_map(|n| match n {
                        PlanNode::Iteration { field, path, body } => {
                            Some((field.clone(), path.clone(), body.clone()))
                        }
                        _ => None,
                    })
                    .expect("nested iteration present");
                assert_eq!(inner_iter.0, "item");
                assert_eq!(inner_iter.1, vec![0]);
                // Inside <li>: marker on <li> at relative []
                // plus the span text at relative [0, 0].
                let inner_paths: Vec<_> = inner_iter
                    .2
                    .iter()
                    .filter_map(|n| match n {
                        PlanNode::Binding(b) => Some(b.path.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(inner_paths.contains(&Vec::<usize>::new()));
                assert!(inner_paths.contains(&vec![0, 0]));
            }
            other => panic!("expected outer list iteration, got {other:?}"),
        }
    }

    // --- repeat-node resolution ------------------------------------------
    //
    // The repeat node is the element the renderer clones once per folded
    // conclusion. These five cases pin the rule down (paths shown beside
    // each element; `[0]` is the single top-level element of the
    // fragment).

    // 1. <div>                       [0]
    //      <span>{count}</span>      [0, 0] text host [0, 0]
    //    No {this}, one subject ref. The fragment-root <div> repeats and
    //    gets an implicit with={this}.
    #[dialog_common::test]
    fn it_repeats_the_root_when_only_a_subject_field_is_bound() {
        let root = this_repeat_root(&[text_binding(&[0, 0, 0], "count")]);
        assert_eq!(root, Some(vec![0]));
    }

    // 2a. <div subject={this}>       [0]     attr {this}
    //       <span>{count}</span>     [0, 0]  text host [0, 0]
    //     {this} is on the outermost ref-bearing element (<div>), so the
    //     <div> repeats.
    #[dialog_common::test]
    fn it_repeats_the_element_holding_this_when_this_is_outermost() {
        let root = this_repeat_root(&[
            attr_binding(&[0], "subject", "this"),
            text_binding(&[0, 0, 0], "count"),
        ]);
        assert_eq!(root, Some(vec![0]));
    }

    // 2b. <div>                                  [0]
    //       <span data-this={this} data-name={name}>  [0, 0] attrs
    //     Every reference sits on the inner <span>, so the <span>
    //     repeats — not the <div>.
    #[dialog_common::test]
    fn it_repeats_the_inner_element_when_all_refs_are_on_it() {
        let root = this_repeat_root(&[
            attr_binding(&[0, 0], "data-this", "this"),
            attr_binding(&[0, 0], "data-name", "name"),
        ]);
        assert_eq!(root, Some(vec![0, 0]));
    }

    // 3. <div>                            [0]
    //      <button data-count={count}>    [0, 0]    attr {count}
    //        <span data-of={this}>{name}</span>  [0,0,0] attr {this}, text {name}
    //    {this} is *deeper* than {count}, so it is not on the outermost
    //    ref-bearing element (<button>). The repeat node falls back to
    //    the fragment-root <div>.
    #[dialog_common::test]
    fn it_repeats_the_root_when_this_is_nested_below_another_ref() {
        let root = this_repeat_root(&[
            attr_binding(&[0, 0], "data-count", "count"),
            attr_binding(&[0, 0, 0], "data-of", "this"),
            text_binding(&[0, 0, 0, 0], "name"),
        ]);
        assert_eq!(root, Some(vec![0]));
    }

    // 4. <div data-model={dom.host/model}>    [0]     attr dom.host ref
    //      <span data-this={this} data-name={name}>  [0, 0] attrs
    //    The {dom.host/*} reference is ignored for repeat resolution, so
    //    the inner <span> (holding {this} and {name}) repeats.
    #[dialog_common::test]
    fn it_ignores_dom_host_refs_when_resolving_the_repeat_node() {
        let root = this_repeat_root(&[
            attr_binding(&[0], "data-model", "dom.host/model"),
            attr_binding(&[0, 0], "data-this", "this"),
            attr_binding(&[0, 0], "data-name", "name"),
        ]);
        assert_eq!(root, Some(vec![0, 0]));
    }

    // Sibling top-level references with no shared enclosing element: the
    // whole fragment repeats (`None`).
    #[dialog_common::test]
    fn it_repeats_the_whole_fragment_for_sibling_roots() {
        let root = this_repeat_root(&[
            text_binding(&[0, 0], "title"),
            text_binding(&[1, 0], "summary"),
        ]);
        assert_eq!(root, None);
    }

    // A {this} marker deep in a table still names its element as the
    // repeat node when it is the outermost (and only) reference holder.
    #[dialog_common::test]
    fn it_finds_a_this_marker_nested_in_chrome() {
        // <table><tbody><tr subject={this}>  [0, 0, 0]
        //   <td>{title}</td>                 [0, 0, 0, 0, 0]
        let root = this_repeat_root(&[
            attr_binding(&[0, 0, 0], "subject", "this"),
            text_binding(&[0, 0, 0, 0, 0], "title"),
        ]);
        assert_eq!(root, Some(vec![0, 0, 0]));
    }

    // --- split_plan: chrome vs per-conclusion body -----------------------

    #[dialog_common::test]
    fn it_splits_chrome_from_the_repeat_body() {
        // <tonk-sheet title={title}>      [0]     chrome (renders once)
        //   <tr subject={this}>           [0, 0]  repeat element
        //     <td>{name}</td>             [0, 0, 0, 0]
        // The title binding stays chrome; the row binding rebases under
        // the repeat element. Each side is planned independently so the
        // cardinality-one title does not wrap the repeated row.
        let plan = split_plan(
            vec![
                attr_binding(&[0], "title", "title"),
                text_binding(&[0, 0, 0, 0], "name"),
            ],
            Some(vec![0, 0]),
        );

        assert_eq!(plan.repeat.path, Some(vec![0, 0]));
        // Chrome holds the title — one iteration root at [0].
        assert_eq!(plan.chrome.len(), 1);
        assert_eq!(node_path(&plan.chrome[0]), &[0usize][..]);
        // Body holds the name, rebased relative to the <tr>: the text
        // node was [0,0,0,0], now [0,0]; its iteration root [0].
        assert_eq!(plan.repeat.body.len(), 1);
        assert_eq!(node_path(&plan.repeat.body[0]), &[0usize][..]);
    }

    #[dialog_common::test]
    fn it_puts_everything_in_the_body_when_the_fragment_repeats() {
        // No repeat element: the whole fragment is the clone. Every
        // binding is body, paths unchanged, RepeatPlan::path stays None.
        let plan = split_plan(vec![text_binding(&[0, 0], "count")], None);
        assert!(plan.chrome.is_empty());
        assert_eq!(plan.repeat.path, None);
        assert_eq!(plan.repeat.body.len(), 1);
    }
}
