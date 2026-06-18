//! Render a [`BindingPlan`] against query conclusions into an HTML
//! string, mirroring `tonk-display`'s browser renderer without a
//! DOM.
//!
//! One-shot: there is no diffing or mounted-state bookkeeping (the
//! browser keeps that to update in place across frames). We clone
//! the parsed template subtree per conclusion, apply the plan's
//! bindings and iterations, and serialize.
//!
//! Semantics match the browser renderer:
//! - **chrome** (bindings outside the repeat) renders once against
//!   the lead conclusion;
//! - the **repeat** element is cloned once per conclusion, stamped
//!   `with=<this>`, with the repeat body applied to each clone;
//! - a cardinality-many field inside a row drives an **iteration**:
//!   the iterated subtree is cloned per value, with that field
//!   shadowed to the current value.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_template::{
    Binding, BindingKind, BindingPlan, PlanNode, RepeatPlan, Segment, render_segments_with_shadow,
    single_field_value,
};

use crate::serialize::serialize_nodes;
use crate::tree::Node;

/// A query result row: the matched entity plus its projected field
/// values. Mirrors `tonk_schema::conclusion::Conclusion` but stays
/// dialog-free so this crate can move to the dialog-db repo later.
#[derive(Debug, Clone)]
pub struct Conclusion {
    /// Entity URI of the matched concept.
    pub this: String,
    /// Field values keyed by query term name.
    pub fields: BTreeMap<String, Ipld>,
}

/// Render `roots` (the parsed + binding-split template) and `plan`
/// against `frame` (the folded conclusions) into an HTML string.
pub fn render(roots: &[Node], plan: &BindingPlan, frame: &[Conclusion]) -> String {
    serialize_nodes(&render_nodes(roots, plan, frame))
}

/// Render to the output [`Node`] tree (before serialization).
pub fn render_nodes(roots: &[Node], plan: &BindingPlan, frame: &[Conclusion]) -> Vec<Node> {
    let lead = frame.first().cloned().unwrap_or_else(empty_conclusion);

    // Start from a clone of the template and apply chrome bindings
    // once against the lead conclusion.
    let mut out: Vec<Node> = roots.to_vec();
    apply_nodes(&mut out, &plan.chrome, &lead, &empty_shadow());

    // Then expand the repeat: replace the repeat element with one
    // clone per conclusion (or, for a whole-fragment repeat, render
    // the lead once over the roots).
    expand_repeat(&mut out, &plan.repeat, roots, frame);

    out
}

/// Expand the per-conclusion repeat in place.
fn expand_repeat(out: &mut Vec<Node>, plan: &RepeatPlan, template: &[Node], frame: &[Conclusion]) {
    match &plan.path {
        Some(path) => {
            // Clone the repeat element from the pristine template once
            // per conclusion, applying the body to each, and splice
            // the run into the slot the repeat element occupies.
            let Some((parent_children, idx)) = locate_parent_slot(out, path) else {
                return;
            };
            let template_el = match navigate(template, path) {
                Some(node) => node.clone(),
                None => return,
            };
            let mut rendered_rows: Vec<Node> = Vec::new();
            let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for member in frame {
                if !seen.insert(member.this.clone()) {
                    continue;
                }
                // Body paths are relative to the repeat element itself
                // (split_plan re-roots them), so apply against the row
                // node, where an empty path means the element.
                let mut row = template_el.clone();
                stamp_with(&mut row, &member.this);
                apply_to_root(&mut row, &plan.body, member, &empty_shadow());
                rendered_rows.push(row);
            }
            parent_children.splice(idx..=idx, rendered_rows);
        }
        None => {
            // Whole-fragment repeat: no single enclosing element to
            // clone, so the lead conclusion renders once over the
            // fragment's own (already chrome-applied) nodes.
            let lead = frame.first().cloned().unwrap_or_else(empty_conclusion);
            apply_nodes(out, &plan.body, &lead, &empty_shadow());
        }
    }
}

