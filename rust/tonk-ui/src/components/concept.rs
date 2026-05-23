//! `/space/:space/branch/:branch/concept/:source` route.
//!
//! Resolves the `source` (bookmark name or entity URI) to its
//! [`ConceptDescriptor`] over the worker's `/query` endpoint, then
//! renders a `<tonk-concept>` element wrapped in a `<table>`
//! template auto-generated from the descriptor's field set. The
//! element subscribes to the live data on its own — this route is
//! purely chrome.
//!
//! `source` may carry a query string (`person?name=Alice`) — the
//! filters round-trip into the element's `source` attribute.

use leptos::prelude::*;
use leptos_router::hooks::{use_params, use_query_map};
use leptos_router::params::Params;
use reqwest::StatusCode;
use serde_json::json;

use crate::api;
use crate::error::TonkUiError;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkConceptParams {
    space: Option<String>,
    branch: Option<String>,
    source: Option<String>,
}

/// Concept-view route. Renders a `<table>` whose `<thead>` mirrors
/// the resolved descriptor's fields and whose `<template>` row body
/// references each field as a `{field}` placeholder. The
/// `<tonk-concept>` element clones the template per match.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkConceptView() -> impl IntoView {
    let params = use_params::<TonkConceptParams>();

    let space_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
    });
    let branch_name = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.branch)
            .filter(|s| !s.is_empty())
    });
    let source_param = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.source)
            .filter(|s| !s.is_empty())
    });

    // The element's `source` attribute carries both the bookmark
    // name *and* any filters. The route's path supplies the name;
    // the URL's query string supplies the filters. Join the two
    // back into the `name?key=value&…` form `<tonk-concept>` parses.
    let query_map = use_query_map();
    let source_attr = Signal::derive_local(move || {
        let name = source_param.get()?;
        let pairs: Vec<(String, String)> = query_map
            .get()
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v))
            .collect();
        Some(join_source_and_query(&name, &pairs))
    });

    let descriptor = LocalResource::new(move || {
        let space = space_name.get();
        let branch = branch_name.get();
        let source = source_attr.get();
        async move {
            let (Some(space), Some(branch), Some(source)) = (space, branch, source) else {
                return Ok::<Option<ResolvedDescriptor>, TonkUiError>(None);
            };
            resolve_descriptor(&space, &branch, &source).await
        }
    });

    view! {
        <Suspense fallback=|| view! { <wa-spinner></wa-spinner> }>
            <ErrorBoundary fallback=|errors| view! {
                <wa-callout variant="danger">
                    <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                    { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                </wa-callout>
            }>
                { move || descriptor.get().map(|result| result.map(|maybe| match maybe {
                    Some(resolved) => render_concept_view(
                        space_name.get().unwrap_or_default(),
                        branch_name.get().unwrap_or_default(),
                        // Banner: bare bookmark name, never the
                        // full `name?key=value` form — filters are
                        // implementation detail of the live query.
                        source_param.get().unwrap_or_default(),
                        // Element: full source-with-filters so the
                        // SSE subscription is constrained.
                        source_attr.get().unwrap_or_default(),
                        resolved,
                    ).into_any(),
                    None => view! {
                        <section class="not-found">
                            "No concept matched " { move || source_param.get() }
                        </section>
                    }.into_any(),
                })) }
            </ErrorBoundary>
        </Suspense>
    }
}

/// Worker-resolved descriptor for the source — the field names
/// extracted from the descriptor JSON. Drives the auto-generated
/// table chrome.
#[derive(Clone, Debug)]
struct ResolvedDescriptor {
    fields: Vec<String>,
}

/// Fire the Phase-1 query against `/api/.../query` to resolve a
/// `source` into its descriptor.
async fn resolve_descriptor(
    space: &str,
    branch: &str,
    source: &str,
) -> Result<Option<ResolvedDescriptor>, TonkUiError> {
    // Build a concept-of-concept query: filter by `name` or `this`
    // depending on whether `source` looks like a URI.
    let head = source.split_once('?').map(|(h, _)| h).unwrap_or(source);
    let is_uri = head.contains(':');

    let mut terms = json!({
        "this":   { "?": { "name": "this" } },
        "name":   { "?": { "name": "name" } },
        "source": { "?": { "name": "source" } }
    });
    if is_uri {
        terms["this"] = json!(head);
    } else {
        terms["name"] = json!(head);
    }
    let body = json!({
        "terms": terms,
        "predicate": {
            "with": {
                "concept":     { "the": "dialog.meta/concept",     "as": "Entity",  "cardinality": "one" },
                "name":        { "the": "dialog.meta/name",        "as": "Text",    "cardinality": "one" },
                "description": { "the": "dialog.meta/description", "as": "Text",    "cardinality": "one" },
                "source":      { "the": "dialog.meta/source",      "as": "Text",    "cardinality": "one" },
                "transient":   { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
            }
        }
    });

    let url = format!(
        "{}/api/repository/{space}/branch/{branch}/query",
        api::origin(),
    );
    let response = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| TonkUiError::ApiError(format!("phase1 fetch: {e}")))?;
    if response.status() != StatusCode::OK {
        return Err(TonkUiError::ApiError(format!(
            "phase1 returned {}",
            response.status()
        )));
    }
    let conclusions: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| TonkUiError::ApiError(format!("phase1 parse: {e}")))?;
    let Some(first) = conclusions.into_iter().next() else {
        return Ok(None);
    };
    let descriptor_json = first
        .get("fields")
        .and_then(|f| f.get("source"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TonkUiError::ApiError(
                "phase1 row had no `source` field; worker may be on an older build".to_owned(),
            )
        })?;
    let descriptor_value: serde_json::Value = serde_json::from_str(descriptor_json)
        .map_err(|e| TonkUiError::ApiError(format!("descriptor JSON: {e}")))?;
    let mut fields: Vec<String> = descriptor_value
        .get("with")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    fields.sort();
    Ok(Some(ResolvedDescriptor { fields }))
}

