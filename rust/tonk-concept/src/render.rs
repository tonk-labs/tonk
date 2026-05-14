//! Diff a stream of [`tonk_schema::conclusion::Conclusion`]
//! frames into the live DOM. One row per conclusion; identity is
//! the `this` URI.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, Node};

use crate::template::{
    Binding, BindingKind, BindingPlan, PlanNode, navigate, render_segments_with_shadow,
};

/// One rendered row of the live view.
struct Row {
    /// Top-level cloned nodes (a template can have multiple roots).
    nodes: Vec<Node>,
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
    /// Existing rows are torn down and re-rendered on update so
    /// the iteration-tree walk handles cardinality-many fields
    /// without an in-place diff. The previous per-binding
    /// `last_values` dedupe is gone — re-add behind a flag if
    /// repaint cost becomes a problem.
    pub fn apply(&mut self, frame: &[Conclusion]) {
        let mut seen: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        for conclusion in frame {
            seen.insert(conclusion.this.clone());
            // Remove any prior render for this `this`; a fresh
            // clone applies the plan tree against the new state.
            if let Some(row) = self.rows.shift_remove(&conclusion.this) {
                drop_row(row);
            }
            self.insert_row(conclusion);
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

    fn insert_row(&mut self, conclusion: &Conclusion) {
        let clone: DocumentFragment = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
            .expect("template fragment clones to a fragment");

        // Walk the plan tree against the clone.
        let root_node: Node = clone.clone().into();
        apply_nodes(&root_node, &self.plan.nodes, conclusion, &BTreeMap::new());

        // Snapshot the top-level nodes before appending so we can
        // remove them on stale-cleanup.
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
        self.rows.insert(conclusion.this.clone(), Row { nodes });
    }
}

fn drop_row(row: Row) {
    for n in row.nodes {
        if let Some(parent) = n.parent_node() {
            let _ = parent.remove_child(&n);
        }
    }
}

/// Apply every plan node in `nodes` to the subtree rooted at
/// `root`. `shadow` carries per-iteration values overriding the
/// conclusion's field lookups.
fn apply_nodes(
    root: &Node,
    nodes: &[PlanNode],
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    for node in nodes {
        match node {
            PlanNode::Binding(b) => apply_binding_at(root, b, conclusion, shadow),
            PlanNode::Iteration { field, path, body } => {
                apply_iteration_at(root, field, path, body, conclusion, shadow);
            }
        }
    }
}

fn apply_binding_at(
    root: &Node,
    binding: &Binding,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    let rendered =
        render_segments_with_shadow(segments, &conclusion.this, &conclusion.fields, shadow);
    let Some(target) = navigate(root, &binding.path) else {
        return;
    };
    match &binding.kind {
        BindingKind::Text { .. } => {
            target.set_text_content(Some(&rendered));
        }
        BindingKind::Attribute { attr_name, .. } => {
            if let Some(el) = target.dyn_ref::<Element>() {
                let _ = el.set_attribute(attr_name, &rendered);
            }
        }
    }
}

fn apply_iteration_at(
    root: &Node,
    field: &str,
    path: &[usize],
    body: &[PlanNode],
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    let Some(iter_root) = navigate(root, path) else {
        return;
    };
    let Some(parent) = iter_root.parent_node() else {
        return;
    };

    let raw_value = shadow
        .get(field)
        .or_else(|| conclusion.fields.get(field))
        .cloned();
    let values = collect_values(raw_value);

    for value in &values {
        let Some(clone) = iter_root.clone_node_with_deep(true).ok() else {
            continue;
        };
        let mut nested_shadow = shadow.clone();
        nested_shadow.insert(field.to_owned(), value.clone());
        apply_nodes(&clone, body, conclusion, &nested_shadow);
        let _ = parent.insert_before(&clone, Some(&iter_root));
    }

    let _: Result<Node, _> = parent.remove_child(&iter_root);
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
    fn it_reflects_field_changes_on_subsequent_frames() {
        // The renderer re-clones the template on every frame, so
        // node identity is *not* preserved across updates — only
        // the latest payload's content is required to surface.
        // (If we add per-binding diffing back, this test should
        // tighten to assert identity preservation again.)
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alicia")])]);
        assert!(host.inner_html().contains("Alicia"));
        assert!(
            !host.inner_html().contains("Alice<"),
            "stale row leaked into: {}",
            host.inner_html(),
        );
        assert_eq!(
            host.query_selector_all("article").unwrap().length(),
            1,
            "expected one row, got: {}",
            host.inner_html(),
        );
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
    fn it_appends_new_rows() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        renderer.apply(&[
            conclusion("did:key:zAlice", &[("name", "Alice")]),
            conclusion("did:key:zBob", &[("name", "Bob")]),
        ]);
        assert_eq!(host.query_selector_all("article").unwrap().length(), 2);
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
        // The renderer re-clones on every frame; applying the
        // same conclusion twice leaves the DOM in the same shape
        // and content. Per-binding write-deduping (which would
        // preserve node identity) was dropped with the move to
        // iteration-aware rendering — re-add behind a flag if
        // repaint cost matters.
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        assert_eq!(
            host.query_selector_all("article").unwrap().length(),
            1,
            "expected exactly one row after a repeat frame",
        );
        assert!(host.inner_html().contains("Alice"));
    }
}