/// Apply a list of plan nodes whose paths are relative to the
/// fragment root (a node list). Used for chrome, where binding paths
/// are absolute from the fragment. Iterations are processed
/// last-to-first so a node's mutation only shifts already-processed
/// siblings.
fn apply_nodes(
    nodes: &mut Vec<Node>,
    plan: &[PlanNode],
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    for node in plan.iter().rev() {
        match node {
            PlanNode::Binding(b) => {
                let rendered = render_binding(b, member, shadow);
                let value = single_field_value(b, &member.this, &member.fields, shadow);
                if let Some(target) = navigate_mut(nodes, &b.path) {
                    write_binding(target, b, rendered, value.as_ref());
                }
            }
            PlanNode::Iteration { field, path, body } => {
                if let Some((parent, idx)) = locate_parent_slot(nodes, path) {
                    expand_iteration(parent, idx, field, body, member, shadow);
                }
            }
        }
    }
}

/// Apply a list of plan nodes whose paths are relative to a single
/// `root` node, where the empty path targets `root` itself. Used for
/// the repeat body and iteration bodies (split_plan re-roots their
/// paths to the enclosing element).
fn apply_to_root(
    root: &mut Node,
    plan: &[PlanNode],
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    for node in plan.iter().rev() {
        match node {
            PlanNode::Binding(b) => {
                let rendered = render_binding(b, member, shadow);
                let value = single_field_value(b, &member.this, &member.fields, shadow);
                if let Some(target) = navigate_node_mut(root, &b.path) {
                    write_binding(target, b, rendered, value.as_ref());
                }
            }
            PlanNode::Iteration { field, path, body } => {
                // An iteration root relative to `root`. The empty path
                // would be `root` itself, which cannot be spliced
                // (no parent here); planner never roots an iteration at
                // the body root, so `path` is non-empty: descend to its
                // parent slot inside `root`.
                if let Some((parent, idx)) = locate_parent_slot_in_node(root, path) {
                    expand_iteration(parent, idx, field, body, member, shadow);
                }
            }
        }
    }
}

/// Write a rendered value to a target node. Text bindings replace the
/// node's text content. Attribute bindings follow the browser's
/// `apply_attribute_binding` dispatch, restricted to what is
/// HTML-observable in a serialized string:
///
/// - **absent single `{field}`** (not `this`): omit the attribute (a
///   missing field is "unset", not `name=""`).
/// - **forced attr (`html:`) + `Bool(true)`**: `name=""`; **`Bool(false)`**:
///   omit; otherwise `name=<rendered>`.
/// - **non-forced single-field non-string value** (number/bool/list/map):
///   the browser assigns a JS *property* and leaves the HTML attribute at
///   its template literal (e.g. `data-n="{count}"`), so SSR leaves it
///   untouched.
/// - **string single-field or multi-segment**: `name=<rendered>`.
///   (The browser may assign a property instead when the name matches a
///   property on a custom element — undetectable headlessly; SSR writes the
///   attribute, which is correct for standard attributes and elements.)
fn write_binding(target: &mut Node, binding: &Binding, rendered: String, value: Option<&Ipld>) {
    let (attr_name, force_attribute, segments) = match &binding.kind {
        BindingKind::Text { .. } => {
            set_text(target, rendered);
            return;
        }
        BindingKind::Attribute {
            attr_name,
            force_attribute,
            segments,
        } => (attr_name, *force_attribute, segments),
    };

    let absent_single_field =
        value.is_none() && matches!(segments.as_slice(), [Segment::Field(name)] if name != "this");

    if force_attribute {
        if absent_single_field {
            remove_attr(target, attr_name);
        } else if let Some(Ipld::Bool(b)) = value {
            if *b {
                set_attr(target, attr_name, String::new());
            } else {
                remove_attr(target, attr_name);
            }
        } else {
            set_attr(target, attr_name, rendered);
        }
        return;
    }

    if absent_single_field {
        remove_attr(target, attr_name);
        return;
    }

    // A single-field non-string value (number/bool/list/map): the
    // browser assigns a JS *property* via Reflect.set and leaves the
    // HTML attribute untouched — so the serialized markup keeps the
    // template's original literal value (e.g. `data-n="{count}"`),
    // unless the name happens to reflect (which we can't detect
    // headlessly). Leave the parsed attribute as-is to match.
    if let Some(v) = value
        && !matches!(v, Ipld::String(_))
    {
        return;
    }

    set_attr(target, attr_name, rendered);
}

