//! Target-agnostic template planner for `tonk-display` view
//! templates.
//!
//! A view template is a `{field}`-interpolated HTML fragment. This
//! crate turns the *bindings* collected from such a fragment (text
//! nodes and attributes that contain `{...}`, each addressed by a
//! child-index path) into a [`BindingPlan`]: the render-once
//! **chrome** plus the per-conclusion **repeat** body, with
//! cardinality-many fields lowered to [`PlanNode::Iteration`].
//!
//! The planner is DOM-free and target-agnostic (no `web-sys`, no
//! `tl`, no target gates), so both the browser renderer
//! (`tonk-display`, walking a real `DocumentFragment`) and the
//! headless renderer (`tonk-render`, walking a parsed `tl` tree)
//! collect bindings their own way and share this one planner: same
//! template, same plan, by construction. (The [`resolve`] and
//! [`fold`] modules, also shared, depend on `tonk-schema`'s wire
//! types — `Query` / `Conclusion` — so the crate as a whole is not
//! dependency-light, only DOM- and target-free.)
//!
//! [`render_segments`] does the per-row string substitution and is
//! shared the same way.
//!
//! It also carries the DOM-free pieces of the `<tonk-display>`
//! resolution pipeline that both the browser component and headless
//! renderers share: the wire-query builders ([`resolve`]) and the
//! row folding ([`fold`]) that turns flat query rows into the folded
//! conclusions the renderer consumes.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;

pub mod fold;
pub mod resolve;

/// One chunk of an interpolated string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// A literal text fragment.
    Text(String),
    /// A `{field}` reference; the inner string is the field name.
    Field(String),
}

/// Parse a `{field}`-interpolated string into a sequence of
/// [`Segment`]s. Single-identifier interpolation only — `{name}`
/// works, `{name + "x"}` does not (the inner expression is treated
/// as the field name verbatim, leading to a guaranteed lookup
/// miss).
///
/// A literal `{` cannot appear in input today; document this
/// limitation upstream.
pub fn parse_segments(input: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            buf.push(ch);
            continue;
        }
        // Find the matching '}'.
        let mut name = String::new();
        let mut closed = false;
        for nch in chars.by_ref() {
            if nch == '}' {
                closed = true;
                break;
            }
            name.push(nch);
        }
        if !closed {
            // Unterminated — emit as literal.
            buf.push('{');
            buf.push_str(&name);
            continue;
        }
        if !buf.is_empty() {
            out.push(Segment::Text(std::mem::take(&mut buf)));
        }
        out.push(Segment::Field(name));
    }
    if !buf.is_empty() {
        out.push(Segment::Text(buf));
    }
    out
}

/// True if any segment is a [`Segment::Field`].
pub fn has_field(segments: &[Segment]) -> bool {
    segments.iter().any(|s| matches!(s, Segment::Field(_)))
}

/// One bound location in a cloned template — a path from the
/// fragment root to a target node, plus what to bind there.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Sequence of child indices from the fragment root down to
    /// the target node. Walking the same path on a clone reaches
    /// the analogous node.
    pub path: Vec<usize>,
    /// Whether the binding fills text content or an attribute
    /// value, plus the segment list that produces the value.
    pub kind: BindingKind,
}

/// Where a [`Binding`]'s output lands.
#[derive(Debug, Clone)]
pub enum BindingKind {
    /// Replaces the target [`web_sys::Text`] node's content with
    /// `segments` rendered against the row.
    Text {
        /// Segment list — `[Field("name")]` for a node that was
        /// originally just `{name}`.
        segments: Vec<Segment>,
    },
    /// Binds a value to the target element. The renderer dispatches
    /// per value at apply time:
    ///
    /// * `force_attribute = true` (template author wrote
    ///   `html:foo={x}`): always `setAttribute(name, rendered)`;
    ///   booleans map to presence/absence.
    /// * Single-field segment with a non-string JSON value
    ///   (bool/number/array/object): set as a JS property via
    ///   `Reflect.set` with a typed `JsValue`.
    /// * Single-field segment with a string value, or multi-segment
    ///   (literal text mixed with fields): check whether `name`
    ///   exists on the element. If yes, assign as a property; if no,
    ///   `setAttribute`.
    ///
    /// The variant is named `Attribute` for historical reasons; the
    /// runtime semantics are property-or-attribute per the rules
    /// above.
    Attribute {
        /// Binding name. With `force_attribute = false` this may be
        /// applied as either an attribute or a property at render
        /// time. With `force_attribute = true` the source `html:`
        /// prefix has already been stripped.
        attr_name: String,
        /// Segment list to render per row.
        segments: Vec<Segment>,
        /// `true` when the template author wrote `html:foo={x}` —
        /// the renderer must use `setAttribute` regardless of value
        /// type.
        force_attribute: bool,
    },
}

