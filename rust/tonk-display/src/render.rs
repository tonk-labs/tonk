//! Single-row DOM renderer used by `<tonk-view>`. Maintains a
//! mounted-state tree mirroring the [`BindingPlan`] tree, so
//! repeated `apply` calls update the DOM in place rather than
//! re-cloning the template every frame.
//!
//! Three primitives:
//!
//! - **MountedBinding** caches the last rendered string for each
//!   placeholder; subsequent applies skip the write when the
//!   value hasn't changed, preserving the text/attribute node's
//!   identity (and any imperative state authors attached to it).
//! - **MountedIteration** holds a `BTreeMap` of rows keyed by the
//!   iteration value's string form, plus a comment-node anchor at
//!   the iteration's slot in the DOM. New keys clone the template
//!   and insert at the sorted position; vanished keys remove their
//!   row; surviving keys recurse into their body to update inner
//!   bindings/iterations in place.
//! - The mounted tree is built lazily on the first `apply` and
//!   reused on every subsequent one. Tearing down (e.g. element
//!   detach) drops the tree; the next apply rebuilds from scratch.
//!
//! Entity URIs are unique per cardinality-many tuple so they make
//! excellent keys; DOM order follows BTreeMap's lexicographic
//! ordering, which gives us deterministic positions across applies
//! without needing to negotiate "what order did the worker send
//! them in?"

use std::collections::BTreeMap;

use tonk_concept::template::{
    Binding, BindingKind, BindingPlan, PlanNode, Snapshot, apply_attribute_binding, extract_plan,
    navigate, render_segments_with_shadow, single_field_value,
};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{Document, DocumentFragment, Element, Node, window};

use crate::events::preprocess::{self, Bindings};

/// Stateful renderer for one entity. Holds the template,
/// extracted plan, and (after the first apply) the mounted-state
/// tree mirroring the plan.
pub struct Renderer {
    /// Where the rendered fragment lives.
    host: Element,
    /// Cloneable template fragment captured at construction time.
    /// Used for the initial mount and for cloning iteration-root
    /// subtrees on the fly when new iteration values appear.
    template: DocumentFragment,
    /// Binding plan extracted from `template` at construction time.
    plan: BindingPlan,
    /// Event-handler bindings discovered on the template by the
    /// preprocess pass. Exposed via [`Renderer::event_bindings`]
    /// so the host element can resolve concept descriptors and
    /// install delegation listeners.
    event_bindings: Bindings,
    /// The currently mounted state. `None` before the first apply;
    /// `Some` thereafter. Dropping it (e.g. when the element
    /// detaches) discards every cached value; the next apply will
    /// rebuild from scratch.
    mounted: Option<MountedScope>,
}

/// The top-level mounted scope — one `Renderer` has exactly one.
/// Holds the live root cloned from the template plus the mounted
/// nodes mirroring `plan.nodes`.
struct MountedScope {
    /// Live root in the DOM. We hold this for navigate() — the
    /// browser owns it via parent chain once appended.
    root: Node,
    /// Mirror of `plan.nodes`, in lockstep order.
    nodes: Vec<MountedNode>,
}

/// One mounted plan-tree node. Mirrors [`PlanNode`] but carries
/// only the DOM bookkeeping needed to update in place — path /
/// kind / segments stay on the plan node, which we walk in
/// lockstep with the mounted tree during reconciliation.
enum MountedNode {
    /// A leaf binding with its last-rendered string cached so we
    /// can skip writes that wouldn't change anything.
    Binding {
        /// The most recent rendered string. Compared against the
        /// freshly rendered value on each apply; equal ⇒ no write.
        last_value: String,
    },
    /// An iteration over a field's values. Owns its rows and the
    /// comment-node anchor in the DOM that marks where new rows
    /// get inserted before. The plan node's `field` and `body`
    /// stay authoritative during reconciliation; we cache only
    /// what the DOM needs.
    Iteration {
        /// Path to the iteration root in the template. Used to
        /// clone fresh rows; the live DOM doesn't have an
        /// iteration root element anymore (it's been replaced by
        /// the anchor and the rows).
        template_path: Vec<usize>,
        /// Comment node marking the iteration's slot in the live
        /// DOM. Rows insert *before* this node; removed rows
        /// detach. The anchor stays put across applies.
        anchor: Node,
        /// Mounted rows, keyed by the stringified iteration value.
        /// BTreeMap so iteration / DOM order is lexicographic by
        /// key — deterministic regardless of input array order.
        rows: BTreeMap<String, MountedRow>,
    },
}

