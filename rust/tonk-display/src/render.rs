//! DOM renderer for a `<tonk-view>` frame. A frame is a list of
//! folded conclusions — one per subject — and the renderer maintains a
//! mounted-state tree so repeated `apply` calls update the DOM in place
//! rather than re-cloning the template every frame.
//!
//! The template is split (by [`crate::template::split_plan`])
//! into two halves:
//!
//! - **chrome** — bindings outside the per-conclusion repeat element.
//!   Rendered once against the lead conclusion (e.g. a sheet title
//!   surrounding a repeated row).
//! - **repeat** — the element cloned once per conclusion. The renderer
//!   keys rows by the conclusion's `this`, clones the repeat element
//!   (or, when no single element encloses the references, the whole
//!   fragment), renders the repeat body against that conclusion, and
//!   stamps `with=<this>` on the clone so the repeat boundary is
//!   inspectable.
//!
//! Inside a repeat row, cardinality-many *subject fields* still iterate
//! their values via [`MountedNode::Iteration`] — `{this}` chooses the
//! repeat element, a many-valued `{tags}` repeats a subtree within one
//! conclusion.
//!
//! Three update primitives:
//!
//! - **MountedBinding** caches the last rendered string and skips the
//!   write when the value is unchanged, preserving node identity.
//! - **MountedIteration** keys rows by the value's string form, cloning
//!   new keys and detaching vanished ones.
//! - **MountedRepeat** keys rows by conclusion `this`, the same way, so
//!   adding/removing a subject touches only its row.
//!
//! Rows build **detached** and insert with a single `insertBefore`, so
//! custom elements inside a row mount exactly once (a move would fire
//! `disconnected`/`connected` and double-mount nested `<tonk-display>`).

use std::collections::BTreeMap;

use crate::template::{
    Binding, BindingKind, BindingPlan, PlanNode, RepeatPlan, Snapshot, apply_attribute_binding,
    extract_plan, navigate, render_segments_with_shadow, single_field_value,
};
use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use web_sys::{Document, DocumentFragment, Element, Node, window};

use crate::events::preprocess::{self, Bindings};

/// Stateful renderer for one `<tonk-view>`. Holds the template, the
/// split plan, and (after the first apply) the mounted-state tree.
pub struct Renderer {
    /// Where the rendered fragment lives.
    host: Element,
    /// Cloneable template fragment captured at construction time. Used
    /// for the initial mount and for cloning repeat rows / iteration
    /// subtrees on the fly.
    template: DocumentFragment,
    /// Split binding plan extracted from `template` at construction.
    plan: BindingPlan,
    /// Event-handler bindings discovered on the template by the
    /// preprocess pass.
    event_bindings: Bindings,
    /// Mounted state. `None` before the first apply; `Some` after.
    /// Dropped on detach; the next apply rebuilds from scratch.
    mounted: Option<MountedScope>,
}

/// Top-level mounted state — one per `Renderer`. Holds the live root
/// cloned from the template, the chrome nodes (rendered once), and the
/// per-conclusion repeat.
struct MountedScope {
    /// Live root in the DOM (the cloned fragment, held for navigate()).
    root: Node,
    /// Mirror of `plan.chrome`, rendered once against the lead
    /// conclusion.
    chrome: Vec<MountedNode>,
    /// The per-conclusion repeat rows.
    repeat: MountedRepeat,
}

/// The per-conclusion repeat: a keyed set of rows, one per subject,
/// plus the anchor marking their slot in the DOM.
struct MountedRepeat {
    /// Comment node marking where rows insert. Rows sit before it; it
    /// stays put across applies.
    anchor: Node,
    /// Rows keyed by the conclusion's `this`. BTreeMap so DOM order is
    /// deterministic (lexicographic by subject URI).
    rows: BTreeMap<String, MountedRow>,
}

