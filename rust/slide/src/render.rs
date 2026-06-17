//! `slide render <route>` — headless server-side rendering of a
//! `<tonk-display>` view to an HTML string.
//!
//! Runs the same model -> view -> entity resolution the browser
//! component runs, but against the reactor's one-shot query, then
//! feeds the result through the shared `tonk-template` planner and
//! the `tonk-render` headless renderer. Nested `<tonk-display>`
//! elements inside a view are rendered recursively; a view whose
//! `type` is `text/html` (portal mode) is emitted as an isolated
//! `<iframe srcdoc>`.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use ipld_core::ipld::Ipld;
use tonk_schema::conclusion::Conclusion as SchemaConclusion;
use tonk_template::fold::select_rows;
use tonk_template::resolve::{
    directory_view_predicate, entity_query, instances_query, looks_like_uri, name_query,
    parse_source, phase1_query, view_by_model_query, view_predicate,
};
use tonk_template::{build_plan_nodes, split_plan, this_repeat_root};

use crate::site::SlideSite;

/// Maximum nesting depth for recursive `<tonk-display>` rendering,
/// a backstop against a view that (directly or transitively)
/// displays itself.
const MAX_DEPTH: usize = 16;

/// A parsed render route.
///
/// Grammar (mirrors the SW/display route shorthand):
/// - `/{model}` — directory: every instance of the model.
/// - `/{entity}@{model}` — a single entity of the model.
/// - `/{entity}@{model}!{view}` — a single entity, explicit view.
/// - `/{model}!{view}` — directory with an explicit view concept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRoute {
    /// The model concept name or URI.
    pub model: String,
    /// The target entity (bookmark name or URI). `None` => directory
    /// mode (render every instance).
    pub entity: Option<String>,
    /// The view *concept* name. `None` => the built-in view (detail)
    /// or directory view.
    pub view: Option<String>,
}

impl RenderRoute {
    /// Parse a route string. Leading `/` is optional. The `entity`
    /// is the part before `@`; the `view` is the part after `!`.
    pub fn parse(input: &str) -> Result<Self> {
        let s = input.strip_prefix('/').unwrap_or(input);
        if s.is_empty() {
            return Err(anyhow!("empty render route"));
        }
        // Split off the view (`!view`) first, from the end.
        let (head, view) = match s.split_once('!') {
            Some((h, v)) if !v.is_empty() => (h, Some(v.to_string())),
            Some((_, _)) => return Err(anyhow!("route has a trailing `!` with no view")),
            None => (s, None),
        };
        // Then split entity@model.
        let (entity, model) = match head.split_once('@') {
            Some((e, m)) if !e.is_empty() && !m.is_empty() => (Some(e.to_string()), m.to_string()),
            Some(_) => return Err(anyhow!("route `{input}` has an empty side of `@`")),
            None => (None, head.to_string()),
        };
        Ok(RenderRoute {
            model,
            entity,
            view,
        })
    }
}

/// Render `route` against `site` and return the HTML string.
pub async fn render(site: &SlideSite, route: &RenderRoute) -> Result<String> {
    let mut visited = Vec::new();
    render_guarded(site, route, 0, &mut visited).await
}

/// Render with both a depth backstop and a `(model, entity, view)`
/// visited-set: the depth cap catches runaway recursion, while the
/// visited-set distinguishes a genuine cycle (a view that nests
/// itself with the same parameters) from legitimately deep but finite
/// nesting.
async fn render_guarded(
    site: &SlideSite,
    route: &RenderRoute,
    depth: usize,
    visited: &mut Vec<RenderRoute>,
) -> Result<String> {
    if depth >= MAX_DEPTH {
        return Err(anyhow!(
            "render recursion exceeded {MAX_DEPTH} levels (cycle in nested views?)"
        ));
    }
    if visited.contains(route) {
        return Err(anyhow!(
            "render cycle: `{}` nests itself with the same parameters",
            route.model
        ));
    }
    visited.push(route.clone());
    let out = render_at_depth(site, route, depth, visited).await;
    visited.pop();
    out
}