/// All bindings extracted from a template fragment, partitioned into
/// the render-once **chrome** and the per-conclusion **repeat body**.
///
/// A `<tonk-display>` frame is a list of folded conclusions (one per
/// subject). The renderer renders `chrome` once, then clones the repeat
/// element once per conclusion, rendering `body` against each. See
/// [`this_repeat_root`] for how the repeat element is chosen and
/// `tonk-core/docs/templates.md` for the model.
#[derive(Debug, Clone, Default)]
pub struct BindingPlan {
    /// Plan nodes outside the repeat element — bindings and iterations
    /// that render **once** against the lead conclusion (e.g. a sheet
    /// title surrounding a repeated row). Empty when the whole fragment
    /// repeats.
    pub chrome: Vec<PlanNode>,
    /// The per-conclusion repeat: the template path of the element to
    /// clone once per conclusion, plus the plan applied inside each
    /// clone (`body` paths relative to that element). `path` is `None`
    /// when the whole fragment repeats (no single enclosing element);
    /// then the renderer clones every top-level node.
    pub repeat: RepeatPlan,
}

/// The per-conclusion repeat half of a [`BindingPlan`].
#[derive(Debug, Clone, Default)]
pub struct RepeatPlan {
    /// Template path of the element cloned once per conclusion, or
    /// `None` to repeat the whole fragment. The renderer stamps
    /// `with=<this>` on each rendered clone so the repeat boundary is
    /// inspectable in the DOM.
    pub path: Option<Vec<usize>>,
    /// Plan applied inside each clone. When `path` is `Some`, paths are
    /// relative to that element; when `None`, they are fragment-root
    /// relative (the whole fragment is the clone).
    pub body: Vec<PlanNode>,
}

/// One node in the binding tree. Plain `Binding` nodes are
/// substituted once per render; `Iteration` nodes mark an element
/// that gets cloned per value of a multi-valued field, with each
/// clone running its own nested body.
#[derive(Debug, Clone)]
pub enum PlanNode {
    /// A leaf binding — a `{field}` reference in text content or
    /// an attribute value. Path is from the fragment root, or
    /// from the enclosing iteration root if nested.
    Binding(Binding),
    /// An iteration over a field's values. The element at `path`
    /// (relative to its enclosing scope) is cloned once per value
    /// of `field`; `body` is rendered against each clone with
    /// `field` shadowed to the current iteration's value.
    Iteration {
        /// Field whose values drive the iteration.
        field: String,
        /// Path to the iteration-root element, relative to the
        /// enclosing scope (fragment root or parent iteration).
        path: Vec<usize>,
        /// Nested plan applied inside each clone of the root.
        /// Paths in `body` are relative to the iteration root.
        body: Vec<PlanNode>,
    },
}

/// Find the longest common prefix among a non-empty slice of
/// paths. Returns the prefix as a fresh `Vec`. Empty input
/// returns the empty prefix.
///
/// Used to locate the **lowest common ancestor** of every
/// occurrence of a single field in the template: each placeholder
/// records the DOM path to its containing node, and the LCA is
/// the longest path prefix all of those share. That LCA is the
/// element we mount as the iteration root for the field.
pub fn longest_common_path_prefix(paths: &[Vec<usize>]) -> Vec<usize> {
    let mut iter = paths.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let mut max_len = first.len();
    for p in iter {
        let common = first
            .iter()
            .zip(p.iter())
            .take_while(|(a, b)| a == b)
            .count();
        if common < max_len {
            max_len = common;
        }
        if max_len == 0 {
            break;
        }
    }
    first[..max_len].to_vec()
}

/// Distinct field names referenced by a binding. A text binding
/// has exactly one field (the splitting pass in `extract_plan`
/// ensures one field per text node); an attribute binding can
/// mix multiple `{X}` placeholders, all collected here.
///
/// `{this}` is **not** a field-iteration target — it's the root
/// scope (one instance per matched conclusion), handled by the
/// renderer's per-conclusion loop rather than `PlanNode::Iteration`.
/// Excluding it here keeps a template like `<a href="/x/{this}">`
/// from being treated as a cardinality-many field. The element that
/// binds `{this}` becomes the per-conclusion *repeat root* (so
/// surrounding chrome renders once); that is discovered separately.
/// See `tonk-core/docs/templates.md`.
///
/// `{dom.host/*}` references are excluded too: they copy a scalar
/// attribute off the *outer* host element (e.g.
/// `active={dom.host/data-active}`), they are not subject fields with
/// a cardinality. Without this exclusion a host-attribute reference
/// would make its element an iteration root, so an absent host
/// attribute (zero values) would clone the element zero times and
/// drop it entirely — `<tonk-sheet-binder active={dom.host/data-active}>`
/// vanished when the host carried no `data-active`. The same exclusion
/// already governs repeat-node resolution (see [`refers_subject`]).
pub fn binding_fields(binding: &Binding) -> Vec<String> {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    let mut out: Vec<String> = Vec::new();
    for seg in segments {
        if let Segment::Field(name) = seg
            && name != "this"
            && !is_dom_host_field(name)
            && !out.contains(name)
        {
            out.push(name.clone());
        }
    }
    out
}

