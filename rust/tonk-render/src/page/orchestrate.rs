//! The render orchestrator: `route -> conclusions -> HTML`.
//!
//! Runs the same model -> view -> entity resolution the browser
//! component runs, against a [`QueryBackend`], then feeds the result
//! through the shared `tonk-template` planner and this crate's pure
//! headless renderer ([`crate::render`]). Nested `<tonk-display>`
//! elements inside a view are rendered recursively; a model whose
//! `show` dictionary carries `type: text/html` (portal mode) is
//! emitted as an isolated `<iframe srcdoc>`.

use std::collections::BTreeMap;

use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion;
use tonk_template::fold::{select_rows, show_template};
use tonk_template::resolve::scalar_field_names;
use tonk_template::resolve::{
    DETAIL_FACET, DIRECTORY_FACET, TYPE_FACET, entity_query, instances_query, looks_like_uri,
    name_query, parse_source, phase1_query, view_query,
};
use tonk_template::{split_plan_with_scalars, this_repeat_root};

use crate::page::{QueryBackend, RenderError, RenderRoute};

/// Maximum nesting depth for recursive `<tonk-display>` rendering,
/// a backstop against a view that (directly or transitively)
/// displays itself.
const MAX_DEPTH: usize = 16;

/// Render `route` against `backend` and return the HTML string.
pub async fn render<B: QueryBackend>(
    backend: &B,
    route: &RenderRoute,
) -> Result<String, RenderError> {
    let mut walk = Walk::default();
    render_guarded(backend, route, 0, &mut walk, &mut Vec::new()).await
}

/// Walk the `<tonk-display>` elements that rendering `route` mounts and
/// return them as a tree, without the markup.
///
/// Each node is a display that can be rendered on its own: its
/// [`Outline::route`] parses back into a [`RenderRoute`]. A nested
/// display that fails to render keeps its place in the tree with the
/// error, so one broken view does not hide its siblings. The root
/// failing is an error.
pub async fn outline<B: QueryBackend>(
    backend: &B,
    route: &RenderRoute,
) -> Result<Outline, RenderError> {
    let mut walk = Walk {
        lenient: true,
        ..Walk::default()
    };
    let mut roots = Vec::new();
    render_guarded(backend, route, 0, &mut walk, &mut roots).await?;
    Ok(roots.pop().expect("a rendered route records its own node"))
}

/// One `<tonk-display>` mounted while rendering a route.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Outline {
    /// The display as `entity@model!view`, which [`RenderRoute::parse`]
    /// reads back to render this node alone.
    pub route: String,
    /// The model as the display named it (a name or a concept URI).
    pub model: String,
    /// The concept the model resolved to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    /// The concept's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The concept's description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The entity shown; absent for a directory of every instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// The `show` facet, when the display named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// How many records the view rendered.
    pub records: usize,
    /// The view is a sandboxed `text/html` document, not a template.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub portal: bool,
    /// Why this display did not render.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The displays this one nests, in document order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Outline>,
}

impl Outline {
    fn new(route: &RenderRoute) -> Self {
        Outline {
            route: route.to_string(),
            model: route.model.clone(),
            concept: None,
            name: None,
            description: None,
            entity: route.entity.clone(),
            view: route.view.clone(),
            records: 0,
            portal: false,
            error: None,
            children: Vec::new(),
        }
    }
}

/// State shared across one recursive render.
#[derive(Default)]
struct Walk {
    /// The routes being rendered, outermost first, to catch a view
    /// that nests itself with the same parameters.
    visited: Vec<RenderRoute>,
    /// Record a nested display's error on its node and continue,
    /// instead of failing the whole render.
    lenient: bool,
}

/// Render with both a depth backstop and a `(model, entity, view)`
/// visited-set: the depth cap catches runaway recursion, while the
/// visited-set distinguishes a genuine cycle (a view that nests
/// itself with the same parameters) from legitimately deep but finite
/// nesting. The display's node is appended to `siblings`.
async fn render_guarded<B: QueryBackend>(
    backend: &B,
    route: &RenderRoute,
    depth: usize,
    walk: &mut Walk,
    siblings: &mut Vec<Outline>,
) -> Result<String, RenderError> {
    let mut node = Outline::new(route);
    let out = if depth >= MAX_DEPTH {
        Err(RenderError::RecursionDepth(MAX_DEPTH))
    } else if walk.visited.contains(route) {
        Err(RenderError::RenderCycle(route.model.clone()))
    } else {
        walk.visited.push(route.clone());
        let out = render_at_depth(backend, route, depth, walk, &mut node).await;
        walk.visited.pop();
        out
    };
    if let Err(error) = &out {
        node.error = Some(error.to_string());
    }
    siblings.push(node);
    out
}