async fn render_at_depth(
    site: &SlideSite,
    route: &RenderRoute,
    depth: usize,
    visited: &mut Vec<RenderRoute>,
) -> Result<String> {
    // 1. Resolve the model concept to its entity URI + descriptor.
    let (model_entity, descriptor_json) = resolve_model(site, &route.model).await?;

    // 2. Resolve the target entity, if any (bookmark name -> URI).
    let entity_uri = match &route.entity {
        Some(e) if looks_like_uri(e) => Some(e.clone()),
        Some(name) => Some(resolve_name(site, name).await?),
        None => None,
    };

    // 3. Resolve the view: pick the view predicate (explicit concept,
    //    or built-in detail/directory), query it by model, and read
    //    the `display` template + optional `type`.
    let view_descriptor = match &route.view {
        Some(view_name) => {
            let (_, view_desc_json) = resolve_model(site, view_name).await?;
            serde_json::from_str(&view_desc_json)
                .map_err(|e| anyhow!("view concept `{view_name}` descriptor invalid: {e}"))?
        }
        None if entity_uri.is_some() => view_predicate(),
        None => directory_view_predicate(),
    };
    let view = resolve_view(site, &view_descriptor, &model_entity).await?;
    let Some(view) = view else {
        return Err(anyhow!(
            "no view found for model `{}` (no model-specific view and no `_:_` default)",
            route.model
        ));
    };

    // 4. Portal views (type=text/html) render as an isolated iframe.
    if view.is_portal {
        return Ok(render_portal(&view.display));
    }

    // 5. Query the entity (detail) or all instances (directory), then
    //    fold flat rows into one conclusion per subject.
    let rows = if let Some(entity) = &entity_uri {
        run_query(site, entity_query(&descriptor_json, entity)?).await?
    } else {
        run_query(site, instances_query(&descriptor_json)?).await?
    };
    let folded = select_rows(rows);

    // 6. Parse the template, collect bindings, plan, and render. Inject
    //    the host attributes (model/entity/view) as `dom.host/*` fields
    //    so a nested `<tonk-display model={dom.host/model}>` resolves,
    //    matching the browser's `with_host_attributes`.
    let mut roots = tonk_render::parse_fragment(&view.display);
    let bindings = tonk_render::collect_bindings(&mut roots);
    let repeat_root = this_repeat_root(&bindings);
    let _ = build_plan_nodes(bindings.clone());
    let plan = split_plan(bindings, repeat_root);
    let host_fields = host_fields(route);
    let conclusions: Vec<tonk_render::Conclusion> = folded
        .iter()
        .map(|c| to_render_conclusion(c, &host_fields))
        .collect();
    let html = tonk_render::render(&roots, &plan, &conclusions);

    // 7. Recursively render any nested <tonk-display> in the output.
    expand_nested(site, html, depth, visited).await
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

/// View resolution result: the `display` template and whether the
/// view is a portal (`type == "text/html"`).
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
async fn resolve_model(site: &SlideSite, name_or_uri: &str) -> Result<(String, String)> {
    let mut parsed = parse_source(name_or_uri);
    if !parsed.is_uri() {
        // Try the Name concept first; on a hit, query Phase-1 by URI.
        if let Ok(rows) = run_query(site, name_query(&parsed.name_or_uri)).await
            && let Some(uri) = rows
                .into_iter()
                .next()
                .and_then(|r| ipld_string(r.fields.get("entity")))
        {
            parsed.name_or_uri = uri;
        }
    }
    let rows = run_query(site, phase1_query(&parsed)).await?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no concept matched `{name_or_uri}`"))?;
    let descriptor = ipld_string(row.fields.get("source"))
        .ok_or_else(|| anyhow!("concept `{name_or_uri}` has no `source` descriptor"))?;
    Ok((row.this, descriptor))
}

/// Resolve a bookmark name to its entity URI via `dialog.name/referent`.
async fn resolve_name(site: &SlideSite, name: &str) -> Result<String> {
    let rows = run_query(site, name_query(name)).await?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no entity named `{name}`"))?;
    ipld_string(row.fields.get("entity")).ok_or_else(|| anyhow!("name `{name}` has no referent"))
}

/// The model the built-in default view is keyed under. When a
/// model-specific view is absent the browser re-queries the view
/// concept constrained to this sentinel; we mirror that.
const DEFAULT_MODEL: &str = "_:_";

/// Resolve the view template by querying the view concept constrained
/// to the model. Falls back to the `_:_` default-model view when the
/// model has no specific one, matching the browser's
/// `spawn_default_view`. Returns `None` only when neither exists.
async fn resolve_view(
    site: &SlideSite,
    view_descriptor: &serde_json::Value,
    model_entity: &str,
) -> Result<Option<ResolvedView>> {
    if let Some(view) = query_view(site, view_descriptor, model_entity).await? {
        return Ok(Some(view));
    }
    // No model-specific view: try the `_:_` default.
    query_view(site, view_descriptor, DEFAULT_MODEL).await
}

/// Run the view-by-model query for one model value and read the first
/// row's `display` + `type`.
async fn query_view(
    site: &SlideSite,
    view_descriptor: &serde_json::Value,
    model_entity: &str,
) -> Result<Option<ResolvedView>> {
    let query = view_by_model_query(view_descriptor, model_entity)
        .map_err(|e| anyhow!("view query construction failed: {e}"))?;
    let rows = run_query(site, query).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let display = ipld_string(row.fields.get("display")).unwrap_or_default();
    let is_portal = ipld_string(row.fields.get("type")).as_deref() == Some("text/html");
    Ok(Some(ResolvedView { display, is_portal }))
}

/// Run a `tonk_schema::query::Query` through the reactor's one-shot
/// query and return the flat conclusions.
async fn run_query(
    site: &SlideSite,
    query: tonk_schema::query::Query,
) -> Result<Vec<SchemaConclusion>> {
    let concept_query = query
        .into_concept_query()
        .map_err(|_| anyhow!("render queries must be concept queries, not formulas"))?;
    site.reactor
        .repository(crate::site::REPO_NAME)
        .branch(crate::site::BRANCH_NAME)
        .query(concept_query)
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow!("query failed: {e}"))
}