/// A field name in the `dom.host/` namespace — the host element's own
/// attributes injected into the conclusion (e.g. `{dom.host/model}`).
/// These describe the *outer* host, not the repeated subject, so they
/// never participate in repeat-node resolution.
fn is_dom_host_field(name: &str) -> bool {
    name.starts_with("dom.host/")
}

/// Whether a binding is an attribute whose value is *exactly* `{this}`
/// (a bare single-segment `{this}`, attribute name irrelevant). This is
/// the explicit repeat marker the author can write, e.g.
/// `<tr subject={this}>`. A mixed value like `href="/entity/{this}"` is
/// a URL substitution, not a marker, so it does not count.
fn binds_this_marker(binding: &Binding) -> bool {
    let BindingKind::Attribute { segments, .. } = &binding.kind else {
        return false;
    };
    matches!(segments.as_slice(), [Segment::Field(name)] if name == "this")
}

/// Whether a binding references any non-`{dom.host/*}` field — a
/// subject field (`{title}`) or `{this}`, in text or attribute, bare or
/// mixed. These are the references that pin the repeat node; host-attr
/// references are excluded. Unlike [`binding_fields`] (which omits
/// `{this}` because it is not a cardinality-many iteration target),
/// this counts `{this}` — it is the primary repeat reference.
fn refers_subject(binding: &Binding) -> bool {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    segments
        .iter()
        .any(|seg| matches!(seg, Segment::Field(name) if !is_dom_host_field(name)))
}

/// Resolve the per-conclusion **repeat node**: the element the renderer
/// clones once per folded conclusion. Everything outside it is rendered
/// once as chrome.
///
/// The rule, from the smallest set of examples that pins it down:
///
/// 1. `<div><span>{count}</span></div>` — no `{this}`, one subject ref.
///    Repeat node is the **fragment root** (`<div>`), with an implicit
///    `with={this}`.
/// 2. `<div subject={this}><span>{count}</span></div>` — `{this}` on the
///    outermost ref-bearing element. Repeat node is that element
///    (`<div>`). Whereas `<div><span data-this={this} data-name={name}>`
///    has every reference on the inner `<span>`, so the `<span>` repeats.
/// 3. `<div><button data-count={count}><span data-of={this}>{name}</span>`
///    — `{this}` is *deeper* than `{count}`, so it is not on the
///    outermost ref-bearing element. Repeat node falls back to the
///    fragment root (`<div>`).
/// 4. `<div data-model={dom.host/model}><span data-this={this} ...>` —
///    the `{dom.host/*}` reference is ignored; the `<span>` holding
///    `{this}` repeats.
///
/// Stated as one rule: among bindings that reference a subject field
/// (anything but `{dom.host/*}`), find the **outermost** (shallowest)
/// host element. If a bare `{this}` marker sits on *that* element, it is
/// the repeat node. Otherwise — no `{this}`, or `{this}` nested below
/// another reference — the **fragment-root element** containing the
/// references is the repeat node.
///
/// `Some(path)` names the exact element to clone per conclusion.
/// `None` means there is no single enclosing element (the references
/// span sibling top-level nodes), so the whole fragment repeats.
pub fn this_repeat_root(bindings: &[Binding]) -> Option<Vec<usize>> {
    // Host paths of every reference that pins the repeat node.
    let subject_hosts: Vec<Vec<usize>> = bindings
        .iter()
        .filter(|b| refers_subject(b))
        .map(host_element_path)
        .collect();
    if subject_hosts.is_empty() {
        // Nothing references the subject — no repeat axis. (A template
        // built only from `{dom.host/*}` refs, say.) Whole fragment.
        return None;
    }

    // The outermost ref-bearing element is the shallowest host path,
    // when it is unique. A `{this}` marker on *that* element makes the
    // element itself the repeat node (examples 2b/4: every ref on the
    // inner `<span>`; example 2a: `{this}` on the outer `<div>`).
    let min_len = subject_hosts.iter().map(Vec::len).min().expect("non-empty");
    let mut shallowest: Vec<Vec<usize>> = subject_hosts
        .iter()
        .filter(|h| h.len() == min_len)
        .cloned()
        .collect();
    shallowest.sort();
    shallowest.dedup();
    if let [outermost] = shallowest.as_slice() {
        let this_on_outermost = bindings
            .iter()
            .filter(|b| binds_this_marker(b))
            .any(|b| &host_element_path(b) == outermost);
        if this_on_outermost {
            return Some(outermost.clone());
        }
    }

    // No `{this}` on the outermost element (absent, nested deeper, or
    // the outermost refs split across siblings). The repeat node is the
    // fragment-root *element* that encloses every reference — the
    // common length-1 prefix shared by all host paths. If they don't
    // share one (genuine multi-root fragment), the whole fragment
    // repeats (`None`).
    let first = subject_hosts[0].first().copied();
    match first {
        Some(idx) if subject_hosts.iter().all(|h| h.first() == Some(&idx)) => Some(vec![idx]),
        _ => None,
    }
}