/// One row inside a [`MountedNode::Iteration`]. Holds the row's
/// live DOM root plus the mounted state of the body bindings
/// inside it.
struct MountedRow {
    /// The cloned iteration-root element in the live DOM. Removed
    /// when the row vanishes.
    root: Node,
    /// Mounted state for the body bindings, paths relative to the
    /// row's `root`.
    body: Vec<MountedNode>,
}

impl Renderer {
    /// Construct a renderer from a pre-snapshotted template +
    /// container pair. Used by `<tonk-view>` after it pulls the
    /// host's children into a `DocumentFragment` at
    /// `connectedCallback`. `snapshot.container` becomes the
    /// renderer's append target.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        // Preprocess on<event>=<concept> attributes into
        // data-on<event>=<concept> before plan extraction. The
        // rewrite is a pure DOM mutation on the template fragment;
        // iteration rows clone the rewritten subtree so every row
        // inherits the data-prefixed form. The plan extractor
        // ignores attributes without `{field}` interpolation, so
        // the rewritten data-on<event> attributes pass through
        // untouched — exactly what the delegation listener needs.
        let event_bindings = preprocess::preprocess(&snapshot.fragment);
        let plan = extract_plan(&snapshot.fragment);
        Self {
            host: snapshot.container,
            template: snapshot.fragment,
            plan,
            event_bindings,
            mounted: None,
        }
    }

    /// Event-handler bindings discovered on the template — the
    /// set of distinct event types and concept names referenced
    /// by `on<event>=<concept>` attributes. The host element
    /// uses these to resolve concept descriptors and install
    /// delegation listeners.
    pub fn event_bindings(&self) -> &Bindings {
        &self.event_bindings
    }

    /// Apply an entity conclusion. First call mounts the template;
    /// subsequent calls reconcile in place — touch only the DOM
    /// nodes whose rendered value changed and add/remove iteration
    /// rows whose key set differs from the previous apply.
    pub fn apply(&mut self, conclusion: &Conclusion) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        if self.mounted.is_none() {
            self.mount_initial(&document, conclusion);
        } else {
            self.update_existing(&document, conclusion);
        }
    }

    /// First-apply path — clone the template fragment, build the
    /// mounted tree from scratch, attach the clone to the host.
    fn mount_initial(&mut self, document: &Document, conclusion: &Conclusion) {
        let Some(fragment) = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
        else {
            return;
        };
        let root: Node = fragment.clone().into();
        let nodes = build_mounted_nodes(
            document,
            &self.plan.nodes,
            &root,
            &self.template,
            &[],
            conclusion,
            &BTreeMap::new(),
        );

        // Tag the first top-level element with `data-this` for
        // CSS / consumer hooks, matching the prior contract.
        let children = fragment.child_nodes();
        for i in 0..children.length() {
            if let Some(n) = children.item(i)
                && let Some(el) = n.dyn_ref::<Element>()
            {
                let _ = el.set_attribute("data-this", &conclusion.this);
                break;
            }
        }

        let _ = self.host.append_child(&fragment);
        self.mounted = Some(MountedScope { root, nodes });
    }

    /// Incremental-update path — walk the existing mounted tree
    /// in lockstep with the plan tree and reconcile against the
    /// new conclusion. No re-clone of the template, no DOM
    /// destruction of nodes whose content hasn't changed.
    fn update_existing(&mut self, document: &Document, conclusion: &Conclusion) {
        let Some(scope) = self.mounted.as_mut() else {
            return;
        };
        update_nodes(
            document,
            &self.plan.nodes,
            &mut scope.nodes,
            &scope.root,
            &self.template,
            conclusion,
            &BTreeMap::new(),
        );

        // `data-this` should still reflect the conclusion. The
        // entity URI rarely changes on a `Renderer` (one entity
        // per view) but if it does, keep the attribute current.
        let children = scope.root.child_nodes();
        for i in 0..children.length() {
            if let Some(n) = children.item(i)
                && let Some(el) = n.dyn_ref::<Element>()
            {
                if el.get_attribute("data-this").as_deref() != Some(&conclusion.this) {
                    let _ = el.set_attribute("data-this", &conclusion.this);
                }
                break;
            }
        }
    }
}

