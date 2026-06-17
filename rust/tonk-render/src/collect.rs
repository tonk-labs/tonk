//! Collect bindings from the native node tree, mirroring
//! `tonk-display`'s browser `collect_bindings` exactly so the
//! resulting [`Binding`] paths feed the shared
//! [`tonk_template`] planner identically.
//!
//! Like the browser collector it **mutates the tree**: an
//! interpolated text node such as `"Hi {name}!"` is split into a
//! run of single-segment text nodes (`"Hi "`, `""`, `"!"`) so each
//! `{field}` gets its own targetable node, and the emitted binding
//! paths address the post-split tree. The renderer later navigates
//! that same split tree.

use tonk_template::{Binding, BindingKind, has_field, parse_segments};

use crate::tree::{Node, is_raw_text_element};

/// Walk `roots` (a fragment's top-level nodes), split interpolated
/// text nodes in place, and return the flat list of bindings. The
/// tree is mutated to match the binding paths.
pub fn collect_bindings(roots: &mut Vec<Node>) -> Vec<Binding> {
    let mut bindings = Vec::new();
    collect_in_children(roots, &mut Vec::new(), &mut bindings);
    bindings
}

/// Recurse over a child list, emitting bindings and splitting text
/// nodes. `path` is the child-index path from the fragment root to
/// the parent of `children`.
fn collect_in_children(children: &mut Vec<Node>, path: &mut Vec<usize>, out: &mut Vec<Binding>) {
    // Text-node splitting changes the child list length, so walk by
    // index and advance manually. When a text node splits into N
    // nodes, the cursor steps over all N (none of them needs another
    // visit: text content has no descendants).
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            Node::Text(raw) => {
                if raw.contains('{') {
                    let segments = parse_segments(raw);
                    if has_field(&segments) {
                        // Build replacement nodes in document order
                        // and emit a binding per Field segment at
                        // index `i + seg_index` (the browser's
                        // `original_idx + i` rule).
                        let mut replacement: Vec<Node> = Vec::with_capacity(segments.len());
                        for (seg_index, seg) in segments.iter().enumerate() {
                            match seg {
                                tonk_template::Segment::Text(t) => {
                                    replacement.push(Node::Text(t.clone()));
                                }
                                tonk_template::Segment::Field(_) => {
                                    replacement.push(Node::Text(String::new()));
                                    let mut new_path = path.clone();
                                    new_path.push(i + seg_index);
                                    out.push(Binding {
                                        path: new_path,
                                        kind: BindingKind::Text {
                                            segments: vec![seg.clone()],
                                        },
                                    });
                                }
                            }
                        }
                        let count = replacement.len();
                        children.splice(i..=i, replacement);
                        i += count;
                        continue;
                    }
                }
                i += 1;
            }
            Node::Comment(_) => {
                i += 1;
            }
            Node::Element(_) => {
                // Collect attribute bindings on this element, then
                // (unless it is style/script) descend into it.
                path.push(i);
                collect_element_attrs(&mut children[i], path, out);
                if !is_raw_text_element(&children[i])
                    && let Node::Element(el) = &mut children[i]
                {
                    collect_in_children(&mut el.children, path, out);
                }
                path.pop();
                i += 1;
            }
        }
    }
}

