//! Diff a stream of [`tonk_schema::conclusion::Conclusion`]
//! frames into the live DOM. One row per conclusion; identity is
//! the `this` URI.
//!
//! Each `this`-keyed row owns a mounted-state subtree mirroring
//! the [`BindingPlan`]; subsequent applies reconcile in place
//! per row. New rows get fresh template clones; vanished rows
//! are detached. The per-row mounted state matches
//! `tonk-display::render::Renderer`'s shape — both crates use the
//! same algorithm, kept duplicated to avoid pulling wasm-only DOM
//! types into a shared crate.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{Document, DocumentFragment, Element, Node, window};

use crate::template::{
    Binding, BindingKind, BindingPlan, PlanNode, navigate, render_segments_with_shadow,
};

/// One rendered row keyed by its `this` URI. Owns the live
/// top-level nodes plus the mounted plan-tree subtree used to
/// reconcile this row on subsequent applies without re-cloning.
struct Row {
    /// Top-level cloned nodes (a template can have multiple roots).
    nodes: Vec<Node>,
    /// Mounted plan-tree mirror for incremental updates.
    mounted: Vec<MountedNode>,
    /// The fragment-cloned root the mounted state navigates from.
    /// Kept alive so we have a stable handle into the row's
    /// subtree across applies.
    root: Node,
}

/// One mounted plan-tree node. Mirrors [`PlanNode`] but carries
/// only the DOM bookkeeping needed to update in place; path /
/// kind / segments stay on the plan node, walked in lockstep with
/// the mounted tree during reconciliation.
enum MountedNode {
    /// A leaf binding with its last-rendered string cached so we
    /// can skip writes that wouldn't change anything.
    Binding {
        /// Most recent rendered string. Compared against fresh
        /// renders; equal ⇒ no write, original DOM node preserved.
        last_value: String,
    },
    /// An iteration over a field's values. Owns its rows and the
    /// comment-node anchor in the DOM that marks the iteration's
    /// slot.
    Iteration {
        /// Path to the iteration root in the template fragment.
        template_path: Vec<usize>,
        /// Comment anchor at the iteration's slot. Rows insert
        /// *before* this node; the anchor stays put across applies.
        anchor: Node,
        /// Mounted rows, keyed by stringified iteration value.
        /// BTreeMap ⇒ DOM order follows lexicographic key order.
        rows: BTreeMap<String, IterationRow>,
    },
}

/// One mounted iteration row.
struct IterationRow {
    /// The cloned iteration-root element in the live DOM.
    root: Node,
    /// Mounted state for the body bindings, relative to `root`.
    body: Vec<MountedNode>,
}

/// Stateful renderer — owns the template + plan and tracks which
/// rows are currently in the DOM.
pub struct Renderer {
    plan: BindingPlan,
    template: DocumentFragment,
    container: Element,
    rows: IndexMap<String, Row>,
}

impl Renderer {
    /// Construct a renderer over a snapshot.
    pub fn new(plan: BindingPlan, template: DocumentFragment, container: Element) -> Self {
        Self {
            plan,
            template,
            container,
            rows: IndexMap::new(),
        }
    }

    /// Remove every currently-rendered row from the DOM. Used when
    /// an attribute change tears the subscription down and the
    /// caller wants a clean slate before the next frame arrives.
    pub fn clear(&mut self) {
        for (_, row) in std::mem::take(&mut self.rows) {
            for n in row.nodes {
                if let Some(parent) = n.parent_node() {
                    let _ = parent.remove_child(&n);
                }
            }
        }
    }