/// One mounted plan-tree node. Mirrors [`PlanNode`] but carries only
/// the DOM bookkeeping needed to update in place — the plan node stays
/// authoritative for path / kind / segments.
enum MountedNode {
    /// A leaf binding with its last-rendered string cached.
    Binding {
        /// Most recent rendered string; equal ⇒ no write.
        last_value: String,
    },
    /// An iteration over a subject field's values (cardinality-many
    /// within one conclusion).
    Iteration {
        /// Absolute template path to the iteration root, for cloning.
        template_path: Vec<usize>,
        /// Comment anchor marking the iteration's slot.
        anchor: Node,
        /// Rows keyed by the stringified iteration value.
        rows: BTreeMap<String, MountedRow>,
    },
}

/// One row inside a [`MountedRepeat`] or [`MountedNode::Iteration`].
struct MountedRow {
    /// The cloned row root in the live DOM. Removed when the row
    /// vanishes.
    root: Node,
    /// Mounted state for the body, paths relative to `root`.
    body: Vec<MountedNode>,
}

/// The conclusion a scope falls back to when the frame is empty.
fn empty_conclusion() -> Conclusion {
    Conclusion {
        this: String::new(),
        fields: BTreeMap::new(),
    }
}

impl Renderer {
    /// Construct a renderer from a snapshotted template + container.
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        // Preprocess on<event>=<concept> into data-on<event> before
        // plan extraction; the rewrite is a pure DOM mutation that
        // iteration rows inherit via cloning.
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

    /// Event-handler bindings discovered on the template.
    pub fn event_bindings(&self) -> &Bindings {
        &self.event_bindings
    }

    /// Apply an entity frame. First call mounts; subsequent calls
    /// reconcile in place — touch only the nodes whose rendered value
    /// changed and add/remove repeat rows whose subject set differs.
    pub fn apply(&mut self, frame: &[Conclusion]) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        if self.mounted.is_none() {
            self.mount_initial(&document, frame);
        } else {
            self.update_existing(&document, frame);
        }
    }

    /// First-apply path: clone the template, render chrome once, mount
    /// one repeat row per conclusion, attach to the host.
    fn mount_initial(&mut self, document: &Document, frame: &[Conclusion]) {
        let Some(fragment) = self
            .template
            .clone_node_with_deep(true)
            .ok()
            .and_then(|n| n.dyn_into::<DocumentFragment>().ok())
        else {
            return;
        };
        let root: Node = fragment.clone().into();
        let lead = frame.first().cloned().unwrap_or_else(empty_conclusion);

        // Chrome renders once against the lead conclusion. No shadow.
        let chrome = build_mounted_nodes(
            document,
            &self.plan.chrome,
            &root,
            &self.template,
            &[],
            &lead,
            &BTreeMap::new(),
        );

        let repeat = mount_repeat(document, &self.plan.repeat, &root, &self.template, frame);

        let _ = self.host.append_child(&fragment);
        // After `append_child` the fragment is emptied — its children now
        // live under `host`. `navigate()` on a later update must walk the
        // live tree, so the stored navigation root is the host, not the
        // now-empty fragment. The fragment's top-level children appended
        // in order onto an empty host, so a path relative to the fragment
        // root is equally valid relative to the host. (Initial chrome /
        // repeat above ran against the still-populated fragment, which is
        // why they mount correctly; only the update path read the emptied
        // fragment and silently found no node — dropping every chrome
        // update, e.g. `<tonk-sheet-binder active={dom.host/data-active}>`
        // never reflecting a host-attribute change.)
        self.mounted = Some(MountedScope {
            root: self.host.clone().into(),
            chrome,
            repeat,
        });
    }

    /// Incremental-update path: reconcile chrome against the lead
    /// conclusion and the repeat rows against the new frame.
    fn update_existing(&mut self, document: &Document, frame: &[Conclusion]) {
        let Some(scope) = self.mounted.as_mut() else {
            return;
        };
        let lead = frame.first().cloned().unwrap_or_else(empty_conclusion);

        update_nodes(
            document,
            &self.plan.chrome,
            &mut scope.chrome,
            &scope.root,
            &self.template,
            &lead,
            &BTreeMap::new(),
        );

        update_repeat(
            document,
            &self.plan.repeat,
            &mut scope.repeat,
            &scope.root,
            &self.template,
            frame,
        );
    }
}