async fn render_at_depth<B: QueryBackend>(
    backend: &B,
    route: &RenderRoute,
    depth: usize,
    walk: &mut Walk,
    node: &mut Outline,
) -> Result<String, RenderError> {
    // 1. Resolve the model concept to its entity URI + descriptor.
    let model = resolve_model(backend, &route.model).await?;
    node.concept = Some(model.entity.clone());
    node.name = model.name;
    node.description = model.description;
    let (model_entity, descriptor_json) = (model.entity, model.descriptor);

    // 2. Resolve the target entity, if any (bookmark name -> URI).
    let entity_uri = match &route.entity {
        Some(e) if looks_like_uri(e) => Some(e.clone()),
        Some(name) => Some(resolve_name(backend, name).await?),
        None => None,
    };

    // 3. Pick the facet — the explicit route facet, else `ui` (entity
    //    set) or `directory` (directory mode) — and resolve its
    //    template from the model's `show` dictionary.
    let facet = match &route.view {
        Some(facet) => facet.as_str(),
        None if entity_uri.is_some() => DETAIL_FACET,
        None => DIRECTORY_FACET,
    };
    let view = resolve_view(backend, facet, &model_entity).await?;
    let Some(view) = view else {
        return Err(RenderError::NoView(route.model.clone()));
    };

    // 4. Portal views (a `type: text/html` entry) render as an isolated iframe.
    if view.is_portal {
        node.portal = true;
        return Ok(render_portal(&view.display));
    }

    // 5. Query the entity (detail) or all instances (directory), then
    //    fold flat rows into one conclusion per subject.
    let rows = if let Some(entity) = &entity_uri {
        run_query(backend, entity_query(&descriptor_json, entity)?).await?
    } else {
        run_query(backend, instances_query(&descriptor_json)?).await?
    };
    let folded = select_rows(rows);
    node.records = folded.len();

    // 6. Parse, plan, and render each sibling against the same entity query.
    //    Inject the host attributes (model/entity/view) as `dom.host/*`
    //    fields so nested displays resolve like the browser's
    //    `with_host_attributes` path.
    // Scalar (`cardinality: one`) fields are plain substitutions, not iteration
    // axes — so an absent optional scalar field renders its host once (blank)
    // instead of cloning it zero times and dropping it. Mirrors the browser
    // renderer's `data-scalar-fields` threading.
    let scalar_fields = scalar_field_names(&descriptor_json);
    let host_fields = host_fields(route);
    let conclusions: Vec<crate::Conclusion> = folded
        .iter()
        .map(|c| to_render_conclusion(c, &host_fields))
        .collect();
    let mut roots = crate::parse_fragment(&view.display);
    let bindings = crate::collect_bindings(&mut roots);
    let repeat_root = this_repeat_root(&bindings);
    let plan = split_plan_with_scalars(bindings, repeat_root, &scalar_fields);
    let html = crate::render(&roots, &plan, &conclusions);

    // 7. Recursively render nested <tonk-display> elements before
    //    returning.
    expand_nested(backend, html, depth, walk, &mut node.children).await
}

/// The host attributes a `<tonk-display>` would carry, as
/// `dom.host/<attr>` fields, so templates that reference
/// `{dom.host/model}` (the directory -> detail idiom) resolve.
fn host_fields(route: &RenderRoute) -> BTreeMap<String, Ipld> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "dom.host/model".to_string(),
        Ipld::String(route.model.clone()),
    );
    if let Some(entity) = &route.entity {
        fields.insert("dom.host/entity".to_string(), Ipld::String(entity.clone()));
    }
    if let Some(view) = &route.view {
        fields.insert("dom.host/view".to_string(), Ipld::String(view.clone()));
    }
    fields
}

/// View resolution result: the facet's template and whether the
/// model's views are portals (a `type: text/html` entry).
struct ResolvedView {
    display: String,
    is_portal: bool,
}

