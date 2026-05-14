//! Diff a stream of [`tonk_schema::conclusion::Conclusion`]
//! frames into the live DOM. One row per conclusion; identity is
//! the `this` URI.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, Node};

use crate::template::{Binding, BindingKind, BindingPlan, Segment, navigate, render_segments};

/// One rendered row of the live view.
struct Row {
    /// Top-level cloned nodes (a template can have multiple roots).
    nodes: Vec<Node>,
    /// Per-binding rendered string from the last frame, keyed by
    /// binding index. Used to skip writes when nothing changed.
    last_values: Vec<String>,
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
    pub fn apply(&mut self, frame: &[Conclusion]) {
        let mut seen: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        for conclusion in frame {
            seen.insert(conclusion.this.clone());
            if self.rows.contains_key(&conclusion.this) {
                self.update_row(conclusion);
            } else {
                self.insert_row(conclusion);
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
                for n in row.nodes {
                    if let Some(parent) = n.parent_node() {
                        let _ = parent.remove_child(&n);
                    }
                }
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

        // Render every binding into the clone, recording values.
        let mut values: Vec<String> = Vec::with_capacity(self.plan.bindings.len());
        for binding in &self.plan.bindings {
            let rendered = render_binding(binding, conclusion);
            apply_binding(&clone, binding, &rendered);
            values.push(rendered);
        }

        // Snapshot the top-level nodes before appending so we can
        // remove them later.
        let mut nodes: Vec<Node> = Vec::new();
        let children = clone.child_nodes();
        for i in 0..children.length() {
            if let Some(n) = children.item(i) {
                nodes.push(n);
            }
        }
        // Mark the first root with data-this for diff visibility.
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
                last_values: values,
            },
        );
    }

    fn update_row(&mut self, conclusion: &Conclusion) {
        // Walk every binding; for those whose value changed, find
        // the target node within the row's roots and patch it.
        let row = match self.rows.get_mut(&conclusion.this) {
            Some(r) => r,
            None => return,
        };
        for (i, binding) in self.plan.bindings.iter().enumerate() {
            let rendered = render_binding(binding, conclusion);
            if let Some(prev) = row.last_values.get(i)
                && *prev == rendered
            {
                continue;
            }
            patch_row(row, binding, &rendered);
            if let Some(slot) = row.last_values.get_mut(i) {
                *slot = rendered;
            }
        }
    }
}

fn render_binding(binding: &Binding, conclusion: &Conclusion) -> String {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    render_segments(segments, &conclusion.this, &conclusion.fields)
}

/// Apply a binding inside a freshly-cloned fragment. The binding's
/// `path` is rooted at the fragment.
fn apply_binding(fragment: &DocumentFragment, binding: &Binding, rendered: &str) {
    let root: Node = fragment.clone().into();
    let Some(target) = navigate(&root, &binding.path) else {
        return;
    };
    write_binding(&target, binding, rendered);
}

/// Apply a binding inside an already-mounted row. The binding's
/// `path[0]` indexes into `row.nodes` (the row's roots), and the
/// rest of the path walks down through that root's children.
fn patch_row(row: &Row, binding: &Binding, rendered: &str) {
    let Some(&first) = binding.path.first() else {
        return;
    };
    let Some(root) = row.nodes.get(first) else {
        return;
    };
    let rest = &binding.path[1..];
    let Some(target) = navigate(root, rest) else {
        return;
    };
    write_binding(&target, binding, rendered);
}

fn write_binding(target: &Node, binding: &Binding, rendered: &str) {
    match &binding.kind {
        BindingKind::Text { .. } => {
            target.set_text_content(Some(rendered));
        }
        BindingKind::Attribute { attr_name, .. } => {
            if let Some(el) = target.dyn_ref::<Element>() {
                let _ = el.set_attribute(attr_name, rendered);
            }
        }
    }
}

// Silence unused-import warning when `Segment` and `BTreeMap` are
// only used through the public `render_segments` re-export above.
const _: fn() = || {
    let _: Option<Segment> = None;
    let _: BTreeMap<String, serde_json::Value> = BTreeMap::new();
};

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
    fn it_updates_in_place_on_change() {
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
            .expect("second article");
        assert!(article_before.is_same_node(Some(article_after.unchecked_ref())));
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
    fn it_dedupes_writes_when_field_value_unchanged() {
        let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let h1 = host.query_selector("h1").unwrap().expect("h1");
        let text_node_before = h1.first_child().expect("h1 text node");
        renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
        let h1_again = host.query_selector("h1").unwrap().expect("h1");
        let text_node_after = h1_again.first_child().expect("h1 text node 2");
        assert!(
            text_node_before.is_same_node(Some(text_node_after.unchecked_ref())),
            "unchanged frame should not touch the DOM",
        );
    }
}
