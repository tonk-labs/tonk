//! `/space/:space/branch/:branch/display/:subject` route.
//!
//! Mounts a `<tonk-display>` element after resolving `:subject` to
//! a concrete entity URI. The route accepts either form:
//!
//! - **Entity URI** — anything containing a `:` (e.g. a `did:key:…`
//!   or `concept:…` URI). Used verbatim as the element's `entity`
//!   attribute. No lookup happens.
//!
//! - **Name** — a bare bookmark (e.g. `demo`). The route queries the
//!   branch for a `Name` row whose `this` equals `id:<subject>` and
//!   reads its `entity` field. The resolved URI becomes the
//!   element's `entity` attribute. Missing name ⇒ a 404 section is
//!   rendered and the element is never mounted.
//!
//! Query parameters carry the optional view/model selection:
//!
//! - `?view=<name>` — display name, forwarded as the element's
//!   `view` attribute. If omitted, the element falls back to its
//!   built-in generic rendering (one `<dl>` row per concept field).
//! - `?model=<concept>` — concept name or URI, forwarded as the
//!   element's `model` attribute. If omitted, the element queries
//!   every attribute on the entity instead of going through a
//!   concept descriptor.
//!
//! Doing name resolution at the route (rather than inside
//! `<tonk-display>`) keeps the element decoupled from the routing
//! layer's concept of "the URL says X, look up X in the branch
//! first." That separation also gives us a place to surface a 404
//! when the bookmark doesn't resolve, which the element — designed
//! to subscribe live, not to halt on missing data — couldn't
//! express cleanly.

use leptos::prelude::*;
use leptos_router::hooks::{use_params, use_query_map};
use leptos_router::params::Params;
use reqwest::StatusCode;
use serde_json::json;

use crate::api;
use crate::error::TonkUiError;

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkDisplayParams {
    space: Option<String>,
    branch: Option<String>,
    subject: Option<String>,
}

/// Single-entity display route. Resolves the path's `:subject`
/// (either an entity URI or a bookmark name) to an entity URI,
/// then mounts a `<tonk-display>` element with the resolved URI
/// plus any `?view=` / `?model=` query parameters as attributes.
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkDisplayView() -> impl IntoView {
    let params = use_params::<TonkDisplayParams>();
    let query_map = use_query_map();

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
    let subject = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.subject)
            .filter(|s| !s.is_empty())
    });
    let view_name = Signal::derive_local(move || {
        query_map
            .get()
            .get("view")
            .as_deref()
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
    });
    let model_name = Signal::derive_local(move || {
        query_map
            .get()
            .get("model")
            .as_deref()
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
    });

    // Resolve `:subject` → entity URI. URIs pass through; names hit
    // the worker via a `Name` query. The Suspense below waits on
    // the resolve before rendering anything substantive.
    let resolved_entity = LocalResource::new(move || {
        let space = space_name.get();
        let branch = branch_name.get();
        let subject = subject.get();
        async move {
            let (Some(space), Some(branch), Some(subject)) = (space, branch, subject) else {
                return Ok::<Option<String>, TonkUiError>(None);
            };
            if looks_like_uri(&subject) {
                return Ok(Some(subject));
            }
            resolve_name(&space, &branch, &subject).await
        }
    });

    // The mount node + a single Effect that reads every signal it
    // cares about. When any of (entity-resolution, view, model)
    // updates, the effect re-runs and rebuilds the `<tonk-display>`
    // host with current attribute values. Creating an Effect inside
    // `view!` (the previous shape) racey-mounted multiple hosts as
    // signals settled at different times.
    let mount: NodeRef<leptos::html::Div> = NodeRef::new();
    Effect::new(move |_| {
        let Some(slot) = mount.get() else {
            return;
        };
        // Wait for the entity resolution to land before mounting.
        let Some(result) = resolved_entity.get() else {
            return;
        };
        let document = leptos::prelude::document();
        slot.set_inner_html("");
        match result {
            Ok(Some(uri)) => {
                let host = match document.create_element("tonk-display") {
                    Ok(el) => el,
                    Err(_) => return,
                };
                let space = space_name.get().unwrap_or_default();
                let branch = branch_name.get().unwrap_or_default();
                let _ = host.set_attribute("space", &space);
                let _ = host.set_attribute("branch", &branch);
                let _ = host.set_attribute("entity", &uri);
                if let Some(v) = view_name.get() {
                    let _ = host.set_attribute("view", &v);
                }
                if let Some(m) = model_name.get() {
                    let _ = host.set_attribute("model", &m);
                }
                let _ = slot.append_child(&host);
            }
            Ok(None) => {
                // Unresolved bookmark — render a 404 inline.
                if let Ok(section) = document.create_element("section") {
                    let _ = section.set_attribute("class", "not-found");
                    let label = format!("No entity is named {}", subject.get().unwrap_or_default());
                    section.set_text_content(Some(&label));
                    let _ = slot.append_child(&section);
                }
            }
            Err(_) => {
                // Resolution error — let the ErrorBoundary surface
                // it. The Effect runs after resource resolution, so
                // an error here is shown by re-reading the resource
                // in the ErrorBoundary branch below.
            }
        }
    });

    view! {
        <header slot="main-header" class="space-banner">
            <h1 class="space-banner-title" title=move || subject.get().unwrap_or_default()>
                { move || subject.get().unwrap_or_default() }
            </h1>
        </header>
        <Suspense fallback=|| view! { <wa-spinner></wa-spinner> }>
            <ErrorBoundary fallback=|errors| view! {
                <wa-callout variant="danger">
                    <wa-icon slot="icon" name="circle-exclamation"></wa-icon>
                    { move || errors.get().into_iter().map(|(_, e)| format!("{e}")).collect::<Vec<_>>().join(", ") }
                </wa-callout>
            }>
                { move || resolved_entity.get().map(|result| result.map(|_| ())) }
                <main class="wa-stack display-view">
                    <div class="display-view-slot" node_ref=mount></div>
                </main>
            </ErrorBoundary>
        </Suspense>
    }
}

/// True if `s` looks like an entity URI (contains a `:`) rather
/// than a bookmark name.
fn looks_like_uri(s: &str) -> bool {
    s.contains(':')
}

/// Resolve a bookmark name to its target entity URI via the
/// branch's `Name` index.
///
/// `Name` is a built-in concept (see `tonk_schema::meta::Name`)
/// whose `this` is an `id:<name>` entity and whose `entity` field
/// (backed by `dialog.name/referent`) points at the target. To
/// resolve `demo`, we query for the `Name` row with
/// `this = id:demo` and read its `entity`.
async fn resolve_name(
    space: &str,
    branch: &str,
    name: &str,
) -> Result<Option<String>, TonkUiError> {
    let id_uri = format!("id:{name}");
    let body = json!({
        "terms": {
            "this":   id_uri,
            "entity": { "?": { "name": "entity" } },
        },
        "predicate": {
            "with": {
                "entity": {
                    "the": "dialog.name/referent",
                    "as": "Entity",
                    "cardinality": "one",
                }
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
        .map_err(|e| TonkUiError::ApiError(format!("name resolve fetch: {e}")))?;
    if response.status() != StatusCode::OK {
        return Err(TonkUiError::ApiError(format!(
            "name resolve returned {}",
            response.status()
        )));
    }
    let conclusions: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| TonkUiError::ApiError(format!("name resolve parse: {e}")))?;
    let Some(first) = conclusions.into_iter().next() else {
        return Ok(None);
    };
    let entity = first
        .get("fields")
        .and_then(|f| f.get("entity"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Ok(entity)
}
