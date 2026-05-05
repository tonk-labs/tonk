//! Snapshot the author-supplied row template, extract a binding
//! plan, and apply that plan to a clone for each rendered row.
//!
//! Two pieces:
//! 1. The pure segment parser ([`parse_segments`]) that splits a
//!    string like `"Hello {name}!"` into an alternating sequence
//!    of literal text and field references.
//! 2. (Browser-only) DOM walking that builds a [`BindingPlan`]
//!    over a `DocumentFragment` and re-applies it to a clone.

/// One chunk of an interpolated string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A literal text fragment.
    Text(String),
    /// A `{field}` reference; the inner string is the field name.
    Field(String),
}

/// Parse a `{field}`-interpolated string into a sequence of
/// [`Segment`]s. Single-identifier interpolation only — `{name}`
/// works, `{name + "x"}` does not (the inner expression is treated
/// as the field name verbatim, leading to a guaranteed lookup
/// miss).
///
/// A literal `{` cannot appear in input today; document this
/// limitation upstream.
pub fn parse_segments(input: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            buf.push(ch);
            continue;
        }
        // Find the matching '}'.
        let mut name = String::new();
        let mut closed = false;
        for nch in chars.by_ref() {
            if nch == '}' {
                closed = true;
                break;
            }
            name.push(nch);
        }
        if !closed {
            // Unterminated — emit as literal.
            buf.push('{');
            buf.push_str(&name);
            continue;
        }
        if !buf.is_empty() {
            out.push(Segment::Text(std::mem::take(&mut buf)));
        }
        out.push(Segment::Field(name));
    }
    if !buf.is_empty() {
        out.push(Segment::Text(buf));
    }
    out
}

/// True if any segment is a [`Segment::Field`].
pub fn has_field(segments: &[Segment]) -> bool {
    segments.iter().any(|s| matches!(s, Segment::Field(_)))
}

/// One bound location in a cloned template — a path from the
/// fragment root to a target node, plus what to bind there.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Sequence of child indices from the fragment root down to
    /// the target node. Walking the same path on a clone reaches
    /// the analogous node.
    pub path: Vec<usize>,
    /// Whether the binding fills text content or an attribute
    /// value, plus the segment list that produces the value.
    pub kind: BindingKind,
}

/// Where a [`Binding`]'s output lands.
#[derive(Debug, Clone)]
pub enum BindingKind {
    /// Replaces the target [`web_sys::Text`] node's content with
    /// `segments` rendered against the row.
    Text {
        /// Segment list — `[Field("name")]` for a node that was
        /// originally just `{name}`.
        segments: Vec<Segment>,
    },
    /// Sets `attr_name` on the target element to the rendered
    /// segment list.
    Attribute {
        /// Attribute name (e.g. `href`).
        attr_name: String,
        /// Segment list to render per row.
        segments: Vec<Segment>,
    },
}

/// All bindings extracted from a template fragment, plus the set
/// of distinct field names referenced — the set is precomputed so
/// the renderer can short-circuit when no field actually changed.
#[derive(Debug, Clone, Default)]
pub struct BindingPlan {
    /// Every text- and attribute-binding in the fragment.
    pub bindings: Vec<Binding>,
}

#[cfg(target_arch = "wasm32")]
mod dom {
    use std::collections::BTreeMap;

    use indexmap::IndexMap;
    use web_sys::{DocumentFragment, Element, HtmlTemplateElement, Node, NodeList, window};

    use super::{Binding, BindingKind, BindingPlan, Segment, has_field, parse_segments};
    use crate::error::{ErrorDetail, ErrorKind};
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

        // No <template> — move the first element child (and its
        // siblings, all of them) into a fresh fragment.
        let fragment: DocumentFragment = document.create_document_fragment();
        let mut node = host.first_child();
        while let Some(current) = node {
            let next = current.next_sibling();
            // Skip whitespace-only text and comments.
            let keep = current.node_type() == Node::ELEMENT_NODE;
            let _ = host.remove_child(&current);
            if keep {
                let _ = fragment.append_child(&current);
            }
            node = next;
        }
        if !fragment.has_child_nodes() {
            return Err(ErrorDetail::new(
                ErrorKind::Descriptor,
                "<tonk-concept> has no row template",
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

    /// Find a descendant `<template>` element. Returns the first
    /// one in tree order.
    fn find_template(host: &Element) -> Option<Element> {
        let list: NodeList = host.query_selector_all("template").ok()?;
        list.item(0).and_then(|n| n.dyn_into::<Element>().ok())
    }

    /// Walk a fragment, replace any text node containing
    /// `{field}` with a sequence of split text nodes (so each
    /// `Field` segment lives in its own targetable node), and
    /// build the [`BindingPlan`] of paths-to-bound-nodes.
    ///
    /// Mutates `fragment` in place.
    pub fn extract_plan(fragment: &DocumentFragment) -> BindingPlan {
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
                bindings.push(Binding {
                    path: path.clone(),
                    kind: BindingKind::Attribute {
                        attr_name: name,
                        segments,
                    },
                });
            }
        }

        BindingPlan { bindings }
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
                walk(&child, path, visit);
                path.pop();
            }
        }
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

    /// Render `segments` against `row_fields` into a single string.
    /// Missing fields yield empty strings; `{this}` resolves to
    /// `row_this`.
    pub fn render_segments(
        segments: &[Segment],
        row_this: &str,
        row_fields: &BTreeMap<String, serde_json::Value>,
    ) -> String {
        let mut out = String::new();
        for seg in segments {
            match seg {
                Segment::Text(t) => out.push_str(t),
                Segment::Field(name) => {
                    if name == "this" {
                        out.push_str(row_this);
                    } else if let Some(v) = row_fields.get(name) {
                        push_json_value(&mut out, v);
                    }
                }
            }
        }
        out
    }

    fn push_json_value(out: &mut String, v: &serde_json::Value) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Number(n) => out.push_str(&n.to_string()),
            serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            serde_json::Value::Null => {}
            other => out.push_str(&other.to_string()),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{Snapshot, extract_plan, navigate, render_segments, snapshot_template};

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
}