    /// Apply one wire frame. Adds, updates, and removes rows so
    /// the live DOM reflects exactly the conclusions in `frame`.
    ///
    /// Surviving rows reconcile in place against the new
    /// conclusion (per-binding write dedupe + keyed iteration
    /// diffing). New rows clone the template and build their
    /// mounted state from scratch. Vanished rows are detached and
    /// their mounted state dropped.
    pub fn apply(&mut self, frame: &[Conclusion]) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let mut seen: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        for conclusion in frame {
            seen.insert(conclusion.this.clone());
            if let Some(row) = self.rows.get_mut(&conclusion.this) {
                update_nodes(
                    &document,
                    &self.plan.nodes,
                    &mut row.mounted,
                    &row.root,
                    &self.template,
                    conclusion,
                    &BTreeMap::new(),
                );
            } else {
                self.insert_row(&document, conclusion);
            }
        }
        // Remove rows that vanished from the frame.
        let stale: Vec<String> = self
            .rows
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        for key in stale {
            if let Some(row) = self.rows.shift_remove(&key) {
                drop_row(row);
            }
        }
    }

    fn insert_row(&mut self, document: &Document, conclusion: &Conclusion) {
        let clone: DocumentFragment = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
            .expect("template fragment clones to a fragment");

        let root: Node = clone.clone().into();
        let mounted = build_mounted_nodes(
            document,
            &self.plan.nodes,
            &root,
            &self.template,
            conclusion,
            &BTreeMap::new(),
        );

        // Snapshot the top-level nodes after the plan walk —
        // iteration nodes may have rearranged the fragment's
        // children (replaced iteration roots with anchors + rows).
        let mut nodes: Vec<Node> = Vec::new();
        let children = clone.child_nodes();
        for i in 0..children.length() {
            if let Some(n) = children.item(i) {
                nodes.push(n);
            }
        }
        if let Some(first) = nodes.first().and_then(|n| n.dyn_ref::<Element>())
            && first.set_attribute("data-this", &conclusion.this).is_err()
        {
            // Non-element root or read-only — ignore silently.
        }

        let _ = self.container.append_child(&clone);
        self.rows.insert(
            conclusion.this.clone(),
            Row {
                nodes,
                mounted,
                root,
            },
        );
    }
}

fn drop_row(row: Row) {
    for n in row.nodes {
        if let Some(parent) = n.parent_node() {
            let _ = parent.remove_child(&n);
        }
    }
}

/// Build a fresh `Vec<MountedNode>` from a plan and write its
/// initial values into the DOM.
///
/// Iteration nodes are processed in **reverse sibling order** to
/// avoid invalidating sibling paths: replacing an earlier
/// iteration root with an anchor + rows would shift the indices
/// of later siblings, and the plan's paths are computed against
/// the pristine template. Reverse order keeps every yet-to-be-
/// processed sibling at its planned path. Leaf bindings are
/// order-independent; they're written in whichever pass touches
/// them.
fn build_mounted_nodes(
    document: &Document,
    plan: &[PlanNode],
    scope_root: &Node,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) -> Vec<MountedNode> {
    let mut out: Vec<Option<MountedNode>> = (0..plan.len()).map(|_| None).collect();
    for (i, node) in plan.iter().enumerate().rev() {
        out[i] = Some(build_mounted_node(
            document, node, scope_root, template, conclusion, shadow,
        ));
    }
    out.into_iter()
        .map(|n| n.expect("every slot filled"))
        .collect()
}

fn build_mounted_node(
    document: &Document,
    plan: &PlanNode,
    scope_root: &Node,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) -> MountedNode {
    match plan {
        PlanNode::Binding(b) => {
            let rendered = render_binding(b, conclusion, shadow);
            write_binding(scope_root, b, &rendered);
            MountedNode::Binding {
                last_value: rendered,
            }
        }
        PlanNode::Iteration { field, path, body } => {
            let anchor: Node = document
                .create_comment(&format!("tonk-iter:{field}"))
                .into();
            let mut rows: BTreeMap<String, IterationRow> = BTreeMap::new();

            if let Some(iter_root) = navigate(scope_root, path)
                && let Some(parent) = iter_root.parent_node()
            {
                let _ = parent.insert_before(&anchor, Some(&iter_root));
                let _: Result<Node, _> = parent.remove_child(&iter_root);

                let raw_value = shadow
                    .get(field)
                    .or_else(|| conclusion.fields.get(field))
                    .cloned();
                let values = collect_values(raw_value);

                for value in values {
                    let key = key_for(&value);
                    if rows.contains_key(&key) {
                        continue;
                    }
                    if let Some(row) = build_iteration_row(
                        document, path, template, body, field, value, conclusion, shadow,
                    ) {
                        // BTreeMap iteration is already sorted by
                        // key; inserting each new row before the
                        // anchor yields sorted DOM order without
                        // explicit position math.
                        let _ = parent.insert_before(&row.root, Some(&anchor));
                        rows.insert(key, row);
                    }
                }
            }

            MountedNode::Iteration {
                template_path: path.clone(),
                anchor,
                rows,
            }
        }
    }
}