/// Find and recursively render nested `<tonk-display>` elements in
/// `html`. Each nested element carries `model` / `entity` / `view`
/// attributes (already substituted by the parent render), which we
/// turn into a child route.
async fn expand_nested(
    site: &SlideSite,
    html: String,
    depth: usize,
    visited: &mut Vec<RenderRoute>,
) -> Result<String> {
    if !html.contains("<tonk-display") {
        return Ok(html);
    }
    // Re-parse the rendered output, walk it, and replace each
    // <tonk-display> element's contents with its recursive render.
    let mut roots = tonk_render::parse_fragment(&html);
    expand_nested_nodes(site, &mut roots, depth, visited).await?;
    Ok(tonk_render::serialize_nodes(&roots))
}

/// Recurse over a node list, rendering each `<tonk-display>` in place.
fn expand_nested_nodes<'a>(
    site: &'a SlideSite,
    nodes: &'a mut [tonk_render::Node],
    depth: usize,
    visited: &'a mut Vec<RenderRoute>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
    Box::pin(async move {
        for node in nodes.iter_mut() {
            let tonk_render::Node::Element(el) = node else {
                continue;
            };
            if el.tag == "tonk-display" {
                if let Some(route) = route_from_attrs(&el.attrs) {
                    let inner = render_guarded(site, &route, depth + 1, visited).await?;
                    el.children = tonk_render::parse_fragment(&inner);
                }
            } else {
                expand_nested_nodes(site, &mut el.children, depth, visited).await?;
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
fn to_render_conclusion(
    c: &SchemaConclusion,
    host_fields: &BTreeMap<String, Ipld>,
) -> tonk_render::Conclusion {
    let mut fields = host_fields.clone();
    fields.extend(c.fields.iter().map(|(k, v)| (k.clone(), v.clone())));
    tonk_render::Conclusion {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_a_bare_model_route() {
        let r = RenderRoute::parse("/person").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity, None);
        assert_eq!(r.view, None);
    }

    #[test]
    fn it_parses_entity_at_model() {
        let r = RenderRoute::parse("alice@person").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity.as_deref(), Some("alice"));
        assert_eq!(r.view, None);
    }

    #[test]
    fn it_parses_entity_at_model_bang_view() {
        let r = RenderRoute::parse("/alice@person!card").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity.as_deref(), Some("alice"));
        assert_eq!(r.view.as_deref(), Some("card"));
    }

    #[test]
    fn it_parses_directory_with_view() {
        let r = RenderRoute::parse("person!directory").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity, None);
        assert_eq!(r.view.as_deref(), Some("directory"));
    }

    #[test]
    fn it_keeps_a_did_key_entity_uri() {
        let r = RenderRoute::parse("did:key:zABC@person").unwrap();
        assert_eq!(r.entity.as_deref(), Some("did:key:zABC"));
        assert_eq!(r.model, "person");
    }

    #[test]
    fn it_rejects_empty_and_malformed_routes() {
        assert!(RenderRoute::parse("").is_err());
        assert!(RenderRoute::parse("/").is_err());
        assert!(RenderRoute::parse("@person").is_err());
        assert!(RenderRoute::parse("alice@").is_err());
        assert!(RenderRoute::parse("person!").is_err());
    }
}