/// Resolve a model/view concept name (or URI) to `(entity_uri,
/// descriptor_json)`. Mirrors the browser's `resolve_model_query`: a
/// non-URI source is first resolved through the Name concept (so a
/// pinned concept addressed by its bookmark name, e.g. `workspace` ->
/// `tonk:workspace`, resolves), then a Phase-1 concept lookup runs
/// against the resolved URI. An unresolved name falls through to the
/// Phase-1 name lookup, which reports a clean "no concept matched".
async fn resolve_model<B: QueryBackend>(
    backend: &B,
    name_or_uri: &str,
) -> Result<ResolvedModel, RenderError> {
    let mut parsed = parse_source(name_or_uri);
    if !parsed.is_uri() {
        // Try the Name concept first; on a hit, query Phase-1 by URI.
        if let Ok(rows) = run_query(backend, name_query(&parsed.name_or_uri)).await
            && let Some(uri) = rows
                .into_iter()
                .next()
                .and_then(|r| ipld_string(r.fields.get("entity")))
        {
            parsed.name_or_uri = uri;
        }
    }
    let rows = run_query(backend, phase1_query(&parsed)).await?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| RenderError::NoConcept(name_or_uri.to_string()))?;
    let descriptor = ipld_string(row.fields.get("source")).ok_or_else(|| {
        RenderError::Descriptor(format!(
            "concept `{name_or_uri}` has no `source` descriptor"
        ))
    })?;
    Ok(ResolvedModel {
        name: ipld_string(row.fields.get("name")),
        description: serde_json::from_str::<serde_json::Value>(&descriptor)
            .ok()
            .and_then(|source| source.get("description")?.as_str().map(str::to_owned)),
        entity: row.this,
        descriptor,
    })
}

/// A model resolved to its concept.
struct ResolvedModel {
    entity: String,
    descriptor: String,
    name: Option<String>,
    description: Option<String>,
}

/// Resolve a bookmark name to its entity URI via `db.name/referent`.
async fn resolve_name<B: QueryBackend>(backend: &B, name: &str) -> Result<String, RenderError> {
    let rows = run_query(backend, name_query(name)).await?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| RenderError::UnknownName(name.to_string()))?;
    ipld_string(row.fields.get("entity"))
        .ok_or_else(|| RenderError::Descriptor(format!("name `{name}` has no referent")))
}

/// The model the built-in default view is keyed under. When a
/// model-specific view is absent the browser re-queries the view
/// concept constrained to this sentinel; we mirror that. `tonk:_`
/// is the wildcard-model entity seeded by core.yaml.
const DEFAULT_MODEL: &str = "tonk:_";

/// Resolve one facet's template from the model's `show` dictionary.
/// Falls back to the `tonk:_` default-model dictionary when the model
/// has no entry for the facet, matching the browser's
/// `spawn_default_view`. Returns `None` only when neither carries it.
async fn resolve_view<B: QueryBackend>(
    backend: &B,
    facet: &str,
    model_entity: &str,
) -> Result<Option<ResolvedView>, RenderError> {
    if let Some(view) = query_view(backend, facet, model_entity).await? {
        return Ok(Some(view));
    }
    // No model-specific entry: try the `tonk:_` default.
    query_view(backend, facet, DEFAULT_MODEL).await
}

/// Run the view query for one model entity, fold the entry rows into
/// the `show` dictionary, and read the facet's template (plus the
/// portal marker).
async fn query_view<B: QueryBackend>(
    backend: &B,
    facet: &str,
    model_entity: &str,
) -> Result<Option<ResolvedView>, RenderError> {
    let query = view_query(model_entity)
        .map_err(|e| RenderError::QueryConstruction(format!("view query: {e}")))?;
    let rows = run_query(backend, query).await?;
    let folded = select_rows(rows);
    let Some(row) = folded.first() else {
        return Ok(None);
    };
    let Some(display) = show_template(row, facet) else {
        return Ok(None);
    };
    let is_portal = show_template(row, TYPE_FACET) == Some("text/html");
    Ok(Some(ResolvedView {
        display: display.to_owned(),
        is_portal,
    }))
}

