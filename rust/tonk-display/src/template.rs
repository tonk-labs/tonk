//! Snapshot the author-supplied row template, extract a binding
//! plan, and apply that plan to a clone for each rendered row.
//!
//! Two pieces:
//! 1. The pure segment parser ([`parse_segments`]) that splits a
//!    string like `"Hello {name}!"` into an alternating sequence
//!    of literal text and field references.
//! 2. (Browser-only) DOM walking that builds a [`BindingPlan`]
//!    over a `DocumentFragment` and re-applies it to a clone.

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
    let Some(root) = repeat_root else {
        return BindingPlan {
            chrome: Vec::new(),
            repeat: RepeatPlan {
                path: None,
                body: build_plan_nodes(bindings),
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
        chrome: build_plan_nodes(chrome_bindings),
        repeat: RepeatPlan {
            path: Some(root),
            body: build_plan_nodes(body_bindings),
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

#[cfg(target_arch = "wasm32")]
mod dom {
    use std::collections::BTreeMap;

    use indexmap::IndexMap;
    use ipld_core::ipld::Ipld;
    use web_sys::{DocumentFragment, Element, HtmlTemplateElement, Node, window};

    use super::{Binding, BindingKind, BindingPlan, Segment, has_field, parse_segments};
    use tonk_host::error::{ErrorDetail, ErrorKind};
    use wasm_bindgen::JsCast;

    /// Move the row template's content out of the host element
    /// into a [`DocumentFragment`] and return the fragment plus
    /// the rendering container (where rows get appended).
    ///
    /// Two cases:
    /// 1. Host contains a `<template>` — its `.content` is the
    ///    fragment, the host itself is the container, and
    ///    everything else stays put as static chrome.
    /// 2. No `<template>` — the first non-whitespace child element
    ///    is moved into a fresh fragment; the host is the
    ///    container. Subsequent siblings are also moved (so the
    ///    template can have multiple roots).
    ///
    /// Returns `Err` only when the host has no usable row template
    /// at all.
    pub fn snapshot_template(host: &Element) -> Result<Snapshot, ErrorDetail> {
        let document = window()
            .and_then(|w| w.document())
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "no document"))?;

        // Look for a <template> child anywhere in the host.
        if let Some(tpl) = find_template(host) {
            let template = tpl
                .dyn_ref::<HtmlTemplateElement>()
                .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "template cast failed"))?;
            let fragment = template.content();
            // Container is the parent of the <template> so the
            // chrome (e.g. <tbody>) that wraps the template stays
            // intact. The template element itself is removed —
            // it's an instruction, not visible chrome.
            let parent = template
                .parent_element()
                .ok_or_else(|| ErrorDetail::new(ErrorKind::Descriptor, "template has no parent"))?;
            let _ = parent.remove_child(template);
            return Ok(Snapshot {
                fragment,
                container: parent,
            });
        }

        // No <template> — move the host's child nodes into a fresh
        // fragment. Keep element nodes and *content-bearing* text nodes
        // (so a bare `{name}` text template renders), dropping only
        // whitespace-only text (the indentation between elements) and
        // comments — neither is part of the row.
        let fragment: DocumentFragment = document.create_document_fragment();
        let mut node = host.first_child();
        while let Some(current) = node {
            let next = current.next_sibling();
            let keep = match current.node_type() {
                Node::ELEMENT_NODE => true,
                Node::TEXT_NODE => current
                    .text_content()
                    .is_some_and(|text| !text.trim().is_empty()),
                _ => false,
            };
            let _ = host.remove_child(&current);
            if keep {
                let _ = fragment.append_child(&current);
            }
            node = next;
        }
        if !fragment.has_child_nodes() {
            return Err(ErrorDetail::new(
                ErrorKind::Descriptor,
                "view has no row template",
            ));
        }
        Ok(Snapshot {
            fragment,
            container: host.clone(),
        })
    }

    /// Result of [`snapshot_template`] — the inert template
    /// fragment plus the element that rendered rows append into.
    pub struct Snapshot {
        /// The template body, with `{…}` placeholders intact.
        pub fragment: DocumentFragment,
        /// Where cloned rows get appended.
        pub container: Element,
    }

    /// Find the host's own row `<template>`: the first `<template>`
    /// in tree order that is **not** inside a nested template-owning
    /// component. A `<tonk-view>` display body can embed nested
    /// `<tonk-display>`s, each owning its own `<template>`; a plain
    /// `query_selector_all("template")` would return the first
    /// nested template and `snapshot_template` would then strip it,
    /// breaking that component. The walk stops at component
    /// boundaries so each element only ever claims a template it
    /// actually owns.
    fn find_template(host: &Element) -> Option<Element> {
        let mut child = host.first_element_child();
        while let Some(el) = child {
            if el.local_name() == "template" {
                return Some(el);
            }
            if !is_template_owning_component(&el)
                && let Some(found) = find_template(&el)
            {
                return Some(found);
            }
            child = el.next_element_sibling();
        }
        None
    }

    /// Custom elements that snapshot their own `<template>` child in
    /// their `connected_callback`. Their templates belong to them,
    /// so an ancestor's template search must skip their subtrees.
    ///
    /// Keep this list in sync with the template-snapshotting custom
    /// elements that actually register (`tonk-display` / `tonk-view`
    /// in the `tonk-display` crate). A template-owning element missing
    /// from this list would have its template stolen by an ancestor —
    /// the exact bug this guard prevents.
    fn is_template_owning_component(el: &Element) -> bool {
        matches!(el.local_name().as_str(), "tonk-display" | "tonk-view")
    }

    /// Walk a fragment, replace any text node containing
    /// `{field}` with a sequence of split text nodes (so each
    /// `Field` segment lives in its own targetable node), and
    /// build the [`BindingPlan`] of paths-to-bound-nodes.
    ///
    /// Mutates `fragment` in place.
    pub fn extract_plan(fragment: &DocumentFragment) -> BindingPlan {
        let bindings = collect_bindings(fragment);
        // Fold flat bindings into the iteration-aware plan tree, then
        // split it around the per-conclusion repeat element. Iteration
        // roots inside the body are discovered by the LCA of every
        // binding referencing the same field. The fold and split are
        // pure (operate on the collected `bindings` vec), which is why
        // they live outside the wasm-only `dom` module and get
        // unit-tested natively.
        let repeat_root = super::this_repeat_root(&bindings);
        super::split_plan(bindings, repeat_root)
    }

    /// Walk a fragment, split interpolated text nodes into per-segment
    /// text nodes (mutating `fragment` in place), and return the flat
    /// list of bindings. Used by [`extract_plan`].
    fn collect_bindings(fragment: &DocumentFragment) -> Vec<Binding> {
        let mut bindings: Vec<Binding> = Vec::new();
        // (Path, NodeKind) snapshot — collect first, mutate after,
        // because mutating during walk invalidates the iterator.
        let mut text_targets: Vec<(Vec<usize>, String)> = Vec::new();
        let mut attr_targets: Vec<(Vec<usize>, IndexMap<String, String>)> = Vec::new();
        walk(
            fragment.unchecked_ref::<Node>(),
            &mut Vec::new(),
            &mut |path, node| match node.node_type() {
                Node::TEXT_NODE => {
                    let raw = node.text_content().unwrap_or_default();
                    if raw.contains('{') {
                        text_targets.push((path.to_vec(), raw));
                    }
                }
                Node::ELEMENT_NODE => {
                    if let Some(el) = node.dyn_ref::<Element>() {
                        let attrs = el.attributes();
                        let mut interpolated: IndexMap<String, String> = IndexMap::new();
                        for i in 0..attrs.length() {
                            if let Some(attr) = attrs.item(i) {
                                let name = attr.name();
                                let value = attr.value();
                                if value.contains('{') {
                                    interpolated.insert(name, value);
                                }
                            }
                        }
                        if !interpolated.is_empty() {
                            attr_targets.push((path.to_vec(), interpolated));
                        }
                    }
                }
                _ => {}
            },
        );

        // For each text target, split the original text node into
        // a sequence of single-segment text nodes so the rendered
        // value of one field doesn't trample its neighbours.
        let document = window().and_then(|w| w.document());
        for (path, raw) in text_targets {
            let segments = parse_segments(&raw);
            if !has_field(&segments) {
                continue;
            }
            let Some(node) = navigate(fragment.unchecked_ref::<Node>(), &path) else {
                continue;
            };
            let Some(parent) = node.parent_node() else {
                continue;
            };
            let document = match &document {
                Some(d) => d,
                None => continue,
            };
            // Build replacement nodes in document order, also
            // recording the new sub-path of each Field node so we
            // can target it on a clone.
            let mut new_nodes: Vec<(Node, Option<usize>)> = Vec::new();
            for seg in &segments {
                match seg {
                    Segment::Text(t) => {
                        let n: Node = document.create_text_node(t).into();
                        new_nodes.push((n, None));
                    }
                    Segment::Field(_) => {
                        let n: Node = document.create_text_node("").into();
                        new_nodes.push((n, Some(new_nodes.len())));
                    }
                }
            }
            // Replace the original node with the new sequence.
            let mut last_inserted: Node = node.clone();
            for (n, _) in &new_nodes {
                let _ = parent.insert_before(n, last_inserted.next_sibling().as_ref());
                last_inserted = n.clone();
            }
            let _ = parent.remove_child(&node);

            // Now compute paths of the new Field nodes and emit
            // bindings. The new nodes were inserted at the
            // original sibling index `path.last()`, so their paths
            // are `path[..-1] + [original_idx + i]`.
            let original_idx = *path.last().unwrap_or(&0);
            let prefix = &path[..path.len().saturating_sub(1)];
            for (i, seg) in segments.iter().enumerate() {
                if let Segment::Field(_) = seg {
                    let mut new_path = prefix.to_vec();
                    new_path.push(original_idx + i);
                    bindings.push(Binding {
                        path: new_path,
                        kind: BindingKind::Text {
                            segments: vec![seg.clone()],
                        },
                    });
                }
            }
        }

        for (path, attrs) in attr_targets {
            for (name, value) in attrs {
                let segments = parse_segments(&value);
                if !has_field(&segments) {
                    continue;
                }
                // `html:foo={x}` is the explicit "force attribute"
                // escape hatch — reads as the HTML-attribute
                // namespace. Strip the prefix so the renderer writes
                // the real name; remember the intent. The prefixed
                // attribute (left over from the parsed template) is
                // removed from the fragment so cloned rows don't
                // carry the literal `html:foo="{x}"` placeholder.
                let (attr_name, force_attribute) = match name.strip_prefix("html:") {
                    Some(stripped) => {
                        if let Some(node) = navigate(fragment.unchecked_ref::<Node>(), &path)
                            && let Some(el) = node.dyn_ref::<Element>()
                        {
                            let _ = el.remove_attribute(&name);
                        }
                        (stripped.to_string(), true)
                    }
                    None => (name, false),
                };
                bindings.push(Binding {
                    path: path.clone(),
                    kind: BindingKind::Attribute {
                        attr_name,
                        segments,
                        force_attribute,
                    },
                });
            }
        }

        bindings
    }

    /// Walk a node and its descendants, calling `visit(path, node)`
    /// for each. `path` is the sequence of child-indices from the
    /// caller's starting node down to the visited node.
    fn walk(node: &Node, path: &mut Vec<usize>, visit: &mut impl FnMut(&[usize], &Node)) {
        let children = node.child_nodes();
        for i in 0..children.length() {
            if let Some(child) = children.item(i) {
                path.push(i as usize);
                visit(path, &child);
                // Don't descend into `<style>` / `<script>`: their
                // text is CSS/JS, where `{ … }` are real braces, not
                // template `{field}` placeholders. Walking in would
                // mangle a stylesheet's rule blocks into bindings.
                if !is_raw_text_element(&child) {
                    walk(&child, path, visit);
                }
                path.pop();
            }
        }
    }

    /// `true` for elements whose content is verbatim (CSS/JS), not
    /// template markup: `<style>` and `<script>`.
    fn is_raw_text_element(node: &Node) -> bool {
        node.dyn_ref::<Element>().is_some_and(|el| {
            let tag = el.tag_name().to_ascii_lowercase();
            tag == "style" || tag == "script"
        })
    }

    /// Follow a child-index path from `root` and return the node
    /// at the end, or `None` if any step is missing.
    pub fn navigate(root: &Node, path: &[usize]) -> Option<Node> {
        let mut current = root.clone();
        for &idx in path {
            let children = current.child_nodes();
            current = children.item(idx as u32)?;
        }
        Some(current)
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

    /// Apply an attribute-form binding to the element identified by
    /// `binding.path` under `scope_root`, dispatching between
    /// property and attribute per the rules documented on
    /// [`BindingKind::Attribute`].
    ///
    /// `rendered` is the already-stringified value (computed for
    /// cache-equality checks by the caller); single-field bindings
    /// also receive the underlying [`Ipld`] value so the
    /// dispatcher can choose typed property assignment for
    /// non-strings without re-parsing.
    pub fn apply_attribute_binding(
        scope_root: &Node,
        binding: &Binding,
        rendered: &str,
        single_field_value: Option<&Ipld>,
    ) {
        let BindingKind::Attribute {
            attr_name,
            force_attribute,
            segments,
            ..
        } = &binding.kind
        else {
            return;
        };
        let Some(target) = navigate(scope_root, &binding.path) else {
            return;
        };
        let Some(el) = target.dyn_ref::<Element>() else {
            return;
        };

        // A lone `{field}` binding (not `this`) whose field is absent
        // resolves to no value: omit the attribute entirely rather than
        // setting it to the empty string. `active={dom.host/data-active}`
        // with no `data-active` on the host must leave `<tonk-sheet-binder>`
        // with no `active` attribute at all, not `active=""` — the latter
        // is a real "" selection, not "unset". A present-but-empty value
        // still writes `""`; only a missing field clears the attribute.
        let absent_single_field = single_field_value.is_none()
            && matches!(segments.as_slice(), [Segment::Field(name)] if name != "this");

        if *force_attribute {
            if absent_single_field {
                let _ = el.remove_attribute(attr_name);
            } else {
                write_forced_attribute(el, attr_name, rendered, single_field_value);
            }
            return;
        }

        if absent_single_field {
            let _ = el.remove_attribute(attr_name);
            return;
        }

        // Typed property path: a single `{field}` binding whose
        // value is anything other than an Ipld string assigns the
        // typed value as a property via Reflect.set. Strings (and
        // multi-segment bindings, which always stringify) fall
        // through to the name-in-element dispatch below.
        if let Some(value) = single_field_value
            && !matches!(value, Ipld::String(_))
        {
            let js_value = ipld_to_js_value(value);
            let key = js_sys::JsString::from(attr_name.as_str()).into();
            let _ = js_sys::Reflect::set(el.as_ref(), &key, &js_value);
            return;
        }

        // String value (single-field) or multi-segment binding —
        // the result is the rendered string. Use a property when
        // the name exists on the element, otherwise setAttribute.
        let key = js_sys::JsString::from(attr_name.as_str()).into();
        let is_property = js_sys::Reflect::has(el.as_ref(), &key).unwrap_or(false);
        if is_property {
            let value: wasm_bindgen::JsValue = js_sys::JsString::from(rendered).into();
            let _ = js_sys::Reflect::set(el.as_ref(), &key, &value);
        } else {
            let _ = el.set_attribute(attr_name, rendered);
        }
    }

    /// Force-attribute path: `setAttribute` semantics, but with
    /// HTML's bool-attribute convention when the underlying value
    /// is a JSON bool — `true` adds the empty attribute, `false`
    /// removes it.
    fn write_forced_attribute(
        el: &Element,
        attr_name: &str,
        rendered: &str,
        single_field_value: Option<&Ipld>,
    ) {
        if let Some(Ipld::Bool(b)) = single_field_value {
            if *b {
                let _ = el.set_attribute(attr_name, "");
            } else {
                let _ = el.remove_attribute(attr_name);
            }
            return;
        }
        let _ = el.set_attribute(attr_name, rendered);
    }

    /// Convert an Ipld value to a typed `JsValue` for property
    /// assignment. Strings are handled by the caller (the property
    /// path is only used for non-strings); lists/maps pass through
    /// serde-wasm-bindgen so they reach the DOM as real JS
    /// arrays/objects rather than JSON-stringified blobs.
    fn ipld_to_js_value(value: &Ipld) -> wasm_bindgen::JsValue {
        use wasm_bindgen::JsValue;
        match value {
            Ipld::Bool(b) => JsValue::from_bool(*b),
            Ipld::Integer(n) => JsValue::from_f64(*n as f64),
            Ipld::Float(f) => JsValue::from_f64(*f),
            Ipld::String(s) => JsValue::from_str(s),
            Ipld::Null => JsValue::NULL,
            other => serde_wasm_bindgen::to_value(other).unwrap_or(JsValue::NULL),
        }
    }

    /// Resolve a binding's single `{field}` reference to its
    /// underlying Ipld value. Returns `None` when the binding has
    /// multiple segments or any literal text — in that case the
    /// rendered string is the only well-defined value, and typed
    /// dispatch isn't possible.
    pub fn single_field_value<'a>(
        binding: &Binding,
        row_this: &'a str,
        row_fields: &'a BTreeMap<String, Ipld>,
        shadow: &'a BTreeMap<String, Ipld>,
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

    #[cfg(test)]
    mod find_template_tests {
        use super::{extract_plan, find_template, snapshot_template};
        #[cfg(target_arch = "wasm32")]
        use wasm_bindgen_test::wasm_bindgen_test_configure;
        use web_sys::{Element, window};
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_test_configure!(run_in_browser);

        fn host_with(inner_html: &str) -> Element {
            let document = window().expect("window").document().expect("document");
            let host: Element = document.create_element("div").expect("create div");
            host.set_inner_html(inner_html);
            host
        }

        // A display view can embed nested template-owning components,
        // each owning its own `<template>`. A host's template snapshot
        // must not adopt a nested component's template as its own —
        // doing so strips that component of its row template.
        #[dialog_common::test]
        fn it_skips_a_template_nested_in_a_component() {
            let host = host_with(
                "<tonk-display entity=\"did:key:zBook\"><ul><template><li>{title}</li></template></ul></tonk-display>",
            );
            assert!(find_template(&host).is_none());
        }

        // Tree order would pick the nested component's template first;
        // the search must skip it and reach the host's own template.
        #[dialog_common::test]
        fn it_prefers_an_own_template_over_one_inside_a_nested_component() {
            let host = host_with(
                "<tonk-display entity=\"did:key:zBook\"><template data-which=\"nested\"><li>{b}</li></template></tonk-display><template data-which=\"own\"><p>{a}</p></template>",
            );
            let found = find_template(&host).expect("a usable own template");
            assert_eq!(found.get_attribute("data-which").as_deref(), Some("own"));
        }

        // The walk recurses through plain wrappers, so an own template
        // several non-component levels deep is still found.
        #[dialog_common::test]
        fn it_finds_an_own_template_nested_in_plain_wrappers() {
            let host = host_with(
                "<section><div><ul><template><li>{a}</li></template></ul></div></section>",
            );
            assert!(find_template(&host).is_some());
        }

        // The own template lives inside a plain wrapper that comes
        // *before* a component sibling — recursion into the wrapper
        // must win over (and precede) the skipped component subtree.
        #[dialog_common::test]
        fn it_finds_an_own_template_in_a_wrapper_preceding_a_component() {
            let host = host_with(
                "<div><template data-which=\"own\"><p>{a}</p></template></div><tonk-display entity=\"did:key:zBook\"><template data-which=\"nested\"><li>{b}</li></template></tonk-display>",
            );
            let found = find_template(&host).expect("own template in the leading wrapper");
            assert_eq!(found.get_attribute("data-which").as_deref(), Some("own"));
        }

        // The boundary applies to every template-owning component: a
        // template inside a nested `tonk-display` or `tonk-view` belongs
        // to that component and must be skipped.
        #[dialog_common::test]
        fn it_skips_templates_nested_in_other_owning_components() {
            for tag in ["tonk-display", "tonk-view"] {
                let host = host_with(&format!("<{tag}><template><p>{{a}}</p></template></{tag}>"));
                assert!(
                    find_template(&host).is_none(),
                    "template nested in <{tag}> should be skipped",
                );
            }
        }

        // A `<style>` block's CSS uses `{ … }` for rule bodies, which
        // must not be parsed as template `{field}` bindings. The
        // extractor skips into `<style>`, so the stylesheet survives
        // verbatim and contributes no bindings.
        #[dialog_common::test]
        fn it_does_not_treat_style_braces_as_bindings() {
            let host = host_with(
                "<div><style>.sheet { display: grid; color: #131313; }</style>\
                 <span>{title}</span></div>",
            );
            let snapshot = snapshot_template(&host).expect("snapshot");
            let plan = extract_plan(&snapshot.fragment);
            // Only the `{title}` text binding — nothing from the CSS.
            // `{title}` is a subject field with no `{this}`, so the
            // fragment root repeats and the binding lives in the repeat
            // body; chrome is empty.
            assert!(
                plan.chrome.is_empty(),
                "expected no chrome, got {:?}",
                plan.chrome,
            );
            assert_eq!(
                plan.repeat.body.len(),
                1,
                "expected one plan node (the span's title), got {:?}",
                plan.repeat.body,
            );
            // The stylesheet text is untouched.
            let style = snapshot
                .fragment
                .query_selector("style")
                .ok()
                .flatten()
                .expect("style element preserved");
            assert!(
                style
                    .text_content()
                    .unwrap_or_default()
                    .contains("display: grid"),
                "stylesheet content should survive verbatim",
            );
        }

        // A view whose whole template is a bare `{field}` text node (no
        // wrapping element) must still snapshot and bind — the no-template
        // path keeps content-bearing text nodes, dropping only whitespace.
        #[dialog_common::test]
        fn it_snapshots_a_bare_text_node_template() {
            let host = host_with("{name}\n");
            let snapshot = snapshot_template(&host).expect("bare text node snapshots");
            assert!(
                snapshot.fragment.has_child_nodes(),
                "the `{{name}}` text node should survive the snapshot",
            );
            let plan = extract_plan(&snapshot.fragment);
            // `{name}` is a subject field with no `{this}`, so the fragment
            // root repeats and the single text binding lives in the body.
            assert_eq!(
                plan.repeat.body.len(),
                1,
                "expected one binding (the name text), got {:?}",
                plan.repeat.body,
            );
        }

        // Whitespace-only text between elements is still dropped, so a
        // template's indentation never becomes a stray empty row binding.
        #[dialog_common::test]
        fn it_drops_whitespace_only_text_between_elements() {
            let host = host_with("\n  <span>{title}</span>\n  ");
            let snapshot = snapshot_template(&host).expect("snapshot");
            // Only the `<span>` survives; the two whitespace text nodes go.
            assert_eq!(
                snapshot.fragment.child_nodes().length(),
                1,
                "whitespace-only text nodes should be dropped",
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::{
    Snapshot, apply_attribute_binding, extract_plan, navigate, render_segments,
    render_segments_with_shadow, single_field_value, snapshot_template,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_plain_text_as_one_segment() {
        assert_eq!(parse_segments("hello"), vec![Segment::Text("hello".into())]);
    }

    #[test]
    fn it_parses_a_single_field_reference() {
        assert_eq!(
            parse_segments("{name}"),
            vec![Segment::Field("name".into())]
        );
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
    fn it_parses_two_adjacent_fields() {
        assert_eq!(
            parse_segments("{first}{last}"),
            vec![
                Segment::Field("first".into()),
                Segment::Field("last".into()),
            ],
        );
    }

    #[test]
    fn it_parses_multiple_fields_with_separators() {
        assert_eq!(
            parse_segments("{name} is {age}"),
            vec![
                Segment::Field("name".into()),
                Segment::Text(" is ".into()),
                Segment::Field("age".into()),
            ],
        );
    }

    #[test]
    fn it_treats_unterminated_brace_as_literal() {
        assert_eq!(
            parse_segments("oops {name"),
            vec![Segment::Text("oops {name".into())],
        );
    }

    #[test]
    fn it_returns_empty_for_empty_input() {
        assert!(parse_segments("").is_empty());
    }

    #[test]
    fn it_detects_field_segments() {
        assert!(!has_field(&parse_segments("plain text")));
        assert!(has_field(&parse_segments("hello {name}")));
    }

    // --- LCA + plan-tree tests -------------------------------------------

    /// Helper: build a text binding referencing one field at the
    /// given path. The renderer treats a single-field text binding
    /// as `[Field(name)]`, mirroring what the splitting pass in
    /// `extract_plan` produces.
    fn text_binding(path: &[usize], field: &str) -> Binding {
        Binding {
            path: path.to_vec(),
            kind: BindingKind::Text {
                segments: vec![Segment::Field(field.into())],
            },
        }
    }

    /// Helper: build an attribute binding with one `{field}`
    /// placeholder in its value, at the given element path.
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
    fn it_returns_empty_lca_for_disjoint_paths() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2], vec![1, 2, 3]]);
        assert!(lca.is_empty());
    }

    #[test]
    fn it_returns_full_path_lca_when_paths_share_a_prefix() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2], vec![0, 1, 5, 7]]);
        assert_eq!(lca, vec![0, 1]);
    }

    #[test]
    fn it_returns_the_path_itself_for_a_single_path() {
        let lca = longest_common_path_prefix(&[vec![0, 1, 2]]);
        assert_eq!(lca, vec![0, 1, 2]);
    }

    #[test]
    fn it_returns_empty_lca_for_no_paths() {
        assert!(longest_common_path_prefix(&[]).is_empty());
    }

    #[test]
    fn it_lifts_a_single_text_placeholder_to_its_host_element() {
        // <p>{name}</p> — text binding at path [0, 0]. Its host
        // element is the <p> at [0]. The iteration root is the
        // <p>, not the text node, so empty / multi-valued data
        // can repeat the <p> wholesale.
        let nodes = build_plan_nodes(vec![text_binding(&[0, 0], "name")]);
        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "name");
                assert_eq!(path, &vec![0]);
                assert_eq!(body.len(), 1);
                match &body[0] {
                    PlanNode::Binding(b) => assert_eq!(b.path, vec![0]),
                    _ => panic!("expected Binding, got {:?}", body[0]),
                }
            }
            other => panic!("expected single Iteration, got {other:?}"),
        }
    }

    #[dialog_common::test]
    fn it_does_not_make_a_dom_host_attribute_an_iteration_root() {
        // `<tonk-sheet-binder active={dom.host/data-active}>` — an
        // attribute that copies a scalar off the outer host, not a
        // subject field. It must stay a flat `Binding`, never an
        // `Iteration`: otherwise an absent `data-active` (zero values)
        // would clone the element zero times and drop it entirely.
        let nodes = build_plan_nodes(vec![attr_binding(&[0], "active", "dom.host/data-active")]);
        match &nodes[..] {
            [PlanNode::Binding(b)] => {
                assert_eq!(b.path, vec![0]);
            }
            other => panic!("expected a single flat Binding, got {other:?}"),
        }
    }

    #[test]
    fn it_creates_independent_iteration_roots_for_sibling_placeholders() {
        // Mirrors the todo-list case:
        //   <p>You have {item} items.</p>  text at [0, 0], host <p> = [0]
        //   <li>{item}</li>                text at [1, 0], host <li> = [1]
        // LCA of host paths [0] and [1] is [] — no shared inner
        // ancestor — so each becomes its own iteration root and
        // both elements repeat independently.
        let nodes = build_plan_nodes(vec![
            text_binding(&[0, 0], "item"),
            text_binding(&[1, 0], "item"),
        ]);

        let iters: Vec<_> = nodes
            .iter()
            .filter_map(|n| match n {
                PlanNode::Iteration { field, path, .. } => Some((field.clone(), path.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            iters,
            vec![("item".to_owned(), vec![0]), ("item".to_owned(), vec![1]),],
            "expected two independent iteration roots, got {iters:?}",
        );
    }

    #[test]
    fn it_groups_multiple_occurrences_of_same_field_under_their_lca() {
        // <dl for={title}>          path [0]    attribute
        //   <dt>{title}</dt>        path [0, 0, 0]  text
        //   <dd>{title}</dd>        path [0, 1, 0]  text
        // </dl>
        // LCA = [0] — the <dl>. One iteration root.
        let nodes = build_plan_nodes(vec![
            attr_binding(&[0], "for", "title"),
            text_binding(&[0, 0, 0], "title"),
            text_binding(&[0, 1, 0], "title"),
        ]);

        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "title");
                assert_eq!(path, &vec![0]);
                // Three bindings inside, each with path
                // re-rooted relative to the <dl>.
                assert_eq!(body.len(), 3);
                let rerooted_paths: Vec<_> = body
                    .iter()
                    .filter_map(|n| match n {
                        PlanNode::Binding(b) => Some(b.path.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(rerooted_paths.contains(&Vec::<usize>::new()));
                assert!(rerooted_paths.contains(&vec![0, 0]));
                assert!(rerooted_paths.contains(&vec![1, 0]));
            }
            other => panic!("expected single Iteration, got {other:?}"),
        }
    }

    #[test]
    fn it_splits_same_field_across_disjoint_sibling_sections() {
        // <div class=workspace>            path [0]
        //   <div class=canvas>             path [0, 0]
        //     <div subtree>{sheet}</div>   path [0, 0, 0, 0]
        //   <div class=tabs>               path [0, 1]
        //     <div subtree>{sheet}</div>   path [0, 1, 0, 0]
        // Both occurrences reference `sheet` and their LCA is the
        // outer `.workspace` ([0]) — but nothing is bound ON [0], so
        // collapsing there would clone the whole workspace per sheet.
        // Expect TWO roots, one per sibling section, NOT one at [0].
        let nodes = build_plan_nodes(vec![
            text_binding(&[0, 0, 0, 0], "sheet"),
            text_binding(&[0, 1, 0, 0], "sheet"),
        ]);

        let roots: Vec<_> = nodes
            .iter()
            .filter_map(|n| match n {
                PlanNode::Iteration { field, path, .. } => Some((field.clone(), path.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            roots,
            vec![
                ("sheet".to_owned(), vec![0, 0, 0]),
                ("sheet".to_owned(), vec![0, 1, 0]),
            ],
            "two sibling sections iterating the same field must get \
             separate roots, not one at the shared ancestor",
        );
    }

    #[test]
    fn it_nests_iteration_roots_for_distinct_fields() {
        // <ul for={list}>           path [0]    attr
        //   <li for={item}>         path [0, 0] attr
        //     <span>{item}</span>   path [0, 0, 0, 0] text
        //   </li>
        // </ul>
        let nodes = build_plan_nodes(vec![
            attr_binding(&[0], "for", "list"),
            attr_binding(&[0, 0], "for", "item"),
            text_binding(&[0, 0, 0, 0], "item"),
        ]);

        // Outermost: list-iteration at [0]. Its body should
        // contain an item-iteration at relative [0] holding the
        // marker + the text binding.
        match &nodes[..] {
            [PlanNode::Iteration { field, path, body }] => {
                assert_eq!(field, "list");
                assert_eq!(path, &vec![0]);
                // Inside <ul>: one marker binding on the <ul>
                // itself (`for={list}`) at relative path [], plus
                // a nested item iteration at relative path [0].
                let inner_iter = body
                    .iter()
                    .find_map(|n| match n {
                        PlanNode::Iteration { field, path, body } => {
                            Some((field.clone(), path.clone(), body.clone()))
                        }
                        _ => None,
                    })
                    .expect("nested iteration present");
                assert_eq!(inner_iter.0, "item");
                assert_eq!(inner_iter.1, vec![0]);
                // Inside <li>: marker on <li> at relative []
                // plus the span text at relative [0, 0].
                let inner_paths: Vec<_> = inner_iter
                    .2
                    .iter()
                    .filter_map(|n| match n {
                        PlanNode::Binding(b) => Some(b.path.clone()),
                        _ => None,
                    })
                    .collect();
                assert!(inner_paths.contains(&Vec::<usize>::new()));
                assert!(inner_paths.contains(&vec![0, 0]));
            }
            other => panic!("expected outer list iteration, got {other:?}"),
        }
    }

    // --- repeat-node resolution ------------------------------------------
    //
    // The repeat node is the element the renderer clones once per folded
    // conclusion. These five cases pin the rule down (paths shown beside
    // each element; `[0]` is the single top-level element of the
    // fragment).

    // 1. <div>                       [0]
    //      <span>{count}</span>      [0, 0] text host [0, 0]
    //    No {this}, one subject ref. The fragment-root <div> repeats and
    //    gets an implicit with={this}.
    #[dialog_common::test]
    fn it_repeats_the_root_when_only_a_subject_field_is_bound() {
        let root = this_repeat_root(&[text_binding(&[0, 0, 0], "count")]);
        assert_eq!(root, Some(vec![0]));
    }

    // 2a. <div subject={this}>       [0]     attr {this}
    //       <span>{count}</span>     [0, 0]  text host [0, 0]
    //     {this} is on the outermost ref-bearing element (<div>), so the
    //     <div> repeats.
    #[dialog_common::test]
    fn it_repeats_the_element_holding_this_when_this_is_outermost() {
        let root = this_repeat_root(&[
            attr_binding(&[0], "subject", "this"),
            text_binding(&[0, 0, 0], "count"),
        ]);
        assert_eq!(root, Some(vec![0]));
    }

    // 2b. <div>                                  [0]
    //       <span data-this={this} data-name={name}>  [0, 0] attrs
    //     Every reference sits on the inner <span>, so the <span>
    //     repeats — not the <div>.
    #[dialog_common::test]
    fn it_repeats_the_inner_element_when_all_refs_are_on_it() {
        let root = this_repeat_root(&[
            attr_binding(&[0, 0], "data-this", "this"),
            attr_binding(&[0, 0], "data-name", "name"),
        ]);
        assert_eq!(root, Some(vec![0, 0]));
    }

    // 3. <div>                            [0]
    //      <button data-count={count}>    [0, 0]    attr {count}
    //        <span data-of={this}>{name}</span>  [0,0,0] attr {this}, text {name}
    //    {this} is *deeper* than {count}, so it is not on the outermost
    //    ref-bearing element (<button>). The repeat node falls back to
    //    the fragment-root <div>.
    #[dialog_common::test]
    fn it_repeats_the_root_when_this_is_nested_below_another_ref() {
        let root = this_repeat_root(&[
            attr_binding(&[0, 0], "data-count", "count"),
            attr_binding(&[0, 0, 0], "data-of", "this"),
            text_binding(&[0, 0, 0, 0], "name"),
        ]);
        assert_eq!(root, Some(vec![0]));
    }

    // 4. <div data-model={dom.host/model}>    [0]     attr dom.host ref
    //      <span data-this={this} data-name={name}>  [0, 0] attrs
    //    The {dom.host/*} reference is ignored for repeat resolution, so
    //    the inner <span> (holding {this} and {name}) repeats.
    #[dialog_common::test]
    fn it_ignores_dom_host_refs_when_resolving_the_repeat_node() {
        let root = this_repeat_root(&[
            attr_binding(&[0], "data-model", "dom.host/model"),
            attr_binding(&[0, 0], "data-this", "this"),
            attr_binding(&[0, 0], "data-name", "name"),
        ]);
        assert_eq!(root, Some(vec![0, 0]));
    }

    // Sibling top-level references with no shared enclosing element: the
    // whole fragment repeats (`None`).
    #[dialog_common::test]
    fn it_repeats_the_whole_fragment_for_sibling_roots() {
        let root = this_repeat_root(&[
            text_binding(&[0, 0], "title"),
            text_binding(&[1, 0], "summary"),
        ]);
        assert_eq!(root, None);
    }

    // A {this} marker deep in a table still names its element as the
    // repeat node when it is the outermost (and only) reference holder.
    #[dialog_common::test]
    fn it_finds_a_this_marker_nested_in_chrome() {
        // <table><tbody><tr subject={this}>  [0, 0, 0]
        //   <td>{title}</td>                 [0, 0, 0, 0, 0]
        let root = this_repeat_root(&[
            attr_binding(&[0, 0, 0], "subject", "this"),
            text_binding(&[0, 0, 0, 0, 0], "title"),
        ]);
        assert_eq!(root, Some(vec![0, 0, 0]));
    }

    // --- split_plan: chrome vs per-conclusion body -----------------------

    #[dialog_common::test]
    fn it_splits_chrome_from_the_repeat_body() {
        // <tonk-sheet title={title}>      [0]     chrome (renders once)
        //   <tr subject={this}>           [0, 0]  repeat element
        //     <td>{name}</td>             [0, 0, 0, 0]
        // The title binding stays chrome; the row binding rebases under
        // the repeat element. Each side is planned independently so the
        // cardinality-one title does not wrap the repeated row.
        let plan = split_plan(
            vec![
                attr_binding(&[0], "title", "title"),
                text_binding(&[0, 0, 0, 0], "name"),
            ],
            Some(vec![0, 0]),
        );

        assert_eq!(plan.repeat.path, Some(vec![0, 0]));
        // Chrome holds the title — one iteration root at [0].
        assert_eq!(plan.chrome.len(), 1);
        assert_eq!(node_path(&plan.chrome[0]), &[0usize][..]);
        // Body holds the name, rebased relative to the <tr>: the text
        // node was [0,0,0,0], now [0,0]; its iteration root [0].
        assert_eq!(plan.repeat.body.len(), 1);
        assert_eq!(node_path(&plan.repeat.body[0]), &[0usize][..]);
    }

    #[dialog_common::test]
    fn it_puts_everything_in_the_body_when_the_fragment_repeats() {
        // No repeat element: the whole fragment is the clone. Every
        // binding is body, paths unchanged, RepeatPlan::path stays None.
        let plan = split_plan(vec![text_binding(&[0, 0], "count")], None);
        assert!(plan.chrome.is_empty());
        assert_eq!(plan.repeat.path, None);
        assert_eq!(plan.repeat.body.len(), 1);
    }
}