fn update_nodes(
    document: &Document,
    plan: &[PlanNode],
    mounted: &mut [MountedNode],
    scope_root: &Node,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    for (plan_node, mounted_node) in plan.iter().zip(mounted.iter_mut()) {
        update_node(
            document,
            plan_node,
            mounted_node,
            scope_root,
            template,
            conclusion,
            shadow,
        );
    }
}

fn update_node(
    document: &Document,
    plan: &PlanNode,
    mounted: &mut MountedNode,
    scope_root: &Node,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    match (plan, mounted) {
        (PlanNode::Binding(b), MountedNode::Binding { last_value }) => {
            let rendered = render_binding(b, conclusion, shadow);
            if *last_value != rendered {
                write_binding(scope_root, b, &rendered);
                *last_value = rendered;
            }
        }
        (
            PlanNode::Iteration {
                field: plan_field,
                path: _,
                body,
            },
            MountedNode::Iteration {
                template_path,
                anchor,
                rows,
            },
        ) => {
            update_iteration(
                document,
                plan_field,
                template_path,
                body,
                anchor,
                rows,
                template,
                conclusion,
                shadow,
            );
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn update_iteration(
    document: &Document,
    field: &str,
    template_path: &[usize],
    plan_body: &[PlanNode],
    anchor: &Node,
    rows: &mut BTreeMap<String, IterationRow>,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    let raw_value = shadow
        .get(field)
        .or_else(|| conclusion.fields.get(field))
        .cloned();
    let values = collect_values(raw_value);

    let mut incoming: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for value in values {
        let key = key_for(&value);
        incoming.entry(key).or_insert(value);
    }

    let Some(parent) = anchor.parent_node() else {
        return;
    };

    let stale: Vec<String> = rows
        .keys()
        .filter(|k| !incoming.contains_key(*k))
        .cloned()
        .collect();
    for key in stale {
        if let Some(row) = rows.remove(&key) {
            let _: Result<Node, _> = parent.remove_child(&row.root);
        }
    }

    for (key, value) in incoming {
        if let Some(row) = rows.get_mut(&key) {
            let mut nested_shadow = shadow.clone();
            nested_shadow.insert(field.to_owned(), value);
            update_nodes(
                document,
                plan_body,
                &mut row.body,
                &row.root,
                template,
                conclusion,
                &nested_shadow,
            );
        } else if let Some(new_row) = build_iteration_row(
            document,
            template_path,
            template,
            plan_body,
            field,
            value,
            conclusion,
            shadow,
        ) {
            // Single insertBefore at the correct sorted position.
            // Inserting then moving would fire detach/attach
            // lifecycle on every custom element inside the row,
            // doubling subscriptions on inner `<tonk-display>`
            // elements. See `build_iteration_row` doc.
            let next_anchor = rows
                .range(key.clone()..)
                .next()
                .map(|(_, row)| row.root.clone())
                .unwrap_or_else(|| anchor.clone());
            let _ = parent.insert_before(&new_row.root, Some(&next_anchor));
            rows.insert(key, new_row);
        }
    }
}

/// Clone the iteration root from the pristine template and
/// build the body's mounted state with all bindings applied.
/// The returned [`IterationRow`] is **detached** — the caller
/// is responsible for the single `parent.insertBefore` that
/// attaches it.
///
/// Inserting then moving would amount to a detach/attach pair
/// on every custom element inside the row, firing
/// `disconnected_callback` and `connected_callback` twice and
/// causing inner `<tonk-display>` instances to mount their
/// view slide twice. Building detached lets every attribute
/// settle to its bound value before the row enters the live DOM,
/// so the inner element's lifecycle runs exactly once.
#[allow(clippy::too_many_arguments)]
fn build_iteration_row(
    document: &Document,
    template_path: &[usize],
    template: &DocumentFragment,
    body_plan: &[PlanNode],
    field: &str,
    value: serde_json::Value,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) -> Option<IterationRow> {
    let template_root: Node = template.clone().into();
    let template_iter_root = navigate(&template_root, template_path)?;
    let row_root = template_iter_root.clone_node_with_deep(true).ok()?;

    let mut nested_shadow = shadow.clone();
    nested_shadow.insert(field.to_owned(), value);
    let body = build_mounted_nodes(
        document,
        body_plan,
        &row_root,
        template,
        conclusion,
        &nested_shadow,
    );

    Some(IterationRow {
        root: row_root,
        body,
    })
}

fn key_for(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn render_binding(
    binding: &Binding,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) -> String {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    render_segments_with_shadow(segments, &conclusion.this, &conclusion.fields, shadow)
}

fn write_binding(scope_root: &Node, binding: &Binding, rendered: &str) {
    let Some(target) = navigate(scope_root, &binding.path) else {
        return;
    };
    match &binding.kind {
        BindingKind::Text { .. } => target.set_text_content(Some(rendered)),
        BindingKind::Attribute { attr_name, .. } => {
            if let Some(el) = target.dyn_ref::<Element>() {
                let _ = el.set_attribute(attr_name, rendered);
            }
        }
    }
}

fn collect_values(value: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match value {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items,
        Some(v) => vec![v],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{extract_plan, snapshot_template};

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Mount a fresh `<div>` host with the given inner HTML and
    /// build a [`Renderer`] over it. Browser-only: uses
    /// `window().document()`.
    fn mount(inner_html: &str) -> (Element, Renderer) {
        let document = web_sys::window()
            .expect("window")
            .document()
            .expect("document");
        let host: Element = document.create_element("div").expect("create div");
        host.set_inner_html(inner_html);
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("attach host");
        let snapshot = snapshot_template(&host).expect("snapshot");
        let plan = extract_plan(&snapshot.fragment);
        (
            host,
            Renderer::new(plan, snapshot.fragment, snapshot.container),
        )
    }

    /// Build a [`Conclusion`] with the given `this` URI and
    /// string fields.
    fn conclusion(this: &str, fields: &[(&str, &str)]) -> Conclusion {
        let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), serde_json::Value::String((*v).to_owned()));
        }
        Conclusion {
            this: this.to_owned(),
            fields: map,
        }
    }

    #[dialog_common::test]
    fn it_renders_template_per_conclusion() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        let frame = vec![
            conclusion("did:key:zAlice", &[("name", "Alice")]),
            conclusion("did:key:zBob", &[("name", "Bob")]),
        ];
        renderer.apply(&frame);
        let html = host.inner_html();
        assert!(html.contains("Alice"), "expected Alice in {html}");
        assert!(html.contains("Bob"), "expected Bob in {html}");
        assert_eq!(
            host.query_selector_all("article").unwrap().length(),
            2,
            "expected two row articles, got: {html}",
        );
    }

    #[dialog_common::test]
    fn it_updates_a_row_in_place_on_subsequent_frame() {
        // With incremental updates, an existing row's article
        // node has its identity preserved across applies — only
        // the changed text node inside is rewritten.
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let article_before = host
            .query_selector("article")
            .unwrap()
            .expect("first article");

        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alicia")])]);
        let article_after = host
            .query_selector("article")
            .unwrap()
            .expect("article still mounted");

        assert!(
            article_before.is_same_node(Some(article_after.unchecked_ref())),
            "article node identity should survive a content change",
        );
        assert!(host.inner_html().contains("Alicia"));
        assert!(!host.inner_html().contains("Alice<"));
    }

    #[dialog_common::test]
    fn it_removes_rows_dropped_from_frame() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[
            conclusion("did:key:zAlice", &[("name", "Alice")]),
            conclusion("did:key:zBob", &[("name", "Bob")]),
        ]);
        assert_eq!(host.query_selector_all("article").unwrap().length(), 2);
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let html = host.inner_html();
        assert!(html.contains("Alice"));
        assert!(!html.contains("Bob"), "Bob row should be gone: {html}");
        assert_eq!(host.query_selector_all("article").unwrap().length(), 1);
    }

    #[dialog_common::test]
    fn it_appends_new_rows_without_disturbing_existing_ones() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let alice_article = host
            .query_selector("article")
            .unwrap()
            .expect("Alice's article");

        renderer.apply(&[
            conclusion("did:key:zAlice", &[("name", "Alice")]),
            conclusion("did:key:zBob", &[("name", "Bob")]),
        ]);

        let articles = host.query_selector_all("article").unwrap();
        assert_eq!(articles.length(), 2);
        let alice_after = articles.item(0).expect("first article");
        assert!(
            alice_article.is_same_node(Some(alice_after.unchecked_ref())),
            "Alice's row should retain identity when Bob is added",
        );
        assert!(host.inner_html().contains("Bob"));
    }

    #[dialog_common::test]
    fn it_substitutes_into_attribute_values() {
        let (host, mut renderer) = mount(r#"<a href="/entity/{this}">link</a>"#);
        renderer.apply(&[conclusion("did:key:zAlice", &[])]);
        let a = host.query_selector("a").unwrap().expect("anchor");
        assert_eq!(
            a.get_attribute("href").as_deref(),
            Some("/entity/did:key:zAlice"),
        );
    }

    #[dialog_common::test]
    fn it_uses_template_element_when_present() {
        let (host, mut renderer) = mount(
            r#"<table><thead><tr><th>Name</th></tr></thead>
               <tbody><template><tr><td>{name}</td></tr></template></tbody></table>"#,
        );
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        assert!(
            host.query_selector("table > thead").unwrap().is_some(),
            "expected static <thead>",
        );
        let tbody = host
            .query_selector("table > tbody")
            .unwrap()
            .expect("tbody");
        assert_eq!(
            tbody.query_selector_all("tr").unwrap().length(),
            1,
            "expected one row in tbody",
        );
        assert!(tbody.inner_html().contains("Alice"));
    }

    #[dialog_common::test]
    fn it_falls_back_to_first_child_when_no_template() {
        let (host, mut renderer) = mount("<article>{name}</article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let article = host.query_selector("article").unwrap().expect("article");
        assert_eq!(article.text_content().as_deref(), Some("Alice"));
    }

    #[dialog_common::test]
    fn it_clears_every_row_from_the_dom() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[
            conclusion("did:key:zAlice", &[("name", "Alice")]),
            conclusion("did:key:zBob", &[("name", "Bob")]),
        ]);
        assert_eq!(host.query_selector_all("article").unwrap().length(), 2);
        renderer.clear();
        assert_eq!(
            host.query_selector_all("article").unwrap().length(),
            0,
            "clear() must remove every row's nodes from the DOM",
        );
    }

    #[dialog_common::test]
    fn it_handles_identical_frames_idempotently() {
        // Re-applying the same conclusion must be a no-op for the
        // DOM: the rendered text node retains its identity since
        // nothing changed.
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let h1 = host.query_selector("h1").unwrap().expect("h1");
        let text_before = h1.first_child().expect("h1 text node");

        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let text_after = host
            .query_selector("h1")
            .unwrap()
            .unwrap()
            .first_child()
            .expect("h1 text node still present");
        assert!(
            text_before.is_same_node(Some(text_after.unchecked_ref())),
            "unchanged binding should not rewrite the text node",
        );
    }
}