/// Build a fresh `Vec<MountedNode>` from a plan and write its
/// initial values into the DOM. Used for both the top-level
/// mount and each iteration row's body.
///
/// **Ordering caveat**: iteration nodes mutate the live DOM by
/// replacing their iteration-root element with an anchor + rows.
/// Doing that to an earlier sibling shifts subsequent sibling
/// indices, which would invalidate the path-based navigation
/// the *next* plan node performs. To avoid that, we process the
/// plan in two passes:
///
/// 1. **Reverse pass** — process iteration nodes from last to
///    first, since each iteration's mutation only affects later
///    siblings (already processed).
/// 2. **Build in plan order** — assemble the resulting
///    `MountedNode` Vec in the original plan order so it stays
///    in lockstep with the plan tree.
///
/// Leaf bindings don't mutate sibling structure, so they're
/// processed in plan order without complication.
fn build_mounted_nodes(
    document: &Document,
    plan: &[PlanNode],
    scope_root: &Node,
    template: &DocumentFragment,
    template_scope: &[usize],
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) -> Vec<MountedNode> {
    // Allocate the result vec; fill in reverse so iteration
    // nodes process their DOM mutations from rightmost to
    // leftmost. Bindings are order-independent.
    let mut out: Vec<Option<MountedNode>> = (0..plan.len()).map(|_| None).collect();
    for (i, node) in plan.iter().enumerate().rev() {
        out[i] = Some(build_mounted_node(
            document,
            node,
            scope_root,
            template,
            template_scope,
            conclusion,
            shadow,
        ));
    }
    out.into_iter()
        .map(|n| n.expect("every slot filled"))
        .collect()
}

/// Build one mounted node — leaf or iteration — and perform its
/// initial DOM writes / clones.
///
/// `template_scope` is the path inside `template` that corresponds
/// to `scope_root` in the live DOM. Nested iterations use it to
/// reconstruct the absolute template path of their iteration root
/// (their `path` is relative to the *parent scope*, not the
/// template fragment).
fn build_mounted_node(
    document: &Document,
    plan: &PlanNode,
    scope_root: &Node,
    template: &DocumentFragment,
    template_scope: &[usize],
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
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
            // Locate the iteration root in this scope, replace it
            // with a comment anchor, and clone-then-mount one row
            // per value. The original element is gone from the
            // live DOM after this; future clones come from the
            // pristine `template` fragment.
            let anchor: Node = document
                .create_comment(&format!("tonk-iter:{field}"))
                .into();

            let mut rows: BTreeMap<String, MountedRow> = BTreeMap::new();

            // Compose the absolute template path for cloning rows.
            // `path` is relative to the parent scope; pre-pending
            // `template_scope` (the parent's template-absolute
            // path) gives us the location of the iteration root in
            // the original fragment. Nested iterations rely on
            // this — without composition, an inner iter at path
            // `[0]` would re-clone the outer wrapper instead of
            // the inner row template.
            let template_iter_path: Vec<usize> = template_scope
                .iter()
                .copied()
                .chain(path.iter().copied())
                .collect();

            if let Some(iter_root) = navigate(scope_root, path)
                && let Some(parent) = iter_root.parent_node()
            {
                // Drop the in-clone template element; the anchor
                // takes its slot so rows can insert before it.
                let _ = parent.insert_before(&anchor, Some(&iter_root));
                let _: Result<Node, _> = parent.remove_child(&iter_root);

                let raw_value = shadow
                    .get(field)
                    .or_else(|| conclusion.fields.get(field))
                    .cloned();
                let keyed = collect_keyed_values(raw_value, &conclusion.this);

                for (key, value) in keyed {
                    if rows.contains_key(&key) {
                        continue; // dedupe
                    }
                    if let Some(row) = build_iteration_row(
                        document,
                        &template_iter_path,
                        template,
                        body,
                        field,
                        value,
                        conclusion,
                        shadow,
                    ) {
                        // BTreeMap order ⇒ DOM order. Insert each
                        // row before the anchor so they land in
                        // sorted-key order naturally.
                        let _ = parent.insert_before(&row.root, Some(&anchor));
                        rows.insert(key, row);
                    }
                }
            }

            MountedNode::Iteration {
                template_path: template_iter_path,
                anchor,
                rows,
            }
        }
    }
}

