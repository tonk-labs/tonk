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
//!   stamps `data-this=<this>` on the clone so the repeat boundary is
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
    extract_plan_with_scalars, navigate, render_segments_with_shadow, single_field_value,
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
    /// Construct a renderer from a snapshotted template + container, planning
    /// with `scalar_fields` — the model concept's `cardinality: one` field
    /// names. A scalar field used in a template is rendered as a plain
    /// substitution rather than an iteration root that vanishes when the value
    /// is absent. Pass an empty set for the value-driven default (no descriptor).
    pub fn from_snapshot_with_scalars(
        snapshot: Snapshot,
        scalar_fields: &std::collections::BTreeSet<String>,
    ) -> Self {
        // Preprocess on<event>=<concept> into data-on<event> before
        // plan extraction; the rewrite is a pure DOM mutation that
        // iteration rows inherit via cloning.
        let event_bindings = preprocess::preprocess(&snapshot.fragment);
        let plan = extract_plan_with_scalars(&snapshot.fragment, scalar_fields);
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

    /// The host attributes this template consumes via `{dom.host/<attr>}`
    /// — the render inputs the owning display must watch for changes so a
    /// restamped attribute (e.g. the FAB's `data-space` on a space switch)
    /// re-applies through the binding diff.
    pub fn host_attributes(&self) -> std::collections::BTreeSet<String> {
        tonk_template::host_attributes(&self.plan)
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
        //
        // The whole-fragment repeat (`plan.repeat.path == None`) recorded the
        // fragment as its ROW root for the same navigation purpose — re-point
        // it at the host too, or the row's binding updates walk the emptied
        // fragment and silently drop: the space chrome's nested
        // `<tonk-site with="main@{id}">` never restamped on navigation, so a
        // space→space route change left the previous space mounted.
        let mut repeat = repeat;
        if self.plan.repeat.path.is_none() {
            let host_root: Node = self.host.clone().into();
            for row in repeat.rows.values_mut() {
                row.root = host_root.clone();
            }
        }
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
                // All rows land before the same anchor in frame order, so
                // stage them in a fragment and insert the whole run with one
                // `insertBefore` instead of one reflow per row.
                let batch = document.create_document_fragment();
                for member in frame {
                    let key = member.this.clone();
                    if rows.contains_key(&key) {
                        continue;
                    }
                    if let Some(row) = build_repeat_row(document, plan, template, member) {
                        let _ = batch.append_child(&row.root);
                        rows.insert(key, row);
                    }
                }
                let _ = parent.insert_before(&batch, Some(&anchor));
            }
        }
        None => {
            // Whole-fragment repeat — a multi-root fragment with no
            // single enclosing element (e.g. `<h1>{a}</h1><p>{b}</p>`).
            // There is no element to clone-per-conclusion or stamp
            // `data-this=` on, so the lead conclusion renders once over the
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

    // Incoming subjects, keyed and deduped, in frame order. A subjectless
    // conclusion (empty `this`) is the synthetic host-attribute lead of an
    // empty directory frame, never a real instance: keying it here would
    // let the single-row rename below adopt the last surviving row under
    // the empty key and blank it in place instead of removing it.
    let mut incoming: BTreeMap<String, Conclusion> = BTreeMap::new();
    for member in frame {
        if member.this.is_empty() {
            continue;
        }
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
    // insert at the sorted position. New rows landing before the same
    // successor form an adjacent run; stage each run in a fragment and
    // insert it with one `insertBefore` instead of one per row. The
    // successor of a new key is an *existing* row (or the trailing
    // anchor), never one of this run's own rows, since we walk ascending
    // and a row already inserted into `mounted.rows` keeps its DOM slot —
    // so the batched roots stay detached until the run flushes.
    let mut batch: Option<(Node, DocumentFragment)> = None;
    let flush = |parent: &Node, batch: &mut Option<(Node, DocumentFragment)>| {
        if let Some((anchor, fragment)) = batch.take() {
            let _ = parent.insert_before(&fragment, Some(&anchor));
        }
    };
    for (key, member) in incoming {
        if let Some(row) = mounted.rows.get_mut(&key) {
            flush(&parent, &mut batch);
            update_nodes(
                document,
                &plan.body,
                &mut row.body,
                &row.root,
                template,
                &member,
                &BTreeMap::new(),
            );
            stamp_this(&row.root, &member.this);
        } else if let Some(new_row) = build_repeat_row(document, plan, template, &member) {
            let next_anchor = mounted
                .rows
                .range(key.clone()..)
                .next()
                .map(|(_, row)| row.root.clone())
                .unwrap_or_else(|| mounted.anchor.clone());
            // Flush the open run if this row targets a different anchor.
            if batch.as_ref().is_some_and(|(a, _)| a != &next_anchor) {
                flush(&parent, &mut batch);
            }
            let (_, fragment) =
                batch.get_or_insert_with(|| (next_anchor, document.create_document_fragment()));
            let _ = fragment.append_child(&new_row.root);
            mounted.rows.insert(key, new_row);
        }
    }
    flush(&parent, &mut batch);
}

/// Clone one repeat row from the pristine template and render its body
/// against `member`. The returned row is **detached**; the caller
/// inserts it with a single `insertBefore`. Stamps `data-this=<this>` on the
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
    // A subjectless conclusion (empty `this`) is the synthetic host-attribute
    // lead injected to feed chrome on an empty directory frame (e.g. the FAB's
    // {dom.host/data-space} with zero instances), never a real instance — it
    // must not clone a repeat row.
    if member.this.is_empty() {
        return None;
    }
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

    stamp_this(&row_root, &member.this);

    Some(MountedRow {
        root: row_root,
        body,
    })
}

