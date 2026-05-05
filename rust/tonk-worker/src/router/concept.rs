//! `GET /api/repository/{repo}/branch/{branch}/concept/{source}` —
//! a server-rendered HTML page that mounts a `<tonk-concept>`
//! element wrapped in a default `<table>` template, auto-generated
//! from the resolved concept's field set.
//!
//! The element handles live data on its own through the `/query`
//! SSE; the page is purely chrome — `<thead>` per field name,
//! `<template>` row body with `{this}` and one `<td>{field}</td>`
//! per descriptor field. The element clones the template per match
//! and substitutes values.

use ::axum::extract::{Path, Query, State};
use ::axum::http::{StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::Entity;
use serde::Deserialize;
use std::collections::BTreeMap;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use tonk_schema::concept::{Concept, ConceptDescriptor};

use crate::router::AppState;

/// Path parameters for the concept-view route.
#[derive(Debug, Deserialize)]
pub struct ConceptPath {
    /// Repository name.
    pub repo: String,
    /// Branch name.
    pub branch: String,
    /// Concept identifier — bookmark name (`person`) or entity URI
    /// (`did:key:zPerson…`, `xyz.tonk.person/Person`,
    /// `concept:…`). Detection is by presence of `:`.
    pub source: String,
}

/// Server-side handler. Resolves the source to a descriptor,
/// renders the wrapper page, returns it as `text/html`.
#[wasm_compat]
pub async fn concept_view(
    State(state): State<AppState>,
    Path(path): Path<ConceptPath>,
    Query(filters): Query<BTreeMap<String, String>>,
) -> Result<Response, StatusCode> {
    let tonk = state.read().await;

    // Acquire the branch via the reactor (cached handle).
    let session = tonk
        .reactor
        .repository(&path.repo)
        .branch(&path.branch)
        .acquire(&tonk.operator)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let branch = session.handle();

    let descriptor = match resolve_descriptor(&path.source, branch, &tonk.operator).await {
        Some(d) => d,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let html = render_page(
        &path.repo,
        &path.branch,
        &path.source,
        &filters,
        &descriptor,
    );
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response())
}

/// Try built-in registry first, then bookmark name, then entity
/// URI — first one that resolves wins.
async fn resolve_descriptor(
    source: &str,
    branch: &dialog_repository::Branch,
    operator: &crate::worker::DefaultOperator,
) -> Option<ConceptDescriptor> {
    if source.contains(':') {
        // URI form — try entity lookup directly.
        if let Ok(entity) = source.parse::<Entity>()
            && let Ok(Some(concept)) = Concept::by_entity(entity).resolve(branch, operator).await
        {
            return Some(concept.descriptor);
        }
        return None;
    }

    // Bookmark form — built-ins win, then branch lookup.
    if let Some(found) = tonk_schema::builtin::lookup_concept(source) {
        return Some(found.descriptor);
    }
    Concept::by_name(source)
        .resolve(branch, operator)
        .await
        .ok()
        .flatten()
        .map(|c| c.descriptor)
}

/// Render the wrapper HTML. Field order is alphabetical for
/// determinism.
fn render_page(
    repo: &str,
    branch: &str,
    source: &str,
    filters: &BTreeMap<String, String>,
    descriptor: &ConceptDescriptor,
) -> String {
    let mut fields: Vec<&str> = descriptor.with().keys().collect();
    fields.sort_unstable();

    let header_cells: String = fields
        .iter()
        .map(|name| format!("<th>{}</th>", html_escape(name)))
        .collect::<Vec<_>>()
        .join("");
    let row_cells: String = fields
        .iter()
        .map(|name| format!("<td>{{{}}}</td>", html_escape(name)))
        .collect::<Vec<_>>()
        .join("");

    let source_attr = build_source_attr(source, filters);

    format!(
        "<!doctype html>\n\
         <html>\n\
         <head>\n\
           <meta charset=\"utf-8\"/>\n\
           <title>{title}</title>\n\
         </head>\n\
         <body>\n\
           <h1>{title}</h1>\n\
           <tonk-concept space=\"{space}\" branch=\"{branch}\" source=\"{source_attr}\">\n\
             <table>\n\
               <thead><tr><th>this</th>{header_cells}</tr></thead>\n\
               <tbody>\n\
                 <template>\n\
                   <tr><td>{{this}}</td>{row_cells}</tr>\n\
                 </template>\n\
               </tbody>\n\
             </table>\n\
           </tonk-concept>\n\
           <script type=\"module\" src=\"/-/tonk-concept.js\"></script>\n\
         </body>\n\
         </html>\n",
        title = html_escape(source),
        space = html_escape(repo),
        branch = html_escape(branch),
        source_attr = html_escape_attr(&source_attr),
    )
}

/// Reconstruct the `source` attribute string the element parses —
/// `name?field=value&…` form. Keys/values get URL-form encoding.
fn build_source_attr(source: &str, filters: &BTreeMap<String, String>) -> String {
    if filters.is_empty() {
        return source.to_owned();
    }
    let mut query = String::new();
    let mut first = true;
    for (k, v) in filters {
        if !first {
            query.push('&');
        }
        first = false;
        query.push_str(&form_encode(k));
        query.push('=');
        query.push_str(&form_encode(v));
    }
    format!("{source}?{query}")
}

/// Minimal HTML element-text escaper — covers the five characters
/// that change parsing.
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

/// HTML attribute-value escaper — same as element-text plus we
/// escape backticks too because some old browsers treated those
/// as attribute terminators.
fn html_escape_attr(s: &str) -> String {
    html_escape(s).replace('`', "&#96;")
}

/// Form-urlencode a single value — encode anything outside the
/// unreserved set per RFC 3986.
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

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    fn it_html_escapes_special_characters() {
        assert_eq!(html_escape("<a&b>"), "&lt;a&amp;b&gt;");
        assert_eq!(html_escape("\"quoted'"), "&quot;quoted&#39;");
    }

    #[dialog_common::test]
    fn it_form_encodes_special_characters() {
        assert_eq!(form_encode("a b"), "a+b");
        assert_eq!(form_encode("a/b"), "a%2Fb");
        assert_eq!(form_encode("hello"), "hello");
    }

    #[dialog_common::test]
    fn it_builds_a_source_attribute_without_filters() {
        let attr = build_source_attr("person", &BTreeMap::new());
        assert_eq!(attr, "person");
    }

    #[dialog_common::test]
    fn it_builds_a_source_attribute_with_filters() {
        let mut filters = BTreeMap::new();
        filters.insert("name".to_string(), "Alice".to_string());
        let attr = build_source_attr("person", &filters);
        assert_eq!(attr, "person?name=Alice");
    }
}