/// Clone the subtree at `parent[idx]` once per value of `field` (with
/// the field shadowed), apply `body` to each clone, and splice the
/// run back into the slot. Shared by chrome- and body-rooted apply.
fn expand_iteration(
    parent: &mut Vec<Node>,
    idx: usize,
    field: &str,
    body: &[PlanNode],
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    let template_subtree = parent[idx].clone();
    let raw = shadow
        .get(field)
        .or_else(|| member.fields.get(field))
        .cloned();
    let values = collect_values(raw);

    let mut rows: Vec<Node> = Vec::with_capacity(values.len());
    for value in values {
        let mut child_shadow = shadow.clone();
        child_shadow.insert(field.to_string(), value);
        let mut row = template_subtree.clone();
        // Body paths inside an iteration are relative to the iter root.
        apply_to_root(&mut row, body, member, &child_shadow);
        rows.push(row);
    }
    parent.splice(idx..=idx, rows);
}

/// Resolve a field value into the list of per-row values:
/// `Null`/missing -> none; `List` -> one per element; scalar -> one.
fn collect_values(value: Option<Ipld>) -> Vec<Ipld> {
    match value {
        None | Some(Ipld::Null) => Vec::new(),
        Some(Ipld::List(items)) => items,
        Some(v) => vec![v],
    }
}

/// Render a binding's segments against the conclusion + shadow.
fn render_binding(
    binding: &Binding,
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> String {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    render_segments_with_shadow(segments, &member.this, &member.fields, shadow)
}

/// Stamp `with=<this>` on a repeat row's root element so the repeat
/// boundary is inspectable, matching the browser renderer.
fn stamp_with(node: &mut Node, this: &str) {
    if let Node::Element(el) = node {
        upsert_attr(&mut el.attrs, "with", this.to_string());
    }
}

/// Set a node's text content: replace an element's children with a
/// single text node, or replace a text node's value.
fn set_text(node: &mut Node, value: String) {
    match node {
        Node::Element(el) => el.children = vec![Node::Text(value)],
        Node::Text(t) => *t = value,
        Node::Comment(_) => {}
    }
}

/// Set an attribute on an element node (no-op on text/comment).
fn set_attr(node: &mut Node, name: &str, value: String) {
    if let Node::Element(el) = node {
        upsert_attr(&mut el.attrs, name, value);
    }
}

/// Remove an attribute (the binding resolved to "unset"). A template
/// attribute like `active={field}` exists in the parsed tree as
/// `active="{field}"`; when the field is absent the browser removes it,
/// so drop any pre-existing entry of that name.
fn remove_attr(node: &mut Node, name: &str) {
    if let Node::Element(el) = node {
        el.attrs.retain(|(k, _)| k != name);
    }
}

/// Insert or replace an attribute, preserving position on replace
/// and appending on insert.
fn upsert_attr(attrs: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some(slot) = attrs.iter_mut().find(|(k, _)| k == name) {
        slot.1 = value;
    } else {
        attrs.push((name.to_string(), value));
    }
}

/// Navigate a child-index path from a node list to a node (shared
/// immutable).
fn navigate<'a>(nodes: &'a [Node], path: &[usize]) -> Option<&'a Node> {
    let (&first, rest) = path.split_first()?;
    let mut current = nodes.get(first)?;
    for &idx in rest {
        current = current.children().get(idx)?;
    }
    Some(current)
}