/// The path of the smallest enclosing element containing a
/// binding's placeholder. For text bindings the binding's `path`
/// points at the text node, so the enclosing element is `path[..-1]`.
/// For attribute bindings the path is already on the element itself.
fn host_element_path(binding: &Binding) -> Vec<usize> {
    match binding.kind {
        BindingKind::Text { .. } => binding.path[..binding.path.len().saturating_sub(1)].to_vec(),
        BindingKind::Attribute { .. } => binding.path.clone(),
    }
}

/// Walk a flat binding vec and produce the iteration-aware plan
/// tree.
///
/// Algorithm:
///
/// 1. For every distinct field name referenced, collect the
///    *host element paths* of every binding that mentions it
///    (the smallest element containing the placeholder).
/// 2. Compute the LCA of those host paths via
///    [`longest_common_path_prefix`].
///    - **Non-empty LCA**: the bindings live under a shared
///      enclosing element. Use that LCA as the field's single
///      iteration root; every binding gets pulled into its body.
///    - **Empty LCA but only one host path**: lift that host
///      element as the iteration root. (Single-occurrence
///      placeholders still need to be inside an iteration so
///      empty / many-cardinality values render correctly.)
///    - **Empty LCA with multiple host paths**: the placeholders
///      are siblings under the fragment root with no shared
///      inner ancestor. Each host element becomes its own
///      iteration root for the field — they repeat independently.
/// 3. Sort iteration roots by ascending path length so outer
///    roots are placed first, then walked inside-out when
///    closing scopes.
/// 4. Iteration roots build a scope stack; bindings get attached
///    to the deepest enclosing scope with paths re-rooted
///    relative to that scope.
///
/// Pure — no DOM access — so unit tests run natively.
pub fn build_plan_nodes(bindings: Vec<Binding>) -> Vec<PlanNode> {
    build_plan_nodes_with_scalars(bindings, &std::collections::BTreeSet::new())
}