/// Stamp `data-this=<this>` on a repeat row's root element so the repeat
/// boundary is inspectable. (Not `with=` — that is the routing-context
/// attribute, and a row subject is usually not a repository.) No-op when
/// the root isn't an element (a whole-fragment clone yielding a fragment).
fn stamp_this(root: &Node, this: &str) {
    if let Some(el) = root.dyn_ref::<Element>() {
        let _ = el.set_attribute("data-this", this);
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

                // Same anchor for every row, so stage the run in a fragment
                // and insert it once rather than reflowing per value.
                let batch = document.create_document_fragment();
                for (key, value) in keyed {
                    if rows.contains_key(&key) {
                        continue;
                    }
                    if let Some(row) = build_iteration_row(
                        document,
                        &template_iter_path,
                        template,
                        body,
                        (field, &key, value),
                        member,
                        shadow,
                    ) {
                        let _ = batch.append_child(&row.root);
                        rows.insert(key, row);
                    }
                }
                let _ = parent.insert_before(&batch, Some(&anchor));
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

    // Adjacent new rows sharing a successor flush as one fragment; see the
    // matching note in `update_repeat`.
    let mut batch: Option<(Node, DocumentFragment)> = None;
    let flush = |parent: &Node, batch: &mut Option<(Node, DocumentFragment)>| {
        if let Some((anchor, fragment)) = batch.take() {
            let _ = parent.insert_before(&fragment, Some(&anchor));
        }
    };
    for (key, value) in incoming {
        if let Some(row) = rows.get_mut(&key) {
            flush(&parent, &mut batch);
            let mut nested_shadow = shadow.clone();
            nested_shadow.insert(field.to_owned(), value.clone());
            nested_shadow.insert(row_key_field(field), Ipld::String(key.clone()));
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
            (field, &key, value),
            member,
            shadow,
        ) {
            let next_anchor = rows
                .range(key.clone()..)
                .next()
                .map(|(_, row)| row.root.clone())
                .unwrap_or_else(|| anchor.clone());
            if batch.as_ref().is_some_and(|(a, _)| a != &next_anchor) {
                flush(&parent, &mut batch);
            }
            let (_, fragment) =
                batch.get_or_insert_with(|| (next_anchor, document.create_document_fragment()));
            let _ = fragment.append_child(&new_row.root);
            rows.insert(key, new_row);
        }
    }
    flush(&parent, &mut batch);
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
    shadow_entry: (&str, &str, Ipld),
    member: &Conclusion,
    shadow: &BTreeMap<String, Ipld>,
) -> Option<MountedRow> {
    let template_root: Node = template.clone().into();
    let template_iter_root = navigate(&template_root, template_path)?;
    let row_root = template_iter_root.clone_node_with_deep(true).ok()?;

    let mut nested_shadow = shadow.clone();
    let (field, key, value) = shadow_entry;
    nested_shadow.insert(field.to_owned(), value);
    nested_shadow.insert(row_key_field(field), Ipld::String(key.to_owned()));

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

/// Whether any `{field}` segment of `binding` references a field missing
/// from both `shadow` and `row_fields` (i.e. genuinely absent, not
/// present-but-empty). `{this}` and `{text}` segments never count as absent.
/// Used to hold off writing a partially-resolved multi-segment attribute
/// binding (`with="main@{id}"`) until its field lands — see
/// [`apply_attribute_binding`].
fn binding_has_absent_field(
    binding: &Binding,
    row_fields: &BTreeMap<String, Ipld>,
    shadow: &BTreeMap<String, Ipld>,
) -> bool {
    use crate::template::Segment;
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    segments.iter().any(|seg| match seg {
        Segment::Field(name) => {
            name != "this" && shadow.get(name).is_none() && row_fields.get(name).is_none()
        }
        Segment::Text(_) => false,
    })
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
            } else {
                web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "tonk-render: text binding target missing at {:?}",
                    binding.path
                )));
            }
        }
        BindingKind::Attribute { .. } => {
            if navigate(scope_root, &binding.path).is_none() {
                web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "tonk-render: attribute binding target missing at {:?}",
                    binding.path
                )));
            }
            let value = single_field_value(binding, &member.this, &member.fields, shadow);
            let has_absent_field = binding_has_absent_field(binding, &member.fields, shadow);
            apply_attribute_binding(
                scope_root,
                binding,
                rendered,
                value.as_ref(),
                has_absent_field,
            );
        }
    }
}