/// Build the per-conclusion repeat at mount: replace the repeat element
/// with an anchor and clone one keyed row per conclusion.
///
/// When `plan.path` is `Some`, the repeat element is located and
/// replaced. When `None`, the whole fragment repeats: the anchor is
/// appended at the fragment's end and each row clones every top-level
/// node of the fragment.
fn mount_repeat(
    document: &Document,
    plan: &RepeatPlan,
    root: &Node,
    template: &DocumentFragment,
    frame: &[Conclusion],
) -> MountedRepeat {
    let anchor: Node = document.create_comment("tonk-repeat").into();
    let mut rows: BTreeMap<String, MountedRow> = BTreeMap::new();

    match &plan.path {
        Some(path) => {
            // Locate the repeat element, drop it from the live clone,
            // put the anchor in its slot, and clone rows from the
            // pristine template element at `path`.
            if let Some(repeat_el) = navigate(root, path)
                && let Some(parent) = repeat_el.parent_node()
            {
                let _ = parent.insert_before(&anchor, Some(&repeat_el));
                let _: Result<Node, _> = parent.remove_child(&repeat_el);
                for member in frame {
                    let key = member.this.clone();
                    if rows.contains_key(&key) {
                        continue;
                    }
                    if let Some(row) = build_repeat_row(document, plan, template, member) {
                        let _ = parent.insert_before(&row.root, Some(&anchor));
                        rows.insert(key, row);
                    }
                }
            }
        }
        None => {
            // Whole-fragment repeat — a multi-root fragment with no
            // single enclosing element (e.g. `<h1>{a}</h1><p>{b}</p>`).
            // There is no element to clone-per-conclusion or stamp
            // `with=` on, so the lead conclusion renders once over the
            // fragment's own nodes (matching the historical
            // clone-whole-fragment contract). Single-root templates
            // never reach here: `this_repeat_root` returns the root
            // element path instead.
            let lead = frame.first().cloned().unwrap_or_else(empty_conclusion);
            let body = build_mounted_nodes(
                document,
                &plan.body,
                root,
                template,
                &[],
                &lead,
                &BTreeMap::new(),
            );
            if let Some(first) = frame.first() {
                rows.insert(
                    first.this.clone(),
                    MountedRow {
                        root: root.clone(),
                        body,
                    },
                );
            }
        }
    }

    MountedRepeat { anchor, rows }
}