/// Lower a `tonk_schema::query::Query` to a concept query and run it
/// through the backend, returning the flat conclusions.
async fn run_query<B: QueryBackend>(
    backend: &B,
    query: tonk_schema::query::Query,
) -> Result<Vec<Conclusion>, RenderError> {
    let concept_query = query
        .into_concept_query()
        .map_err(|_| RenderError::NotConceptQuery)?;
    backend.query(concept_query).await
}

/// Find and recursively render nested `<tonk-display>` elements in
/// `html`. Each nested element carries `model` / `entity` / `view`
/// attributes (already substituted by the parent render), which we
/// turn into a child route.
async fn expand_nested<B: QueryBackend>(
    backend: &B,
    html: String,
    depth: usize,
    walk: &mut Walk,
    tree: &mut Vec<Outline>,
) -> Result<String, RenderError> {
    if !html.contains("<tonk-display") {
        return Ok(html);
    }
    // Re-parse the rendered output, walk it, and replace each
    // <tonk-display> element's contents with its recursive render.
    let mut roots = crate::parse_fragment(&html);
    expand_nested_nodes(backend, &mut roots, depth, walk, tree).await?;
    Ok(crate::serialize_nodes(&roots))
}

/// Recurse over a node list, rendering each `<tonk-display>` in place.
fn expand_nested_nodes<'a, B: QueryBackend>(
    backend: &'a B,
    nodes: &'a mut [crate::Node],
    depth: usize,
    walk: &'a mut Walk,
    tree: &'a mut Vec<Outline>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RenderError>> + 'a>> {
    Box::pin(async move {
        for node in nodes.iter_mut() {
            let crate::Node::Element(el) = node else {
                continue;
            };
            if el.tag == "tonk-display" {
                if let Some(route) = route_from_attrs(&el.attrs) {
                    match render_guarded(backend, &route, depth + 1, walk, tree).await {
                        Ok(inner) => el.children = crate::parse_fragment(&inner),
                        // The error is on the display's node; keep walking.
                        Err(_) if walk.lenient => {}
                        Err(error) => return Err(error),
                    }
                }
            } else {
                expand_nested_nodes(backend, &mut el.children, depth, walk, tree).await?;
            }
        }
        Ok(())
    })
}

/// Build a child route from a nested `<tonk-display>`'s attributes.
/// Requires a non-empty `model`; `entity` and `view` are optional. A
/// missing or empty `model` (e.g. a `{dom.host/model}` that resolved
/// to nothing) yields `None` so the nested display is left as-is
/// rather than triggering a bogus "no concept matched" on an empty
/// name. Empty `entity`/`view` attributes are likewise treated as
/// absent.
fn route_from_attrs(attrs: &[(String, String)]) -> Option<RenderRoute> {
    let get = |k: &str| {
        attrs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let model = get("model")?;
    Some(RenderRoute {
        model,
        entity: get("entity"),
        view: get("view"),
    })
}

/// Wrap a portal view's HTML document in an isolated, sandboxed
/// `<iframe srcdoc>`.
///
/// A `text/html` portal `display` is NOT a template: it is an
/// author-written document that runs its own JS against the
/// `window.tonk` bridge to fetch what it needs. The browser loads it
/// verbatim (no `{field}` interpolation) and prepends a bridge
/// bootstrap script. We inline the same verbatim content but omit the
/// bootstrap, which can't function headlessly (no service worker, no
/// message-channel peer) — so the portal's own queries don't run under
/// SSR. Inlining the content verbatim is the faithful match to the
/// browser's `content` attribute; substituting placeholders would
/// diverge from it.
fn render_portal(document: &str) -> String {
    format!(
        "<iframe sandbox=\"allow-scripts\" srcdoc=\"{}\"></iframe>",
        escape_attr(document)
    )
}

/// Escape a double-quoted attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

/// Convert a schema conclusion to a `tonk_render` conclusion, merging
/// in the `dom.host/*` host fields. The conclusion's own fields win on
/// a name clash (host fields only fill names the row doesn't define).
fn to_render_conclusion(c: &Conclusion, host_fields: &BTreeMap<String, Ipld>) -> crate::Conclusion {
    let mut fields = host_fields.clone();
    fields.extend(c.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
    crate::Conclusion {
        this: c.this.clone(),
        fields,
    }
}

/// Read an `Ipld::String` value, if present.
fn ipld_string(value: Option<&Ipld>) -> Option<String> {
    match value {
        Some(Ipld::String(s)) => Some(s.clone()),
        _ => None,
    }
}