/// Emit attribute bindings for one element at `path`, applying the
/// `html:` force-attribute rule (strip the prefix, drop the
/// prefixed source attribute, mark `force_attribute`).
fn collect_element_attrs(node: &mut Node, path: &[usize], out: &mut Vec<Binding>) {
    let Node::Element(el) = node else {
        return;
    };
    // Collect interpolated attributes in source order, then mutate
    // (removing `html:`-prefixed source attrs) after, so we don't
    // disturb the iteration.
    let interpolated: Vec<(String, String)> = el
        .attrs
        .iter()
        .filter(|(_, v)| v.contains('{'))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    for (name, value) in interpolated {
        let segments = parse_segments(&value);
        if !has_field(&segments) {
            continue;
        }
        let (attr_name, force_attribute) = match name.strip_prefix("html:") {
            Some(stripped) => {
                // Rename the `html:`-prefixed source attribute to its
                // stripped name IN PLACE (rather than removing it), so the
                // forced attribute keeps its original position in the
                // element's attribute list. The renderer then updates that
                // same slot, matching the browser's serialized order.
                let stripped = stripped.to_string();
                if let Some(slot) = el.attrs.iter_mut().find(|(k, _)| k == &name) {
                    slot.0 = stripped.clone();
                }
                (stripped, true)
            }
            None => (name, false),
        };
        out.push(Binding {
            path: path.to_vec(),
            kind: BindingKind::Attribute {
                attr_name,
                segments,
                force_attribute,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_fragment;
    use crate::tree::Node;
    use tonk_template::{BindingKind, Segment};

    /// Helper: parse, collect, return (mutated tree, bindings).
    fn run(html: &str) -> (Vec<Node>, Vec<Binding>) {
        let mut roots = parse_fragment(html);
        let bindings = collect_bindings(&mut roots);
        (roots, bindings)
    }

    fn text_binding_paths(bindings: &[Binding]) -> Vec<(Vec<usize>, String)> {
        bindings
            .iter()
            .filter_map(|b| match &b.kind {
                BindingKind::Text { segments } => match segments.as_slice() {
                    [Segment::Field(name)] => Some((b.path.clone(), name.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// The render-test `LIST` template: `data-id={this}` is an
    /// attribute binding on the <li>; `{name}` is a text binding
    /// inside it.
    #[test]
    fn it_collects_the_list_template() {
        let (_tree, bindings) = run("<ul><li data-id={this}>{name}</li></ul>");

        // One attribute binding (data-id={this}) at <li>: path [0,0]
        // (ul is root[0], li is its child[0]).
        let attrs: Vec<_> = bindings
            .iter()
            .filter_map(|b| match &b.kind {
                BindingKind::Attribute { attr_name, .. } => {
                    Some((b.path.clone(), attr_name.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(attrs, vec![(vec![0, 0], "data-id".to_string())]);

        // One text binding ({name}) at the text node inside <li>:
        // li's only child is the text, index 0, so path [0,0,0].
        assert_eq!(
            text_binding_paths(&bindings),
            vec![(vec![0, 0, 0], "name".to_string())]
        );
    }

    /// A text node with surrounding literals and two fields splits
    /// into per-segment text nodes; field bindings target the split
    /// indices (the browser's `original_idx + seg_index` rule).
    #[test]
    fn it_splits_a_multi_field_text_node() {
        // <p>Hi {first} {last}!</p>
        // segments: [Text("Hi "), Field(first), Text(" "), Field(last), Text("!")]
        // indices:        0          1            2          3            4
        let (tree, bindings) = run("<p>Hi {first} {last}!</p>");

        assert_eq!(
            text_binding_paths(&bindings),
            vec![
                (vec![0, 1], "first".to_string()),
                (vec![0, 3], "last".to_string()),
            ]
        );

        // The <p>'s children are now the 5 split text nodes.
        let Node::Element(p) = &tree[0] else {
            panic!("expected <p>");
        };
        assert_eq!(p.children.len(), 5);
        assert_eq!(p.children[0], Node::Text("Hi ".to_string()));
        assert_eq!(p.children[1], Node::Text(String::new()));
        assert_eq!(p.children[4], Node::Text("!".to_string()));
    }

    /// Two sibling text nodes that both split: the second's field
    /// indices must reflect the first's expansion (cursor-relative,
    /// matching the live DOM positions the browser inserts at).
    #[test]
    fn it_splits_sibling_text_nodes_with_shifted_indices() {
        // <p>{a}<br>{b} {c}</p>
        // children pre-split: [Text("{a}"), <br>, Text("{b} {c}")]
        //                        0            1      2
        // After splitting child 0 -> [Text("")] (1 node, same length here),
        // <br> stays at 1, then child 2 "{b} {c}" splits into
        // [Field b -> ""], Text(" "), [Field c -> ""] at indices 2,3,4.
        let (_tree, bindings) = run("<p>{a}<br>{b} {c}</p>");
        assert_eq!(
            text_binding_paths(&bindings),
            vec![
                (vec![0, 0], "a".to_string()),
                (vec![0, 2], "b".to_string()),
                (vec![0, 4], "c".to_string()),
            ]
        );
    }

    /// `html:foo={x}` forces an attribute, strips the prefix, and
    /// removes the prefixed source attribute from the tree.
    #[test]
    fn it_handles_the_html_force_attribute_prefix() {
        let (tree, bindings) = run(r#"<a html:href={url}>x</a>"#);
        let attr = bindings
            .iter()
            .find_map(|b| match &b.kind {
                BindingKind::Attribute {
                    attr_name,
                    force_attribute,
                    ..
                } => Some((attr_name.clone(), *force_attribute)),
                _ => None,
            })
            .expect("an attribute binding");
        assert_eq!(attr, ("href".to_string(), true));

        // The literal `html:href` attribute is gone from the tree.
        let Node::Element(a) = &tree[0] else {
            panic!("expected <a>");
        };
        assert!(a.attrs.iter().all(|(k, _)| k != "html:href"));
    }

    /// `<style>`/`<script>` content is not walked for bindings: a
    /// `{ ... }` inside CSS is real braces, not a field.
    #[test]
    fn it_does_not_descend_into_style_or_script() {
        let (_tree, bindings) = run("<div><style>.x{color:{c}}</style>{name}</div>");
        // Only the {name} text binding outside <style> is collected.
        assert_eq!(
            text_binding_paths(&bindings),
            vec![(vec![0, 1], "name".to_string())]
        );
    }

    /// End-to-end through the shared planner: the native collect ->
    /// `this_repeat_root` -> `split_plan` reproduces the browser's
    /// plan for the canonical LIST template. `data-id={this}` lifts
    /// the repeat root to the <li> at path [0, 0].
    #[test]
    fn it_plans_the_list_template_like_the_browser() {
        use tonk_template::{build_plan_nodes, split_plan, this_repeat_root};

        let (_tree, bindings) = run("<ul><li data-id={this}>{name}</li></ul>");
        // Plan-folding is the same call the browser's extract_plan makes.
        let _ = build_plan_nodes(bindings.clone());
        let repeat_root = this_repeat_root(&bindings);
        assert_eq!(
            repeat_root,
            Some(vec![0, 0]),
            "{{this}} on the <li> lifts the repeat root to it"
        );

        let plan = split_plan(bindings, repeat_root);
        // The whole binding set is per-conclusion (inside the repeat),
        // so chrome is empty and the repeat path is the <li>.
        assert!(plan.chrome.is_empty(), "no chrome outside the repeat");
        assert_eq!(plan.repeat.path, Some(vec![0, 0]));
        assert!(
            !plan.repeat.body.is_empty(),
            "repeat body carries the data-id and name bindings"
        );
    }
}
