//! Single-row DOM renderer used by `<tonk-view>`. Clones the
//! template fragment on each `apply`, walks the [`BindingPlan`]
//! tree, and applies bindings + iteration roots against the
//! conclusion. Iteration roots clone their sub-element once per
//! value of the bound field; the original root is then removed.
//!
//! Each `apply` re-renders from scratch — there is no in-place
//! diffing today. For a single-entity element this is cheap;
//! when we have evidence that update latency matters we can add
//! it back behind the existing `last_values` machinery.

use std::collections::BTreeMap;

use tonk_concept::template::{
    Binding, BindingKind, BindingPlan, PlanNode, Snapshot, extract_plan, navigate,
    render_segments_with_shadow,
};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{DocumentFragment, Element, Node};

/// Stateful renderer for one entity.
pub struct Renderer {
    /// Where rows get appended.
    host: Element,
    /// Cloneable template fragment captured from the host's
    /// children at construction time.
    template: DocumentFragment,
    /// Binding plan extracted from `template` at construction time.
    plan: BindingPlan,
    /// Top-level nodes of the currently mounted clone (if any),
    /// kept so we can remove them on the next `apply`.
    mounted: Vec<Node>,
}

impl Renderer {
    /// Construct a renderer from a pre-snapshotted template +
    /// container pair. Used by `<tonk-view>` after it pulls the
    /// host's children into a `DocumentFragment` at
    /// `connectedCallback`. `snapshot.container` becomes the
    /// renderer's append target.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        let plan = extract_plan(&snapshot.fragment);
        Self {
            host: snapshot.container,
            template: snapshot.fragment,
            plan,
            mounted: Vec::new(),
        }
    }

    /// Apply an entity conclusion: remove any previously rendered
    /// clone, deep-clone the template, walk the plan tree
    /// against `conclusion`, mount the result. The
    /// iteration-tree walk handles cardinality-many fields by
    /// cloning the iteration root once per value.
    pub fn apply(&mut self, conclusion: &Conclusion) {
        // Remove prior mount.
        for node in self.mounted.drain(..) {
            if let Some(parent) = node.parent_node() {
                let _: Result<Node, _> = parent.remove_child(&node);
            }
        }

        let Some(clone) = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
        else {
            return;
        };

        // Walk the plan tree against the cloned fragment.
        let root_node: Node = clone.clone().into();
        apply_nodes(&root_node, &self.plan.nodes, conclusion, &BTreeMap::new());

        // Tag the first top-level element with `data-this` for
        // CSS/consumer hooks, matching prior behaviour.
        let children = clone.child_nodes();
        let mut top: Vec<Node> = Vec::new();
        for i in 0..children.length() {
            if let Some(n) = children.item(i) {
                top.push(n);
            }
        }
        if let Some(first) = top.first().and_then(|n| n.dyn_ref::<Element>()) {
            let _ = first.set_attribute("data-this", &conclusion.this);
        }

        let _ = self.host.append_child(&clone);
        self.mounted = top;
    }
}

/// Apply every plan node in `nodes` to the subtree rooted at
/// `root` (a real `Node`, not a fragment, so we can navigate from
/// it and mutate its children). `shadow` carries per-iteration
/// values overriding the conclusion's field lookups.
fn apply_nodes(
    root: &Node,
    nodes: &[PlanNode],
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    for node in nodes {
        match node {
            PlanNode::Binding(b) => apply_binding(root, b, conclusion, shadow),
            PlanNode::Iteration { field, path, body } => {
                apply_iteration(root, field, path, body, conclusion, shadow);
            }
        }
    }
}

/// Render a leaf binding against the conclusion (+ shadow) and
/// write its value to the target DOM node.
fn apply_binding(
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

/// Clone the iteration root once per value of `field`, applying
/// `body` against each clone with `field` shadowed to the
/// iteration's value. The original root is removed; if the
/// resolved value list is empty the iteration root vanishes
/// entirely, which is the right behaviour for an empty
/// cardinality-many field.
fn apply_iteration(
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

    // Compute the value list for this iteration. The shadow
    // takes precedence (nested iterations see their parent's
    // current value); fall back to the conclusion's fields.
    let raw_value = shadow
        .get(field)
        .or_else(|| conclusion.fields.get(field))
        .cloned();
    let values = collect_values(raw_value);

    // Anchor for insertion — the original iter_root stays put
    // until we've inserted all clones before it, then we remove it.
    for value in &values {
        let Some(clone) = iter_root.clone_node_with_deep(true).ok() else {
            continue;
        };
        let mut nested_shadow = shadow.clone();
        nested_shadow.insert(field.to_owned(), value.clone());
        apply_nodes(&clone, body, conclusion, &nested_shadow);
        let _ = parent.insert_before(&clone, Some(&iter_root));
    }

    // Remove the original template node — it served only as the
    // location anchor. If `values` was empty, the iteration root
    // disappears entirely, which is the empty-list behaviour the
    // spec asks for.
    let _: Result<Node, _> = parent.remove_child(&iter_root);
}

/// Resolve a JSON value into the list of per-iteration values:
/// `Array` flattens to its elements; `Null` / missing becomes
/// empty; anything else is a single-element list. The renderer
/// uses this to decide how many times to clone an iteration root.
fn collect_values(value: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match value {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items,
        Some(v) => vec![v],
    }
}