/// Reconcile the repeat rows against a new frame: new subjects clone +
/// mount, vanished subjects detach, surviving subjects recurse into
/// their body.
fn update_repeat(
    document: &Document,
    plan: &RepeatPlan,
    mounted: &mut MountedRepeat,
    scope_root: &Node,
    template: &DocumentFragment,
    frame: &[Conclusion],
) {
    let Some(parent) = mounted.anchor.parent_node() else {
        // No anchor in the DOM — the whole-fragment (`None`) path, which
        // tracks a single lead row over the host's own nodes.
        let Some(lead) = frame.first() else {
            // Frame went empty: clear the tracked row's bound nodes so a
            // stale value doesn't linger, but keep the row entry — the
            // host nodes stay mounted to receive the next frame.
            if let Some((_, row)) = mounted.rows.iter_mut().next() {
                update_nodes(
                    document,
                    &plan.body,
                    &mut row.body,
                    &row.root,
                    template,
                    &empty_conclusion(),
                    &BTreeMap::new(),
                );
            }
            return;
        };
        match mounted.rows.iter_mut().next() {
            // Already tracking a row — reconcile it against the lead.
            Some((_, row)) => {
                update_nodes(
                    document,
                    &plan.body,
                    &mut row.body,
                    &row.root,
                    template,
                    lead,
                    &BTreeMap::new(),
                );
            }
            // First non-empty frame after an empty one: no row was
            // mounted yet (the initial frame had no lead), so build the
            // body bindings over the host's nodes now and record the row.
            None => {
                let body = build_mounted_nodes(
                    document,
                    &plan.body,
                    scope_root,
                    template,
                    &[],
                    lead,
                    &BTreeMap::new(),
                );
                mounted.rows.insert(
                    lead.this.clone(),
                    MountedRow {
                        root: scope_root.clone(),
                        body,
                    },
                );
            }
        }
        return;
    };

    // Incoming subjects, keyed and deduped, in frame order.
    let mut incoming: BTreeMap<String, Conclusion> = BTreeMap::new();
    for member in frame {
        incoming
            .entry(member.this.clone())
            .or_insert_with(|| member.clone());
    }

    // Single-row rename: when exactly one row exists whose subject
    // vanished and a fresh subject appears, reuse the row (preserves
    // inner custom-element state across a single-entity swap).
    if mounted.rows.len() == 1 {
        let old_key = mounted.rows.keys().next().cloned().expect("len == 1");
        if !incoming.contains_key(&old_key)
            && let Some(new_key) = incoming
                .keys()
                .find(|k| !mounted.rows.contains_key(*k))
                .cloned()
            && let Some(row) = mounted.rows.remove(&old_key)
        {
            mounted.rows.insert(new_key, row);
        }
    }

    // Remove vanished subjects.
    let stale: Vec<String> = mounted
        .rows
        .keys()
        .filter(|k| !incoming.contains_key(*k))
        .cloned()
        .collect();
    for key in stale {
        if let Some(row) = mounted.rows.remove(&key) {
            let _: Result<Node, _> = parent.remove_child(&row.root);
        }
    }

    // Walk incoming in sorted-key order: reuse + recurse, or clone +
    // insert at the sorted position.
    for (key, member) in incoming {
        if let Some(row) = mounted.rows.get_mut(&key) {
            update_nodes(
                document,
                &plan.body,
                &mut row.body,
                &row.root,
                template,
                &member,
                &BTreeMap::new(),
            );
            stamp_with(&row.root, &member.this);
        } else if let Some(new_row) = build_repeat_row(document, plan, template, &member) {
            let next_anchor = mounted
                .rows
                .range(key.clone()..)
                .next()
                .map(|(_, row)| row.root.clone())
                .unwrap_or_else(|| mounted.anchor.clone());
            let _ = parent.insert_before(&new_row.root, Some(&next_anchor));
            mounted.rows.insert(key, new_row);
        }
    }
}

/// Clone one repeat row from the pristine template and render its body
/// against `member`. The returned row is **detached**; the caller
/// inserts it with a single `insertBefore`. Stamps `with=<this>` on the
/// clone.
///
/// `plan.path` selects what to clone: a specific element, or (for a
/// whole-fragment repeat) the fragment's nodes wrapped so the body
/// paths still resolve.
fn build_repeat_row(
    document: &Document,
    plan: &RepeatPlan,
    template: &DocumentFragment,
    member: &Conclusion,
) -> Option<MountedRow> {
    let template_root: Node = template.clone().into();

    let (row_root, body_scope): (Node, Vec<usize>) = match &plan.path {
        Some(path) => {
            let template_el = navigate(&template_root, path)?;
            let clone = template_el.clone_node_with_deep(true).ok()?;
            (clone, path.clone())
        }
        None => {
            // Whole-fragment repeat: clone the fragment's single root
            // element. A multi-root fragment isn't a supported repeat
            // unit (no single element to stamp / key); fall back to the
            // first element child.
            let clone = template_root.clone_node_with_deep(true).ok()?;
            (clone, Vec::new())
        }
    };

    let body = build_mounted_nodes(
        document,
        &plan.body,
        &row_root,
        template,
        &body_scope,
        member,
        &BTreeMap::new(),
    );

    stamp_with(&row_root, &member.this);

    Some(MountedRow {
        root: row_root,
        body,
    })
}