/// The shadow field a row's key is visible under inside its iteration:
/// `{block/key}` beside `{block}`. For a keyed collection that is the
/// entry's own key (a sequence's position); for a list it is the
/// row's derived key.
fn row_key_field(field: &str) -> String {
    format!("{field}/key")
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
        // A keyed collection: one row per entry, keyed by the entry's
        // own key, so a re-render after an insert keeps every other
        // row in place.
        Some(Ipld::Map(entries)) => entries.into_iter().collect(),
        Some(v) => vec![(entity_key.to_owned(), v)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::snapshot_template;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::window;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a `Renderer` whose template is `template_html`, mounted on a
    /// detached `<tonk-view>` host so each test is isolated.
    fn renderer(template_html: &str) -> (Renderer, Element) {
        let document = window().expect("window").document().expect("doc");
        let host = document.create_element("tonk-view").expect("create host");
        host.set_inner_html(template_html);
        let snapshot = snapshot_template(&host).expect("snapshot");
        (
            Renderer::from_snapshot_with_scalars(snapshot, &std::collections::BTreeSet::new()),
            host,
        )
    }

    /// One conclusion with string fields.
    fn row(this: &str, fields: &[(&str, &str)]) -> Conclusion {
        let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
        for (k, v) in fields {
            map.insert((*k).to_owned(), Ipld::String((*v).to_owned()));
        }
        Conclusion {
            this: this.to_owned(),
            fields: map,
        }
    }

    /// The trimmed text of every `<li>` under the host, in DOM order. The
    /// renderer's output order is what a fragment-batched insert must keep
    /// identical to a row-by-row insert.
    fn li_texts(host: &Element) -> Vec<String> {
        let lis = host.query_selector_all("li").expect("query li");
        (0..lis.length())
            .filter_map(|i| lis.item(i))
            .filter_map(|n| n.text_content())
            .map(|t| t.trim().to_owned())
            .collect()
    }

    /// `data-this=` attribute of every `<li>`, in DOM order — confirms each row
    /// is keyed to the right subject after batched inserts.
    fn li_withs(host: &Element) -> Vec<String> {
        let lis = host.query_selector_all("li").expect("query li");
        (0..lis.length())
            .filter_map(|i| lis.item(i))
            .filter_map(|n| n.dyn_into::<Element>().ok())
            .filter_map(|e| e.get_attribute("data-this"))
            .collect()
    }

    // `data-id={this}` lifts the per-conclusion repeat root to the `<li>`, so
    // the frame renders one `<li>` per subject (without a `{this}` reference
    // the body would render once as chrome over the lead conclusion).
    const LIST: &str = "<ul><li data-id={this}>{name}</li></ul>";

    /// A multi-root template with no `{this}` reference — the whole-fragment
    /// repeat, which is the space chrome's shape (a nested router and the FAB
    /// mount, both carrying `{id}`-derived attributes). The row used to record
    /// the template fragment as its root; `append_child` empties the fragment,
    /// so a second frame's attribute bindings navigated into the void and the
    /// old values stayed stamped — a space→space navigation left the previous
    /// space mounted. Regression test for that root re-pointing.
    #[dialog_common::test]
    fn it_restamps_attribute_bindings_across_frames_in_a_whole_fragment_repeat() {
        // The EXACT space-chrome template shape: two custom-element roots
        // separated by a newline text node, an unquoted `path={rest}` whose
        // field is ABSENT from the conclusions, and `{id}` attribute
        // bindings on both roots.
        let (mut r, host) = renderer(
            "<tonk-site with=\"main@{id}\" allow=\"main@{id}\" path={rest}></tonk-site>\n<tonk-fab with=\"main@profile:tonk\" space={id}></tonk-fab>\n",
        );
        r.apply(&[row("site:1", &[("id", "did:key:aaa")])]);
        let inner = host
            .query_selector("tonk-site")
            .expect("q")
            .expect("tonk-site");
        assert_eq!(
            inner.get_attribute("with").as_deref(),
            Some("main@did:key:aaa"),
            "first frame stamps the binding"
        );

        r.apply(&[row("site:1", &[("id", "did:key:bbb")])]);
        let inner = host
            .query_selector("tonk-site")
            .expect("q")
            .expect("tonk-site");
        assert_eq!(
            inner.get_attribute("with").as_deref(),
            Some("main@did:key:bbb"),
            "a retained whole-fragment row must restamp attribute bindings"
        );
        let other = host
            .query_selector("tonk-fab")
            .expect("q")
            .expect("tonk-fab");
        assert_eq!(
            other.get_attribute("space").as_deref(),
            Some("did:key:bbb"),
            "every root's bindings restamp, not just the first"
        );
        assert_eq!(
            other.get_attribute("with").as_deref(),
            Some("main@profile:tonk"),
            "the FAB keeps its profile routing context"
        );

        // Network-bearing attributes on actual custom-element roots must
        // never receive a placeholder token. This guard is intentionally
        // attribute-scoped: literal braces in ordinary text or code remain
        // valid content.
        let elements = host.query_selector_all("*").expect("query descendants");
        for index in 0..elements.length() {
            let Some(element) = elements
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            if !element.tag_name().contains('-') {
                continue;
            }
            for attribute in ["with", "allow", "path", "space", "entity", "src", "href"] {
                if let Some(value) = element.get_attribute(attribute) {
                    assert!(
                        !value.contains('{') && !value.contains('}'),
                        "{}[{attribute}] contains unresolved binding {value:?}",
                        element.tag_name(),
                    );
                }
            }
        }
    }

    /// Same whole-fragment shape, text bindings: the multi-root template's
    /// text updates ride the same re-pointed row root.
    #[dialog_common::test]
    fn it_updates_text_bindings_across_frames_in_a_whole_fragment_repeat() {
        let (mut r, host) = renderer("<h1>{title}</h1><p>{body}</p>");
        r.apply(&[row("doc:1", &[("title", "first"), ("body", "one")])]);
        assert_eq!(
            host.query_selector("h1")
                .expect("q")
                .expect("h1")
                .text_content()
                .as_deref(),
            Some("first")
        );

        r.apply(&[row("doc:1", &[("title", "second"), ("body", "two")])]);
        assert_eq!(
            host.query_selector("h1")
                .expect("q")
                .expect("h1")
                .text_content()
                .as_deref(),
            Some("second"),
            "text bindings must update across frames in a whole-fragment repeat"
        );
        assert_eq!(
            host.query_selector("p")
                .expect("q")
                .expect("p")
                .text_content()
                .as_deref(),
            Some("two")
        );
    }

    /// A keyed-collection field arrives as a `{key: value}` map: one row
    /// per entry in key order (a sequence's position order), each row
    /// keyed by its entry key and able to read it as `{field/key}`. A
    /// later frame inserting an entry between two others lands it
    /// between them and leaves the rest in place.
    #[dialog_common::test]
    fn it_iterates_a_keyed_collection_in_key_order() {
        fn frame(entries: &[(&str, &str)]) -> Conclusion {
            let mut map: BTreeMap<String, Ipld> = BTreeMap::new();
            for (key, value) in entries {
                map.insert((*key).to_owned(), Ipld::String((*value).to_owned()));
            }
            Conclusion {
                this: "nb".to_owned(),
                fields: BTreeMap::from([("block".to_owned(), Ipld::Map(map))]),
            }
        }
        fn keys(host: &Element) -> Vec<String> {
            let items = host.query_selector_all("li").expect("q");
            (0..items.length())
                .filter_map(|i| items.item(i))
                .filter_map(|n| n.dyn_into::<Element>().ok())
                .filter_map(|li| li.get_attribute("data-key"))
                .collect()
        }

        let (mut r, host) = renderer("<ul><li data-key={block/key}>{block}</li></ul>");
        r.apply(&[frame(&[("N5", "second"), ("N1", "first"), ("N9", "third")])]);
        assert_eq!(li_texts(&host), vec!["first", "second", "third"]);
        assert_eq!(keys(&host), vec!["N1", "N5", "N9"]);

        r.apply(&[frame(&[
            ("N1", "first"),
            ("N3", "between"),
            ("N5", "second"),
            ("N9", "third"),
        ])]);
        assert_eq!(li_texts(&host), vec!["first", "between", "second", "third"]);
        assert_eq!(keys(&host), vec!["N1", "N3", "N5", "N9"]);
    }

    #[dialog_common::test]
    fn it_mounts_every_row_in_frame_order() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[
            row("a", &[("name", "Ann")]),
            row("b", &[("name", "Bo")]),
            row("c", &[("name", "Cy")]),
        ]);
        // BTreeMap keys rows lexicographically by subject, so DOM order is
        // a < b < c regardless of frame order.
        assert_eq!(li_texts(&host), vec!["Ann", "Bo", "Cy"]);
        assert_eq!(li_withs(&host), vec!["a", "b", "c"]);
    }

    #[dialog_common::test]
    fn it_mounts_rows_in_frame_order_not_sorted() {
        // The initial mount inserts rows in frame iteration order (the
        // batched fragment must preserve that), unlike the update path which
        // reconciles against the sorted `rows` map.
        let (mut r, host) = renderer(LIST);
        r.apply(&[
            row("c", &[("name", "Cy")]),
            row("a", &[("name", "Ann")]),
            row("b", &[("name", "Bo")]),
        ]);
        assert_eq!(li_withs(&host), vec!["c", "a", "b"]);
        assert_eq!(li_texts(&host), vec!["Cy", "Ann", "Bo"]);
    }

    #[dialog_common::test]
    fn it_appends_a_run_of_new_rows_after_the_existing_ones() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")])]);
        // Three new rows all sort after "a" → one adjacent run, batched
        // before the trailing anchor.
        r.apply(&[
            row("a", &[("name", "Ann")]),
            row("b", &[("name", "Bo")]),
            row("c", &[("name", "Cy")]),
            row("d", &[("name", "Di")]),
        ]);
        assert_eq!(li_withs(&host), vec!["a", "b", "c", "d"]);
        assert_eq!(li_texts(&host), vec!["Ann", "Bo", "Cy", "Di"]);
    }

    #[dialog_common::test]
    fn it_inserts_a_run_of_new_rows_between_existing_ones() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")]), row("z", &[("name", "Zoe")])]);
        // "m" and "n" both sort between "a" and "z" → an adjacent run that
        // must land before "z", not at the end.
        r.apply(&[
            row("a", &[("name", "Ann")]),
            row("m", &[("name", "Mo")]),
            row("n", &[("name", "Ned")]),
            row("z", &[("name", "Zoe")]),
        ]);
        assert_eq!(li_withs(&host), vec!["a", "m", "n", "z"]);
        assert_eq!(li_texts(&host), vec!["Ann", "Mo", "Ned", "Zoe"]);
    }

    #[dialog_common::test]
    fn it_keeps_order_when_new_runs_are_split_by_a_surviving_row() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")]), row("m", &[("name", "Mo")])]);
        // New rows on both sides of the surviving "m": "b","c" (run before
        // "m") and "x","y" (run before the anchor). Two separate runs, two
        // separate anchors — the flush-on-anchor-change path.
        r.apply(&[
            row("a", &[("name", "Ann")]),
            row("b", &[("name", "Bo")]),
            row("c", &[("name", "Cy")]),
            row("m", &[("name", "Mo")]),
            row("x", &[("name", "Xi")]),
            row("y", &[("name", "Yo")]),
        ]);
        assert_eq!(li_withs(&host), vec!["a", "b", "c", "m", "x", "y"]);
        assert_eq!(li_texts(&host), vec!["Ann", "Bo", "Cy", "Mo", "Xi", "Yo"]);
    }

    #[dialog_common::test]
    fn it_removes_vanished_rows_and_keeps_the_rest_ordered() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[
            row("a", &[("name", "Ann")]),
            row("b", &[("name", "Bo")]),
            row("c", &[("name", "Cy")]),
        ]);
        r.apply(&[row("a", &[("name", "Ann")]), row("c", &[("name", "Cy")])]);
        assert_eq!(li_withs(&host), vec!["a", "c"]);
        assert_eq!(li_texts(&host), vec!["Ann", "Cy"]);
    }

    #[dialog_common::test]
    fn it_updates_surviving_rows_in_place_while_inserting_new_ones() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")]), row("c", &[("name", "Cy")])]);
        // "a" survives with a changed value, "b" is a fresh row inserted
        // between, "c" survives unchanged.
        r.apply(&[
            row("a", &[("name", "Annie")]),
            row("b", &[("name", "Bo")]),
            row("c", &[("name", "Cy")]),
        ]);
        assert_eq!(li_withs(&host), vec!["a", "b", "c"]);
        assert_eq!(li_texts(&host), vec!["Annie", "Bo", "Cy"]);
    }

    #[dialog_common::test]
    fn it_renders_an_empty_frame_with_no_rows() {
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")])]);
        r.apply(&[]);
        assert!(li_texts(&host).is_empty());
    }

    #[dialog_common::test]
    fn it_drops_a_repeat_row_whose_subject_is_empty() {
        // The empty-directory chrome fix feeds a synthetic conclusion with
        // an empty `this` (carrying only dom.host/* fields) so chrome can
        // read host attributes with zero instances. That subjectless
        // conclusion must never materialize as a repeat row.
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("", &[("name", "Ghost")]), row("a", &[("name", "Ann")])]);
        assert_eq!(li_withs(&host), vec!["a"]);
        assert_eq!(li_texts(&host), vec!["Ann"]);
    }

    #[dialog_common::test]
    fn it_removes_the_last_row_when_only_the_synthetic_lead_remains() {
        // Removing a directory's last instance leaves a frame holding just
        // the synthetic host-attribute lead (empty `this`). The single-row
        // rename heuristic must not adopt the surviving row under that
        // subjectless key — the row vanished, it wasn't renamed — or the
        // Hub keeps a ghost row with every binding blank.
        let (mut r, host) = renderer(LIST);
        r.apply(&[row("a", &[("name", "Ann")])]);
        r.apply(&[row("", &[("dom.host/data-space", "acme")])]);
        assert!(li_withs(&host).is_empty());
        assert!(li_texts(&host).is_empty());
    }

    #[dialog_common::test]
    fn it_renders_chrome_host_fields_from_a_subjectless_lead() {
        // Mirrors the FAB: chrome reads {dom.host/data-space} while the
        // repeat collection is empty. A single subjectless conclusion
        // carrying the host field renders chrome but no row.
        let (mut r, host) = renderer(
            "<div><span class=\"space\">{dom.host/data-space}</span><ul><li data-id={this}>{name}</li></ul></div>",
        );
        r.apply(&[row("", &[("dom.host/data-space", "acme")])]);
        let space = host
            .query_selector(".space")
            .expect("query .space")
            .expect("span exists");
        assert_eq!(space.text_content().unwrap_or_default().trim(), "acme");
        assert!(li_texts(&host).is_empty());
    }

    #[dialog_common::test]
    fn it_iterates_many_valued_fields_within_a_row() {
        // One conclusion whose `tags` field is many-valued: `subject={tags}`
        // lifts the iteration root to the `<span>`, so each value gets its own
        // span. The batched insert must still emit all values in order.
        let (mut r, host) =
            renderer("<ul><li data-id={this}><span subject={tags}>{tags}</span></li></ul>");
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert(
            "tags".into(),
            Ipld::List(vec![
                Ipld::String("x".into()),
                Ipld::String("y".into()),
                Ipld::String("z".into()),
            ]),
        );
        r.apply(&[Conclusion {
            this: "a".into(),
            fields,
        }]);
        let spans = host.query_selector_all("span").expect("query span");
        let texts: Vec<String> = (0..spans.length())
            .filter_map(|i| spans.item(i))
            .filter_map(|n| n.text_content())
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        assert_eq!(texts, vec!["x", "y", "z"]);
    }
}
