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
    render_at_depth(site, route, 0).await
}

/// Render with a recursion depth guard.
async fn render_at_depth(site: &SlideSite, route: &RenderRoute, depth: usize) -> Result<String> {
    if depth >= MAX_DEPTH {
        return Err(anyhow!(
            "render recursion exceeded {MAX_DEPTH} levels (cycle in nested views?)"
        ));
    }

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
            "no view found for model `{}` (looked up `display` by model)",
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

    // 6. Parse the template, collect bindings, plan, and render.
    let mut roots = tonk_render::parse_fragment(&view.display);
    let bindings = tonk_render::collect_bindings(&mut roots);
    let repeat_root = this_repeat_root(&bindings);
    let _ = build_plan_nodes(bindings.clone());
    let plan = split_plan(bindings, repeat_root);
    let conclusions: Vec<tonk_render::Conclusion> =
        folded.iter().map(to_render_conclusion).collect();
    let html = tonk_render::render(&roots, &plan, &conclusions);

    // 7. Recursively render any nested <tonk-display> in the output.
    expand_nested(site, html, depth).await
}

/// View resolution result: the `display` template and whether the
/// view is a portal (`type == "text/html"`).
struct ResolvedView {
    display: String,
    is_portal: bool,
}

/// Resolve a model/view concept name (or URI) to `(entity_uri,
/// descriptor_json)` via the Phase-1 concept query.
async fn resolve_model(site: &SlideSite, name_or_uri: &str) -> Result<(String, String)> {
    let parsed = parse_source(name_or_uri);
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

/// Resolve the view template by querying the view concept constrained
/// to the model.
async fn resolve_view(
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
async fn expand_nested(site: &SlideSite, html: String, depth: usize) -> Result<String> {
    if !html.contains("<tonk-display") {
        return Ok(html);
    }
    // Re-parse the rendered output, walk it, and replace each
    // <tonk-display> element's contents with its recursive render.
    let mut roots = tonk_render::parse_fragment(&html);
    expand_nested_nodes(site, &mut roots, depth).await?;
    Ok(tonk_render::serialize_nodes(&roots))
}

/// Recurse over a node list, rendering each `<tonk-display>` in place.
fn expand_nested_nodes<'a>(
    site: &'a SlideSite,
    nodes: &'a mut [tonk_render::Node],
    depth: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
    Box::pin(async move {
        for node in nodes.iter_mut() {
            let tonk_render::Node::Element(el) = node else {
                continue;
            };
            if el.tag == "tonk-display" {
                if let Some(route) = route_from_attrs(&el.attrs) {
                    let inner = render_at_depth(site, &route, depth + 1).await?;
                    el.children = tonk_render::parse_fragment(&inner);
                }
            } else {
                expand_nested_nodes(site, &mut el.children, depth).await?;
            }
        }
        Ok(())
    })
}

/// Build a child route from a nested `<tonk-display>`'s attributes.
/// Requires a `model`; `entity` and `view` are optional.
fn route_from_attrs(attrs: &[(String, String)]) -> Option<RenderRoute> {
    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let model = get("model")?;
    Some(RenderRoute {
        model,
        entity: get("entity"),
        view: get("view"),
    })
}

/// Wrap a portal view's HTML document in an isolated, sandboxed
/// `<iframe srcdoc>`, matching the browser's iframe isolation.
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

/// Convert a schema conclusion to a `tonk_render` conclusion. They
/// are structurally identical; this is the crate boundary hop.
fn to_render_conclusion(c: &SchemaConclusion) -> tonk_render::Conclusion {
    tonk_render::Conclusion {
        this: c.this.clone(),
        fields: c.fields.clone(),
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