/// Stamp `with=<this>` on a repeat row's root element so the repeat
/// boundary is inspectable. No-op when the root isn't an element (a
/// whole-fragment clone yielding a fragment).
fn stamp_with(root: &Node, this: &str) {
    if let Some(el) = root.dyn_ref::<Element>() {
        let _ = el.set_attribute("with", this);
    }
}

/// Build a `Vec<MountedNode>` from a plan and write its initial values
/// into the DOM. Used for chrome, repeat bodies, and iteration rows.
///
/// **Ordering caveat**: iteration nodes mutate the live DOM (replace
/// their root with an anchor + rows), shifting later sibling indices.
/// Process iterations last-to-first so each mutation only affects
/// already-processed siblings; assemble the result in plan order to
/// stay in lockstep with the plan.
fn build_mounted_nodes(
    document: &Document,
    plan: &[PlanNode],
    scope_root: &Node,
    template: &DocumentFragment,
    template_scope: &[usize],
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> Vec<MountedNode> {
    let mut out: Vec<Option<MountedNode>> = (0..plan.len()).map(|_| None).collect();
    for (i, node) in plan.iter().enumerate().rev() {
        out[i] = Some(build_mounted_node(
            document,
            node,
            scope_root,
            template,
            template_scope,
            member,
            shadow,
        ));
    }
    out.into_iter()
        .map(|n| n.expect("every slot filled"))
        .collect()
}

/// Build one mounted node — leaf or iteration — and perform its initial
/// DOM writes / clones.
///
/// `template_scope` is the path inside `template` corresponding to
/// `scope_root` in the live DOM. Nested iterations use it to
/// reconstruct the absolute template path of their iteration root.
fn build_mounted_node(
    document: &Document,
    plan: &PlanNode,
    scope_root: &Node,
    template: &DocumentFragment,
    template_scope: &[usize],
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> MountedNode {
    match plan {
        PlanNode::Binding(b) => {
            let rendered = render_binding(b, member, shadow);
            write_binding(scope_root, b, &rendered, member, shadow);
            MountedNode::Binding {
                last_value: rendered,
            }
        }
        PlanNode::Iteration { field, path, body } => {
            let anchor: Node = document
                .create_comment(&format!("tonk-iter:{field}"))
                .into();
            let mut rows: BTreeMap<String, MountedRow> = BTreeMap::new();

            let template_iter_path: Vec<usize> = template_scope
                .iter()
                .copied()
                .chain(path.iter().copied())
                .collect();

            if let Some(iter_root) = navigate(scope_root, path)
                && let Some(parent) = iter_root.parent_node()
            {
                let _ = parent.insert_before(&anchor, Some(&iter_root));
                let _: Result<Node, _> = parent.remove_child(&iter_root);

                let raw_value = shadow
                    .get(field)
                    .or_else(|| member.fields.get(field))
                    .cloned();
                let keyed = collect_keyed_values(raw_value, &member.this);

                for (key, value) in keyed {
                    if rows.contains_key(&key) {
                        continue;
                    }
                    if let Some(row) = build_iteration_row(
                        document,
                        &template_iter_path,
                        template,
                        body,
                        (field, value),
                        member,
                        shadow,
                    ) {
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
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    for (plan_node, mounted_node) in plan.iter().zip(mounted.iter_mut()) {
        update_node(
            document,
            plan_node,
            mounted_node,
            scope_root,
            template,
            member,
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
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    match (plan, mounted) {
        (PlanNode::Binding(b), MountedNode::Binding { last_value }) => {
            let rendered = render_binding(b, member, shadow);
            if *last_value != rendered {
                write_binding(scope_root, b, &rendered, member, shadow);
                *last_value = rendered;
            }
        }
        (
            PlanNode::Iteration {
                field,
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
                field,
                template_path,
                body,
                anchor,
                rows,
                template,
                member,
                shadow,
            );
        }
        _ => {
            // Plan/mounted shapes diverged — impossible for a constant
            // plan. Bail silently rather than panic.
        }
    }
}

/// Reconcile one mounted subject-field iteration against a new value
/// list. New keys clone + mount; vanished keys detach; survivors
/// recurse into their body with `field` shadowed.
#[allow(clippy::too_many_arguments)]
fn update_iteration(
    document: &Document,
    field: &str,
    template_path: &[usize],
    plan_body: &[PlanNode],
    anchor: &Node,
    rows: &mut BTreeMap<String, MountedRow>,
    template: &DocumentFragment,
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    let raw_value = shadow
        .get(field)
        .or_else(|| member.fields.get(field))
        .cloned();
    let mut incoming: BTreeMap<String, Ipld> = BTreeMap::new();
    for (key, value) in collect_keyed_values(raw_value, &member.this) {
        incoming.entry(key).or_insert(value);
    }

    let Some(parent) = anchor.parent_node() else {
        return;
    };

    // Single-row rename fallback (scalar edit / scalar↔array swaps).
    if rows.len() == 1 {
        let old_key = rows.keys().next().cloned().expect("len == 1");
        if !incoming.contains_key(&old_key)
            && let Some(new_key) = incoming.keys().find(|k| !rows.contains_key(*k)).cloned()
            && let Some(row) = rows.remove(&old_key)
        {
            rows.insert(new_key, row);
        }
    }

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
            nested_shadow.insert(field.to_owned(), value.clone());
            update_nodes(
                document,
                plan_body,
                &mut row.body,
                &row.root,
                template,
                member,
                &nested_shadow,
            );
        } else if let Some(new_row) = build_iteration_row(
            document,
            template_path,
            template,
            plan_body,
            (field, value),
            member,
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

/// Clone a subject-field iteration root from the pristine template and
/// build its body with `field` shadowed to `value`. Detached; caller
/// inserts.
#[allow(clippy::too_many_arguments)]
fn build_iteration_row(
    document: &Document,
    template_path: &[usize],
    template: &DocumentFragment,
    body_plan: &[PlanNode],
    shadow_entry: (&str, Ipld),
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> Option<MountedRow> {
    let template_root: Node = template.clone().into();
    let template_iter_root = navigate(&template_root, template_path)?;
    let row_root = template_iter_root.clone_node_with_deep(true).ok()?;

    let mut nested_shadow = shadow.clone();
    let (field, value) = shadow_entry;
    nested_shadow.insert(field.to_owned(), value);

    let body = build_mounted_nodes(
        document,
        body_plan,
        &row_root,
        template,
        template_path,
        member,
        &nested_shadow,
    );

    Some(MountedRow {
        root: row_root,
        body,
    })
}

/// Stable string key for an iteration value.
fn key_for(value: &Ipld) -> String {
    match value {
        Ipld::String(s) => s.clone(),
        other => serde_ipld_dagjson::to_vec(other)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default(),
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

/// Write a binding's rendered value to the target DOM node.
fn write_binding(
    scope_root: &Node,
    binding: &Binding,
    rendered: &str,
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) {
    match &binding.kind {
        BindingKind::Text { .. } => {
            if let Some(target) = navigate(scope_root, &binding.path) {
                target.set_text_content(Some(rendered));
            }
        }
        BindingKind::Attribute { .. } => {
            let value = single_field_value(binding, &member.this, &member.fields, shadow);
            apply_attribute_binding(scope_root, binding, rendered, value.as_ref());
        }
    }
}

/// Resolve a value into the list of (row-key, row-value) pairs the
/// iteration renderer needs.
///
/// - `Null` / missing → no rows.
/// - `Array` → one row per element, keyed by `key_for`.
/// - scalar → one row keyed by the conclusion's `this`, so editing a
///   cardinality-one field reuses the same row.
fn collect_keyed_values(value: Option<Ipld>, entity_key: &str) -> Vec<(String, Ipld)> {
    match value {
        None | Some(Ipld::Null) => Vec::new(),
        Some(Ipld::List(items)) => items.into_iter().map(|v| (key_for(&v), v)).collect(),
        Some(v) => vec![(entity_key.to_owned(), v)],
    }
}