/// Like [`build_plan_nodes`], but excludes `scalar_fields` — the concept's
/// `cardinality: one` field names — from becoming iteration roots. A scalar
/// field referenced in a template is a plain substitution rendered once; only
/// `cardinality: many` fields are iteration axes. Without this, an element whose
/// only hole is an absent optional field is cloned zero times and dropped (the
/// same failure the `{dom.host/*}` exclusion already guards). Callers that lack
/// the descriptor pass an empty set (the value-driven behaviour of
/// [`build_plan_nodes`]).
pub fn build_plan_nodes_with_scalars(
    bindings: Vec<Binding>,
    scalar_fields: &std::collections::BTreeSet<String>,
) -> Vec<PlanNode> {
    // Group bindings by every field they reference. A binding
    // that mentions two fields appears in two groups. Use the
    // *host element path* (smallest containing element), not the
    // raw binding path, so siblings-under-fragment-root don't
    // accidentally share an LCA at a text node level.
    let mut field_hosts: std::collections::BTreeMap<String, Vec<Vec<usize>>> =
        std::collections::BTreeMap::new();
    for b in &bindings {
        let host = host_element_path(b);
        for field in binding_fields(b) {
            field_hosts.entry(field).or_default().push(host.clone());
        }
    }

    // Iteration root determination per field.
    let mut iter_roots: Vec<(String, Vec<usize>)> = Vec::new();
    for (field, hosts) in field_hosts {
        // A `cardinality: one` field is never an iteration axis — iteration is
        // a cardinality-many concept. Skipping it here leaves its binding to be
        // placed as a flat `Binding` below, so an absent value renders the host
        // element once with a blank hole instead of cloning it zero times and
        // dropping it. (The `{dom.host/*}` exclusion in `binding_fields` guards
        // the same failure mode for host-attribute references.)
        if scalar_fields.contains(&field) {
            continue;
        }
        // `{x/key}` is the key of the `{x}` row it sits in — the renderer
        // shadows it per row — so it is a plain binding inside `x`'s
        // iteration, never an iteration axis of its own.
        if field.ends_with("/key") {
            continue;
        }
        let lca = longest_common_path_prefix(&hosts);
        // A binding whose host *is* the LCA element (e.g. `for={f}`
        // on the shared ancestor, or a `{f}` directly on it) pins the
        // iteration to that one element — a single root carrying the
        // whole subtree.
        let bound_on_lca = hosts.contains(&lca);
        if !lca.is_empty() && bound_on_lca {
            // Shared inner ancestor that is itself a host — single
            // iteration root rooted at the LCA.
            iter_roots.push((field, lca));
        } else if !lca.is_empty() {
            // Shared inner ancestor, but no binding sits on it: the
            // occurrences live in disjoint subtrees below the LCA
            // (e.g. the same field iterated in two sibling sections).
            // Root each disjoint cluster separately so the LCA element
            // is NOT cloned per item — otherwise the whole shared
            // ancestor repeats once per value. Cluster by the first
            // path segment past the LCA; the per-cluster root is the
            // LCA of that cluster's hosts.
            let mut clusters: std::collections::BTreeMap<usize, Vec<Vec<usize>>> =
                std::collections::BTreeMap::new();
            for host in &hosts {
                let key = host[lca.len()];
                clusters.entry(key).or_default().push(host.clone());
            }
            for (_, cluster_hosts) in clusters {
                let root = longest_common_path_prefix(&cluster_hosts);
                if root.is_empty() {
                    continue;
                }
                iter_roots.push((field.clone(), root));
            }
        } else {
            // No shared inner ancestor. Each distinct host path
            // becomes its own iteration root (a placeholder at
            // the fragment root itself — empty host path — falls
            // through as a flat binding instead).
            let mut seen: std::collections::BTreeSet<Vec<usize>> =
                std::collections::BTreeSet::new();
            for host in hosts {
                if host.is_empty() || !seen.insert(host.clone()) {
                    continue;
                }
                iter_roots.push((field.clone(), host));
            }
        }
    }

    // Outer-first ordering, then deterministic on path content.
    iter_roots.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.1.cmp(&b.1)));

    // Scope: an iteration's accumulating body. The first scope
    // (path = []) is the top level — its `body` is what we
    // ultimately return.
    let mut scopes: Vec<Scope> = vec![Scope {
        path: Vec::new(),
        field: String::new(),
        body: Vec::new(),
    }];

    // Open one new scope per iteration root, nested by absolute
    // path. Order is outer-first per the sort above.
    for (field, lca) in iter_roots {
        scopes.push(Scope {
            path: lca,
            field,
            body: Vec::new(),
        });
    }

    // Place each binding into the deepest open scope whose path
    // is a prefix of the binding's path. Re-root the binding's
    // path against the chosen scope.
    for b in bindings {
        let idx = deepest_enclosing(&scopes, &b.path);
        let parent_len = scopes[idx].path.len();
        let mut rerooted = b;
        rerooted.path = rerooted.path[parent_len..].to_vec();
        scopes[idx].body.push(PlanNode::Binding(rerooted));
    }

    // Close scopes inside-out: pop the innermost, wrap its body
    // in an Iteration node with path re-rooted against its
    // parent, push the Iteration into the parent's body.
    while scopes.len() > 1 {
        let inner = scopes.pop().expect("scope present");
        let parent_idx = deepest_enclosing(&scopes, &inner.path);
        let parent_len = scopes[parent_idx].path.len();
        let rel_path = inner.path[parent_len..].to_vec();
        // Sort the iteration's own body by path so siblings come
        // out in document order even though we built the tree
        // inside-out.
        let mut sorted_body = inner.body;
        sort_nodes_by_path(&mut sorted_body);
        scopes[parent_idx].body.push(PlanNode::Iteration {
            field: inner.field,
            path: rel_path,
            body: sorted_body,
        });
    }

    let mut top = scopes
        .into_iter()
        .next()
        .map(|s| s.body)
        .unwrap_or_default();
    sort_nodes_by_path(&mut top);
    top
}

/// Sort a list of plan nodes by their template-source path so
/// the rendered output matches source order. Iteration node
/// paths take precedence over their bodies; bindings sort by
/// their own path.
fn sort_nodes_by_path(nodes: &mut [PlanNode]) {
    nodes.sort_by(|a, b| node_path(a).cmp(node_path(b)));
}

/// The template path of a top-level plan node — a `Binding`'s target
/// path or an `Iteration` root's path. Used to partition a plan into
/// the chrome (outside the repeat root) and the per-conclusion row
/// (inside it).
pub fn node_path(node: &PlanNode) -> &[usize] {
    match node {
        PlanNode::Binding(b) => &b.path,
        PlanNode::Iteration { path, .. } => path,
    }
}

