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
use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{Document, DocumentFragment, Element, Node, window};

use crate::template::{
    Binding, BindingKind, BindingPlan, PlanNode, apply_attribute_binding, navigate,
    render_segments_with_shadow, single_field_value,
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
    ///
    /// A `<tonk-concept>` shelf clones the **whole template** per
    /// conclusion (its own row loop), so it consumes `plan.repeat.body`
    /// — the per-conclusion body — and never the chrome/repeat split a
    /// `<tonk-display>` uses. Concept row templates do not carry `{this}`
    /// markers, so the split leaves every node in `repeat.body` with
    /// fragment-relative paths (`repeat.path == None`).
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
                    &self.plan.repeat.body,
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
            &self.plan.repeat.body,
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
    shadow: &BTreeMap<String, Ipld>,
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
    shadow: &BTreeMap<String, Ipld>,
) -> MountedNode {
    match plan {
        PlanNode::Binding(b) => {
            let rendered = render_binding(b, conclusion, shadow);
            write_binding(scope_root, b, &rendered, conclusion, shadow);
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
    shadow: &BTreeMap<String, Ipld>,
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
    shadow: &BTreeMap<String, Ipld>,
) {
    match (plan, mounted) {
        (PlanNode::Binding(b), MountedNode::Binding { last_value }) => {
            let rendered = render_binding(b, conclusion, shadow);
            if *last_value != rendered {
                write_binding(scope_root, b, &rendered, conclusion, shadow);
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
    shadow: &BTreeMap<String, Ipld>,
) {
    let raw_value = shadow
        .get(field)
        .or_else(|| conclusion.fields.get(field))
        .cloned();
    let values = collect_values(raw_value);

    let mut incoming: BTreeMap<String, Ipld> = BTreeMap::new();
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
    value: Ipld,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
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

fn key_for(value: &Ipld) -> String {
    match value {
        Ipld::String(s) => s.clone(),
        other => serde_ipld_dagjson::to_vec(other)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default(),
    }
}

fn render_binding(
    binding: &Binding,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> String {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    render_segments_with_shadow(segments, &conclusion.this, &conclusion.fields, shadow)
}

fn write_binding(
    scope_root: &Node,
    binding: &Binding,
    rendered: &str,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    match &binding.kind {
        BindingKind::Text { .. } => {
            if let Some(target) = navigate(scope_root, &binding.path) {
                target.set_text_content(Some(rendered));
            }
        }
        BindingKind::Attribute { .. } => {
            let value = single_field_value(binding, &conclusion.this, &conclusion.fields, shadow);
            apply_attribute_binding(scope_root, binding, rendered, value.as_ref());
        }
    }
}

fn collect_values(value: Option<Ipld>) -> Vec<Ipld> {
    match value {
        None | Some(Ipld::Null) => Vec::new(),
        Some(Ipld::List(items)) => items,
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
        let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), Ipld::String((*v).to_owned()));
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
    fn it_interpolates_slash_values_into_attributes() {
        // Regression: `model={model} view={view}` where the field
        // values contain slashes (namespaced concept names like
        // `trip/stop`, `workspace/tab`) must land on the element
        // as the exact, whole attribute value — not truncated at the
        // slash, not corrupted with a trailing slash from an adjacent
        // self-close. A self-closing `/>` is used deliberately to
        // mirror the artifact view template.
        let (host, mut renderer) =
            mount("<tonk-display entity={entity} model={model} view={view} />");
        renderer.apply(&[conclusion(
            "id:demo/sheet",
            &[
                ("entity", "id:demo/itinerary"),
                ("model", "trip/stop"),
                ("view", "workspace/tab"),
            ],
        )]);
        let el = host
            .query_selector("tonk-display")
            .unwrap()
            .expect("tonk-display present");
        assert_eq!(
            el.get_attribute("model").as_deref(),
            Some("trip/stop"),
            "model attribute must keep its slash value, got {:?} (html: {})",
            el.get_attribute("model"),
            host.inner_html(),
        );
        assert_eq!(
            el.get_attribute("view").as_deref(),
            Some("workspace/tab"),
            "view attribute must keep its slash value, got {:?}",
            el.get_attribute("view"),
        );
        assert_eq!(
            el.get_attribute("entity").as_deref(),
            Some("id:demo/itinerary"),
            "entity attribute must keep its slash value",
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

    /// Like [`conclusion`] but accepts arbitrary JSON values so
    /// tests can drive boolean / numeric properties.
    fn conclusion_json(this: &str, fields: &[(&str, Ipld)]) -> Conclusion {
        let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), v.clone());
        }
        Conclusion {
            this: this.to_owned(),
            fields: map,
        }
    }

    /// Look up the live `<input type=checkbox>` under `host` and
    /// return its `.checked` property. Re-queries each call because
    /// the iteration renderer may swap the input node on each apply.
    fn checkbox_checked(host: &Element) -> bool {
        let input: web_sys::HtmlInputElement = host
            .query_selector(r#"input[type="checkbox"]"#)
            .unwrap()
            .expect("checkbox")
            .dyn_into()
            .expect("HtmlInputElement");
        input.checked()
    }

    #[dialog_common::test]
    fn it_sets_a_boolean_field_as_property_on_a_checkbox() {
        // `checked` on <input> is a property name — bool value must
        // flow to el.checked, not to a "true"/"false" attribute the
        // browser would treat as always-checked.
        let (host, mut renderer) =
            mount(r#"<label><input type="checkbox" checked={done} /><span>{label}</span></label>"#);
        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[
                ("done", Ipld::Bool(true)),
                ("label", Ipld::String("ship it".to_owned())),
            ],
        )]);
        assert!(
            checkbox_checked(&host),
            "el.checked should be true for done=true",
        );

        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[
                ("done", Ipld::Bool(false)),
                ("label", Ipld::String("ship it".to_owned())),
            ],
        )]);
        assert!(
            !checkbox_checked(&host),
            "el.checked should flip to false for done=false",
        );
    }

    #[dialog_common::test]
    fn it_sets_a_string_field_as_attribute_when_name_not_on_element() {
        // aria-hidden is not a property name on a span (camelCase
        // `ariaHidden` is, but the in-element check uses the literal
        // attribute name). Should round-trip as an attribute.
        let (host, mut renderer) = mount(r#"<span aria-hidden="{hidden}">x</span>"#);
        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[("hidden", Ipld::String("true".to_owned()))],
        )]);
        let span = host.query_selector("span").unwrap().expect("span");
        assert_eq!(
            span.get_attribute("aria-hidden").as_deref(),
            Some("true"),
            "aria-hidden should be set as an attribute",
        );
    }

    #[dialog_common::test]
    fn it_honours_html_prefix_as_force_attribute() {
        // `html:class={cls}` is the escape hatch: even though
        // `class` is a property name on every element, the prefix
        // pins the write to setAttribute. Use a string value so the
        // type-dispatch path doesn't already pick "attribute"; this
        // exercises the prefix.
        let (host, mut renderer) = mount(r#"<div html:id="{id}">x</div>"#);
        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[("id", Ipld::String("forced".to_owned()))],
        )]);
        let div = host.query_selector("div").unwrap().expect("div");
        assert_eq!(
            div.get_attribute("id").as_deref(),
            Some("forced"),
            "html: prefix should write a real attribute named `id`",
        );
        assert!(
            div.get_attribute("html:id").is_none(),
            "the html: prefix itself must not leak into the live attribute name",
        );
    }

    #[dialog_common::test]
    fn it_uses_html_prefix_with_boolean_for_presence_absence() {
        // Force-attribute path with a bool value: presence/absence
        // per HTML's bool-attribute convention. Wrap the div in a
        // host element that survives iteration node swaps so we can
        // re-query the (potentially fresh) inner div each apply.
        let (host, mut renderer) = mount(r#"<section><div html:hidden="{flag}">x</div></section>"#);
        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[("flag", Ipld::Bool(true))],
        )]);
        assert_eq!(
            host.query_selector("div")
                .unwrap()
                .expect("div")
                .get_attribute("hidden")
                .as_deref(),
            Some(""),
            "true should set an empty `hidden` attribute",
        );

        renderer.apply(&[conclusion_json(
            "did:key:zAlice",
            &[("flag", Ipld::Bool(false))],
        )]);
        let div = host.query_selector("div").unwrap().expect("div");
        assert!(
            div.get_attribute("hidden").is_none(),
            "false should remove the `hidden` attribute",
        );
        assert!(
            div.get_attribute("html:hidden").is_none(),
            "the html: prefix attribute must not survive on the cloned element",
        );
    }

    /// `{dom.host/model}` is a namespaced field name (with a dot and a
    /// slash). `<tonk-display>` injects host attributes under such keys
    /// so a directory template can thread the outer model into a nested
    /// display: `<tonk-display model={dom.host/model}>`. Proves the
    /// parser + substituter resolve the namespaced placeholder as an
    /// ordinary field lookup — no special casing needed.
    #[dialog_common::test]
    fn it_resolves_a_dom_host_namespaced_field_in_an_attribute() {
        let (host, mut renderer) = mount("<tonk-display entity={this} model={dom.host/model} />");
        renderer.apply(&[conclusion("id:trip/a", &[("dom.host/model", "trip")])]);
        let el = host
            .query_selector("tonk-display")
            .unwrap()
            .expect("nested tonk-display present");
        assert_eq!(
            el.get_attribute("model").as_deref(),
            Some("trip"),
            "{{dom.host/model}} must resolve to the injected field; html: {}",
            host.inner_html(),
        );
        assert_eq!(el.get_attribute("entity").as_deref(), Some("id:trip/a"));
    }
}