/// Walk plan + mounted in lockstep, reconciling each pair.
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

/// Reconcile one plan/mounted pair.
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
        _ => {
            // Plan/mounted shapes diverged — should be impossible
            // since the plan is constant for a Renderer instance.
            // Bail silently rather than panic in production.
        }
    }
}

/// Reconcile one mounted iteration against a new value list.
/// New keys clone + mount; vanished keys detach + drop; surviving
/// keys recurse into their body.
#[allow(clippy::too_many_arguments)]
fn update_iteration(
    document: &Document,
    field: &str,
    template_path: &[usize],
    plan_body: &[PlanNode],
    anchor: &Node,
    rows: &mut BTreeMap<String, MountedRow>,
    template: &DocumentFragment,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
) {
    let raw_value = shadow
        .get(field)
        .or_else(|| conclusion.fields.get(field))
        .cloned();
    let keyed = collect_keyed_values(raw_value, &conclusion.this);

    // Index incoming values by key, deduping.
    let mut incoming: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (key, value) in keyed {
        incoming.entry(key).or_insert(value);
    }

    // Find the anchor's parent — every row sits between
    // siblings of the anchor in the parent.
    let Some(parent) = anchor.parent_node() else {
        return;
    };

    // Single-row rename fallback. When the existing set has
    // exactly one row whose key vanished from `incoming`, and at
    // least one of the incoming keys isn't currently present,
    // reuse the old row under a new key instead of destroying it.
    //
    // This preserves DOM identity (and therefore inner custom-
    // element state, focus, in-flight subscriptions) across the
    // common cardinality transitions that fold_rows produces:
    //
    //   - scalar value edit: `width: 12` → `width: 16`. The same
    //     row is reused, only its bound attribute is patched.
    //   - scalar → array growth: `column: "a"` → `column: ["a","b"]`.
    //     The folded representation flips from a string to an
    //     array, but the row for "a" should survive — the new "b"
    //     row is added alongside, the existing column is not
    //     torn down.
    //   - array → scalar shrink (cardinality 1 → 1 with a swap):
    //     `column: ["a"]` → `column: "b"`. The single row is
    //     reused under the new key.
    //
    // For "true" array reconciliation (multiple rows on both
    // sides) the rename never fires — keys match exactly.
    if rows.len() == 1 {
        let old_key = rows.keys().next().cloned().expect("len == 1");
        if !incoming.contains_key(&old_key)
            && let Some((new_key, _)) = incoming
                .iter()
                .find(|(k, _)| !rows.contains_key(*k))
                .map(|(k, v)| (k.clone(), v.clone()))
        {
            if let Some(row) = rows.remove(&old_key) {
                rows.insert(new_key, row);
            }
        }
    }

    // Remove vanished keys.
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

    // Walk incoming in sorted-key order. For each key:
    //   - Reuse existing row → recurse into body.
    //   - Add new row → clone from template, mount, insert at
    //     the correct sorted position.
    for (key, value) in incoming {
        if let Some(row) = rows.get_mut(&key) {
            // Existing row: update its body bindings in place
            // with shadow extended by this iteration's value.
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
        } else {
            // New row: clone + build (detached), then insert
            // once at the correct sorted position. Single
            // insertBefore avoids the move-induced custom-element
            // lifecycle re-fire (see `build_iteration_row`'s
            // doc-comment).
            if let Some(new_row) = build_iteration_row(
                document,
                template_path,
                template,
                plan_body,
                field,
                value,
                conclusion,
                shadow,
            ) {
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
}

/// Clone the iteration root from the pristine template, build
/// its body's mounted state with all bindings applied. The
/// returned [`MountedRow`] is **detached** — the caller decides
/// where to insert it via a single `parent.insertBefore` call.
///
/// We deliberately don't insert here. Inserting now and then
/// again at the sorted position would amount to a *move*, which
/// the DOM spec implements as detach + reattach; that fires
/// `disconnected_callback` and `connected_callback` on every
/// custom element inside the row, causing inner `<tonk-display>`
/// instances to spin up *twice* and mount two slides.
///
/// Bindings are written against the detached clone before any
/// element is connected to the DOM, so custom-element lifecycle
/// callbacks don't fire during `build_mounted_nodes`. By the
/// time the caller inserts, every observed attribute is already
/// at its final value and `connected_callback` runs exactly
/// once.
///
/// Returns `None` if cloning fails (degenerate template).
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
) -> Option<MountedRow> {
    let template_root: Node = template.clone().into();
    let template_iter_root = navigate(&template_root, template_path)?;
    let row_root = template_iter_root.clone_node_with_deep(true).ok()?;

    let mut nested_shadow = shadow.clone();
    nested_shadow.insert(field.to_owned(), value);
    // The row's template scope is `template_path` — anything
    // nested inside (e.g. a child iteration's `path`) is relative
    // to it. Threading this lets nested iterations recover the
    // absolute template path needed for cloning.
    let body = build_mounted_nodes(
        document,
        body_plan,
        &row_root,
        template,
        template_path,
        conclusion,
        &nested_shadow,
    );

    Some(MountedRow {
        root: row_root,
        body,
    })
}

/// Stable string key for an iteration value. Entity URIs (and
/// any other JSON string) become themselves; non-strings get
/// canonicalised via `serde_json::to_string` so two equal values
/// produce equal keys.
fn key_for(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Render a binding's segments against the conclusion + shadow,
/// returning the substituted string.
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

/// Write a binding's rendered value to the target DOM node
/// identified by its path within `scope_root`. Attribute-form
/// bindings flow through [`apply_attribute_binding`] which decides
/// between property and attribute assignment per value type.
fn write_binding(
    scope_root: &Node,
    binding: &Binding,
    rendered: &str,
    conclusion: &Conclusion,
    shadow: &BTreeMap<String, serde_json::Value>,
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

/// Resolve a JSON value into the list of (row-key, row-value)
/// pairs the iteration renderer needs.
///
/// - `Null` / missing → no rows.
/// - `Array` → one row per element, keyed by `key_for(value)`
///   so cardinality-many reconciliation matches rows across
///   applies by element identity.
/// - Anything else (a scalar) → one row keyed by the enclosing
///   conclusion's `this` (entity URI). This identifies the row
///   by the *entity it describes*, not by the placeholder's
///   current value, so editing a cardinality-one field (e.g. a
///   column's width) reuses the same row and patches only the
///   bound attribute. The `entity_key` is supplied by the
///   caller because it carries the relevant scope's `this` —
///   for nested iterations that may differ from the outer
///   conclusion's `this` once `subject=` markers identify an
///   inner entity.
fn collect_keyed_values(
    value: Option<serde_json::Value>,
    entity_key: &str,
) -> Vec<(String, serde_json::Value)> {
    match value {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => {
            items.into_iter().map(|v| (key_for(&v), v)).collect()
        }
        Some(v) => vec![(entity_key.to_owned(), v)],
    }
}