/// Partition the flat bindings around the per-conclusion repeat element
/// and build a [`BindingPlan`]: the **chrome** (bindings outside the
/// repeat element, planned and rendered once) and the **repeat body**
/// (bindings at or under it, paths rebased relative to the repeat
/// element, planned and rendered per conclusion).
///
/// Splitting the *flat bindings* — before [`build_plan_nodes`] folds
/// them into iteration trees — is what keeps chrome and body
/// independent. A cardinality-one chrome field (a sheet `title`) would
/// otherwise be lifted into an iteration root that wraps the repeat
/// element, swallowing the row into the title's subtree. Planning each
/// side separately means the title iterates only its own value and the
/// repeat element is the body's root.
///
/// `repeat_root` is `None` when the whole fragment repeats: there is no
/// chrome, every binding is body, paths unchanged.
pub fn split_plan(bindings: Vec<Binding>, repeat_root: Option<Vec<usize>>) -> BindingPlan {
    split_plan_with_scalars(bindings, repeat_root, &std::collections::BTreeSet::new())
}

/// Like [`split_plan`], but threads `scalar_fields` (the concept's
/// `cardinality: one` field names) through to [`build_plan_nodes_with_scalars`]
/// for both the chrome and the repeat body, so an optional scalar field never
/// becomes an iteration root (and so never drops its host when absent). Callers
/// without the descriptor pass an empty set.
pub fn split_plan_with_scalars(
    bindings: Vec<Binding>,
    repeat_root: Option<Vec<usize>>,
    scalar_fields: &std::collections::BTreeSet<String>,
) -> BindingPlan {
    let Some(root) = repeat_root else {
        return BindingPlan {
            chrome: Vec::new(),
            repeat: RepeatPlan {
                path: None,
                body: build_plan_nodes_with_scalars(bindings, scalar_fields),
            },
        };
    };

    let mut chrome_bindings = Vec::new();
    let mut body_bindings = Vec::new();
    for b in bindings {
        // A binding belongs to the body when its target sits at or
        // under the repeat element. Compare the *host element* path so
        // an attribute on the repeat element itself (host == root) and
        // a text node inside it both land in the body.
        if host_element_path(&b).starts_with(&root) {
            let mut rebased = b;
            rebased.path = rebased.path[root.len()..].to_vec();
            body_bindings.push(rebased);
        } else {
            chrome_bindings.push(b);
        }
    }

    BindingPlan {
        chrome: build_plan_nodes_with_scalars(chrome_bindings, scalar_fields),
        repeat: RepeatPlan {
            path: Some(root),
            body: build_plan_nodes_with_scalars(body_bindings, scalar_fields),
        },
    }
}

/// Internal: one accumulating iteration scope inside
/// [`build_plan_nodes`]. Defined at module scope (rather than
/// inside the function) so `deepest_enclosing` can take a
/// `&[Scope]` reference.
struct Scope {
    path: Vec<usize>,
    field: String,
    body: Vec<PlanNode>,
}

/// Index of the deepest scope whose path is a (possibly equal)
/// prefix of `target`. The top-level scope's path is `[]`, which
/// is a prefix of every path, so the result is always defined.
/// Internal helper for `build_plan_nodes`.
fn deepest_enclosing(scopes: &[Scope], target: &[usize]) -> usize {
    let mut best = 0;
    for (i, scope) in scopes.iter().enumerate() {
        if target.starts_with(&scope.path) && scope.path.len() > scopes[best].path.len() {
            best = i;
        }
    }
    best
}

/// Render `segments` against `row_fields` into a single string.
/// Missing fields yield empty strings; `{this}` resolves to
/// `row_this`.
pub fn render_segments(
    segments: &[Segment],
    row_this: &str,
    row_fields: &BTreeMap<String, Ipld>,
) -> String {
    render_segments_with_shadow(segments, row_this, row_fields, &BTreeMap::new())
}

/// Like [`render_segments`] but consults `shadow` first for
/// each `{field}` lookup, falling back to `row_fields` on
/// miss. Used by the iteration renderer so a per-iteration
/// value can override the conclusion's full field set inside
/// the iterated subtree.
pub fn render_segments_with_shadow(
    segments: &[Segment],
    row_this: &str,
    row_fields: &BTreeMap<String, Ipld>,
    shadow: &BTreeMap<String, Ipld>,
) -> String {
    let mut out = String::new();
    for seg in segments {
        match seg {
            Segment::Text(t) => out.push_str(t),
            Segment::Field(name) => {
                if name == "this" {
                    out.push_str(row_this);
                } else if let Some(v) = shadow.get(name) {
                    push_ipld_value(&mut out, v);
                } else if let Some(v) = row_fields.get(name) {
                    push_ipld_value(&mut out, v);
                }
            }
        }
    }
    out
}