/// Navigate to a mutable node at `path`.
fn navigate_mut<'a>(nodes: &'a mut [Node], path: &[usize]) -> Option<&'a mut Node> {
    let (&first, rest) = path.split_first()?;
    let mut current = nodes.get_mut(first)?;
    for &idx in rest {
        current = match current {
            Node::Element(el) => el.children.get_mut(idx)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Return the mutable child list containing the node at `path` and
/// the node's index within it. `path` must be non-empty.
fn locate_parent_slot<'a>(
    nodes: &'a mut Vec<Node>,
    path: &[usize],
) -> Option<(&'a mut Vec<Node>, usize)> {
    let (&last, parent_path) = path.split_last()?;
    if parent_path.is_empty() {
        return Some((nodes, last));
    }
    let (&first, rest) = parent_path.split_first()?;
    let mut current = nodes.get_mut(first)?;
    for &idx in rest {
        current = match current {
            Node::Element(el) => el.children.get_mut(idx)?,
            _ => return None,
        };
    }
    match current {
        Node::Element(el) => Some((&mut el.children, last)),
        _ => None,
    }
}

/// Navigate a path relative to a single `root` node. The empty path
/// targets `root` itself; otherwise each component descends into the
/// current element's children.
fn navigate_node_mut<'a>(root: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    let mut current = root;
    for &idx in path {
        current = match current {
            Node::Element(el) => el.children.get_mut(idx)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Like [`locate_parent_slot`] but rooted at a single node. `path`
/// must be non-empty (the empty path would be `root` itself, which
/// has no parent here).
fn locate_parent_slot_in_node<'a>(
    root: &'a mut Node,
    path: &[usize],
) -> Option<(&'a mut Vec<Node>, usize)> {
    let (&last, parent_path) = path.split_last()?;
    let parent = navigate_node_mut(root, parent_path)?;
    match parent {
        Node::Element(el) => Some((&mut el.children, last)),
        _ => None,
    }
}

fn empty_conclusion() -> Conclusion {
    Conclusion {
        this: String::new(),
        fields: BTreeMap::new(),
    }
}

fn empty_shadow() -> BTreeMap<String, Ipld> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::collect_bindings;
    use crate::parse::parse_fragment;
    use tonk_template::{build_plan_nodes, split_plan, this_repeat_root};

    /// Full pipeline: parse -> collect (mutates tree) -> plan ->
    /// render against a frame -> HTML string.
    fn render_template(html: &str, frame: &[Conclusion]) -> String {
        let mut roots = parse_fragment(html);
        let bindings = collect_bindings(&mut roots);
        let repeat_root = this_repeat_root(&bindings);
        // build_plan_nodes is part of the browser's extract_plan path;
        // split_plan consumes the raw bindings + repeat root.
        let _ = build_plan_nodes(bindings.clone());
        let plan = split_plan(bindings, repeat_root);
        render(&roots, &plan, frame)
    }

    fn row(this: &str, fields: &[(&str, &str)]) -> Conclusion {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_string(), Ipld::String((*v).to_string()));
        }
        Conclusion {
            this: this.to_string(),
            fields: map,
        }
    }

    #[test]
    fn it_renders_one_row_per_conclusion_for_the_list_template() {
        let html = "<ul><li data-id={this}>{name}</li></ul>";
        let out = render_template(
            html,
            &[row("a", &[("name", "Ann")]), row("b", &[("name", "Bo")])],
        );
        // One <li> per subject, each stamped with= and data-id, name filled.
        assert_eq!(
            out,
            "<ul>\
<li data-id=\"a\" with=\"a\">Ann</li>\
<li data-id=\"b\" with=\"b\">Bo</li>\
</ul>"
        );
    }

    #[test]
    fn it_renders_an_empty_frame_as_just_chrome() {
        // No conclusions: the repeat element is removed, leaving the
        // surrounding chrome (here, just the empty <ul>).
        let html = "<ul><li data-id={this}>{name}</li></ul>";
        let out = render_template(html, &[]);
        assert_eq!(out, "<ul></ul>");
    }

    #[test]
    fn it_escapes_text_and_attribute_values() {
        let html = "<ul><li data-id={this}>{name}</li></ul>";
        let out = render_template(html, &[row("x&y", &[("name", "<b>hi</b>")])]);
        assert_eq!(
            out,
            "<ul><li data-id=\"x&amp;y\" with=\"x&amp;y\">&lt;b&gt;hi&lt;/b&gt;</li></ul>"
        );
    }

    #[test]
    fn it_iterates_a_many_valued_field_within_a_row() {
        // {this} on the <li> lifts the repeat root there; the inner
        // <span subject={tags}>{tags}</span> iterates the many-valued
        // tags field within each row.
        let html = "<ul><li data-id={this}><span>{tags}</span></li></ul>";
        let mut fields = BTreeMap::new();
        fields.insert(
            "tags".to_string(),
            Ipld::List(vec![
                Ipld::String("red".into()),
                Ipld::String("blue".into()),
            ]),
        );
        let frame = vec![Conclusion {
            this: "a".to_string(),
            fields,
        }];
        let out = render_template(html, &frame);
        // The <span> clones once per tag value.
        assert_eq!(
            out,
            "<ul><li data-id=\"a\" with=\"a\"><span>red</span><span>blue</span></li></ul>"
        );
    }
}