/// Build the `<tonk-concept>` markup with an auto-generated
/// `<table>` template inside.
///
/// `banner_name` is the bare bookmark name shown in the page
/// banner (`person`); `source_attr` is the full source-with-
/// filters form passed to the element (`person?name=Alice`).
fn render_concept_view(
    space: String,
    branch: String,
    banner_name: String,
    source_attr: String,
    descriptor: ResolvedDescriptor,
) -> impl IntoView {
    // The `<tonk-concept>` host's body is built imperatively
    // because (a) Leptos's renderer drops `<template>` (it's
    // parsed specially by the HTML parser; createElement-based
    // DOM construction doesn't preserve it inside `<tbody>`),
    // and (b) custom-element attributes need to land verbatim,
    // not under Leptos's `attr:` namespace.
    let inner_html = build_template_html(&descriptor);
    let mount: NodeRef<leptos::html::Div> = NodeRef::new();
    let space_for_effect = space.clone();
    let branch_for_effect = branch.clone();
    let source_for_effect = source_attr.clone();
    Effect::new(move |_| {
        if let Some(slot) = mount.get() {
            // Replace the slot's contents with a fresh
            // `<tonk-concept>` carrying the right attributes and
            // template body. The Effect re-runs if any of the
            // captured strings change.
            let document = leptos::prelude::document();
            slot.set_inner_html("");
            let host = match document.create_element("tonk-concept") {
                Ok(el) => el,
                Err(_) => return,
            };
            let _ = host.set_attribute("space", &space_for_effect);
            let _ = host.set_attribute("branch", &branch_for_effect);
            let _ = host.set_attribute("source", &source_for_effect);
            host.set_inner_html(&inner_html);
            let _ = slot.append_child(&host);
        }
    });

    // `descriptor` is already captured by the closure that built
    // `inner_html`; no Leptos reactivity is needed for the table
    // chrome itself.
    let _ = descriptor;

    view! {
        <header slot="main-header" class="space-banner">
            <h1 class="space-banner-title" title=banner_name.clone()>
                { banner_name.clone() }
            </h1>
        </header>
        <main class="wa-stack concept-view">
            <div class="concept-view-table" node_ref=mount></div>
        </main>
    }
}

/// Build the `<table>`-shaped inner HTML for `<tonk-concept>`.
/// Setting this via `set_inner_html` ensures the browser's HTML
/// parser preserves `<template>` correctly inside `<tbody>`.
///
/// The first column carries the entity URI (`{this}`); CSS
/// truncates it with an ellipsis so a long `did:key:…` string
/// can't push the table off-screen. The cell's `title` attribute
/// holds the full value for hover-to-reveal.
///
/// Every field cell wraps its value in a `<span>` too — a `<td>`
/// ignores `max-width`, but a block-level inner span honors it,
/// so the stylesheet can cap a column's width and keep one long
/// value from stretching the whole table.
fn build_template_html(descriptor: &ResolvedDescriptor) -> String {
    let header_cells: String = descriptor
        .fields
        .iter()
        .map(|n| format!("<th>{}</th>", html_escape(n)))
        .collect();
    let row_cells: String = descriptor
        .fields
        .iter()
        .map(|n| format!("<td><span>{{{}}}</span></td>", html_escape(n)))
        .collect();
    format!(
        "<table>\
           <thead><tr><th class=\"concept-view-this\">this</th>{header_cells}</tr></thead>\
           <tbody>\
             <template>\
               <tr>\
                 <td class=\"concept-view-this\" title=\"{{this}}\"><span>{{this}}</span></td>\
                 {row_cells}\
               </tr>\
             </template>\
           </tbody>\
         </table>",
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Join the `source` path segment and URL query pairs into the
/// `name?key=value&…` form that `<tonk-concept>`'s source attribute
/// parser understands. Empty `query` returns the bare name. Keys
/// and values are form-urlencoded.
pub fn join_source_and_query(name: &str, query: &[(String, String)]) -> String {
    if query.is_empty() {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len() + 8 * query.len());
    out.push_str(name);
    out.push('?');
    let mut first = true;
    for (k, v) in query {
        if !first {
            out.push('&');
        }
        first = false;
        out.push_str(&form_encode(k));
        out.push('=');
        out.push_str(&form_encode(v));
    }
    out
}

fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_returns_bare_name_when_query_empty() {
        assert_eq!(join_source_and_query("person", &[]), "person");
    }

    #[test]
    fn it_appends_one_filter() {
        let q = vec![("name".to_owned(), "Alice".to_owned())];
        assert_eq!(join_source_and_query("person", &q), "person?name=Alice");
    }

    #[test]
    fn it_appends_multiple_filters_with_separators() {
        let q = vec![
            ("name".to_owned(), "Alice".to_owned()),
            ("age".to_owned(), "30".to_owned()),
        ];
        assert_eq!(
            join_source_and_query("person", &q),
            "person?name=Alice&age=30",
        );
    }

    #[test]
    fn it_url_encodes_unsafe_characters() {
        let q = vec![("name".to_owned(), "Alice B".to_owned())];
        assert_eq!(join_source_and_query("person", &q), "person?name=Alice+B",);
    }

    #[test]
    fn it_preserves_uri_form_source() {
        let q = vec![];
        assert_eq!(
            join_source_and_query("did:key:zPerson", &q),
            "did:key:zPerson",
        );
    }
}