/// Resolve a binding's single `{field}` reference to its underlying
/// Ipld value. Returns `None` when the binding has multiple segments
/// or any literal text (then the rendered string is the only
/// well-defined value, and typed dispatch isn't possible). `{this}`
/// resolves to `row_this`.
pub fn single_field_value(
    binding: &Binding,
    row_this: &str,
    row_fields: &BTreeMap<String, Ipld>,
    shadow: &BTreeMap<String, Ipld>,
) -> Option<Ipld> {
    let segments = match &binding.kind {
        BindingKind::Text { segments } => segments,
        BindingKind::Attribute { segments, .. } => segments,
    };
    let [Segment::Field(name)] = segments.as_slice() else {
        return None;
    };
    if name == "this" {
        return Some(Ipld::String(row_this.to_string()));
    }
    shadow.get(name).or_else(|| row_fields.get(name)).cloned()
}

fn push_ipld_value(out: &mut String, v: &Ipld) {
    match v {
        Ipld::String(s) => out.push_str(s),
        Ipld::Integer(n) => out.push_str(&n.to_string()),
        Ipld::Float(f) => out.push_str(&f.to_string()),
        Ipld::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Ipld::Null => {}
        other => out.push_str(&ipld_to_json_string(other)),
    }
}

/// Stringify an `Ipld` value as compact JSON. Used as a
/// best-effort fallback when a field's value lands in an
/// interpolated string position but isn't a scalar — the
/// template author at least sees the shape.
fn ipld_to_json_string(value: &Ipld) -> String {
    serde_ipld_dagjson::to_vec(value)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// The host attributes a plan's bindings consume via `{dom.host/<attr>}`
/// references — render inputs that come from the DISPLAY HOST's own
/// attributes rather than from conclusions. A display watches exactly this
/// set for changes and replays its cached frame through the binding diff,
/// so `dom.host/*` behaves as a live binding instead of a mount-time
/// snapshot (e.g. the FAB's `data-space`, restamped on a space switch).
pub fn host_attributes(plan: &BindingPlan) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    collect_host_attributes(&plan.chrome, &mut out);
    collect_host_attributes(&plan.repeat.body, &mut out);
    out
}

/// Walk `nodes` (recursing into iterations) and record every
/// `dom.host/<attr>` field reference's attribute name into `out`.
fn collect_host_attributes(nodes: &[PlanNode], out: &mut std::collections::BTreeSet<String>) {
    for node in nodes {
        match node {
            PlanNode::Binding(binding) => {
                let segments = match &binding.kind {
                    BindingKind::Text { segments } => segments,
                    BindingKind::Attribute { segments, .. } => segments,
                };
                for segment in segments {
                    if let Segment::Field(field) = segment
                        && let Some(attr) = field.strip_prefix("dom.host/")
                    {
                        out.insert(attr.to_owned());
                    }
                }
            }
            PlanNode::Iteration { field, body, .. } => {
                if let Some(attr) = field.strip_prefix("dom.host/") {
                    out.insert(attr.to_owned());
                }
                collect_host_attributes(body, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn text_binding(path: &[usize], field: &str) -> Binding {
        Binding {
            path: path.to_vec(),
            kind: BindingKind::Text {
                segments: vec![Segment::Field(field.into())],
            },
        }
    }

    fn attr_binding(path: &[usize], attr: &str, field: &str) -> Binding {
        Binding {
            path: path.to_vec(),
            kind: BindingKind::Attribute {
                attr_name: attr.into(),
                segments: vec![Segment::Field(field.into())],
                force_attribute: false,
            },
        }
    }

    #[test]
    fn it_does_not_make_a_cardinality_one_field_an_iteration_root() {
        // `<tonk-site path={rest}>` where `rest` is a cardinality-one
        // (optional) field. Given the scalar-field set, it must stay a flat
        // `Binding`, never an `Iteration`: otherwise an absent `rest` (zero
        // values) clones the element zero times and drops it entirely — the
        // same failure mode the `{dom.host/*}` exclusion already guards.
        let scalars: std::collections::BTreeSet<String> = ["rest".to_owned()].into_iter().collect();
        let nodes =
            build_plan_nodes_with_scalars(vec![attr_binding(&[0], "path", "rest")], &scalars);
        match &nodes[..] {
            [PlanNode::Binding(b)] => assert_eq!(b.path, vec![0]),
            other => panic!("expected a single flat Binding, got {other:?}"),
        }
    }

    #[test]
    fn split_plan_keeps_a_scalar_body_field_a_flat_binding() {
        // A `cardinality: one` field inside the repeat body must plan as a flat
        // `Binding`, not an `Iteration`, so an absent value renders the host
        // once rather than dropping it.
        let scalars: std::collections::BTreeSet<String> = ["rest".to_owned()].into_iter().collect();
        let plan = split_plan_with_scalars(
            vec![attr_binding(&[0, 0], "path", "rest")],
            Some(vec![0]),
            &scalars,
        );
        assert!(
            plan.repeat
                .body
                .iter()
                .all(|n| matches!(n, PlanNode::Binding(_))),
            "scalar field should stay a flat Binding in the body, got {:?}",
            plan.repeat.body
        );
    }

    #[test]
    fn it_still_iterates_a_field_absent_from_the_scalar_set() {
        // A field NOT declared cardinality-one (absent from the scalar set)
        // keeps the existing value-driven iteration behaviour — its host
        // element becomes an iteration root.
        let scalars: std::collections::BTreeSet<String> = ["rest".to_owned()].into_iter().collect();
        let nodes = build_plan_nodes_with_scalars(vec![text_binding(&[0, 0], "item")], &scalars);
        match &nodes[..] {
            [PlanNode::Iteration { field, .. }] => assert_eq!(field, "item"),
            other => panic!("expected a single Iteration, got {other:?}"),
        }
    }

    #[test]
    fn it_parses_text_field_text() {
        assert_eq!(
            parse_segments("Hello {name}!"),
            vec![
                Segment::Text("Hello ".into()),
                Segment::Field("name".into()),
                Segment::Text("!".into()),
            ],
        );
    }

    #[test]
    fn it_treats_an_unterminated_brace_as_literal() {
        assert_eq!(parse_segments("a {b"), vec![Segment::Text("a {b".into())],);
    }

    #[test]
    fn it_builds_one_plan_node_for_one_binding() {
        let nodes = build_plan_nodes(vec![text_binding(&[0], "name")]);
        assert_eq!(nodes.len(), 1, "one binding -> one plan node");
        assert_eq!(node_path(&nodes[0]), &[0]);
    }

    #[test]
    fn it_splits_chrome_from_a_repeat_root() {
        // Two subject fields under a common element: the planner
        // can split a per-conclusion repeat from the surrounding
        // chrome given a repeat root.
        let bindings = vec![
            text_binding(&[0, 0], "title"),
            text_binding(&[1, 0], "name"),
        ];
        let plan = split_plan(bindings, Some(vec![1]));
        assert!(
            plan.repeat.path.is_some() || !plan.chrome.is_empty(),
            "split produced a usable plan"
        );
    }

    #[test]
    fn it_renders_segments_against_a_row() {
        let segments = parse_segments("Hi {name}");
        let mut fields = BTreeMap::new();
        fields.insert("name".to_string(), Ipld::String("Ada".into()));
        assert_eq!(render_segments(&segments, "did:key:z", &fields), "Hi Ada");
    }

    #[test]
    fn it_resolves_this_in_segments() {
        let segments = parse_segments("{this}");
        let fields = BTreeMap::new();
        assert_eq!(
            render_segments(&segments, "did:key:zEntity", &fields),
            "did:key:zEntity"
        );
    }

    /// `host_attributes` reports every `{dom.host/<attr>}` reference —
    /// attribute bindings, text bindings, iteration fields, and nested
    /// iteration bodies alike — from both the chrome and the repeat body.
    #[test]
    fn it_collects_dom_host_attributes_across_the_whole_plan() {
        let plan = BindingPlan {
            chrome: vec![
                PlanNode::Binding(attr_binding(&[0], "with", "dom.host/data-space")),
                PlanNode::Binding(text_binding(&[1], "dom.host/data-label")),
            ],
            repeat: RepeatPlan {
                path: None,
                body: vec![PlanNode::Iteration {
                    field: "dom.host/data-items".into(),
                    path: vec![0],
                    body: vec![PlanNode::Binding(attr_binding(
                        &[0],
                        "active",
                        "dom.host/data-active",
                    ))],
                }],
            },
        };
        let attrs = host_attributes(&plan);
        assert_eq!(
            attrs.iter().cloned().collect::<Vec<_>>(),
            ["data-active", "data-items", "data-label", "data-space"],
            "every dom.host reference is reported, deduplicated and sorted"
        );
    }

    /// A plan with no `dom.host/*` references reports an empty set — the
    /// display then installs no host-attribute watcher at all.
    #[test]
    fn it_collects_no_host_attributes_when_the_plan_reads_none() {
        let plan = BindingPlan {
            chrome: vec![PlanNode::Binding(text_binding(&[0], "name"))],
            repeat: RepeatPlan {
                path: None,
                body: vec![PlanNode::Binding(attr_binding(&[1], "href", "url"))],
            },
        };
        assert!(host_attributes(&plan).is_empty());
    }
}
