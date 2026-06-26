//! `/space/:space` and `/space/:space/*subject` routes.
//!
//! `:space` is the `{branch}@{name}` segment (branch defaults to
//! `main`); `*subject` is a wildcard (not a single `:segment`) so
//! entity URIs containing `/` — e.g. `id:tonk-workspace/itinerary` —
//! are captured whole instead of being truncated at the first slash.
//!
//! Mounts a `<tonk-display>` element. With no `subject` the space's
//! default model ([`SPACE_CONCEPT`]) is shown. Otherwise the segment
//! encodes up to three attributes with `@` and `!` delimiters:
//!
//! - `{model}` — directory mode: only `model` is set, so the element
//!   renders every instance of the model (e.g. `trip`).
//! - `{entity}@{model}` — `entity` + `model` (e.g.
//!   `id:x@trip`).
//! - `{entity}@{model}!{view}` — all three (e.g.
//!   `id:x@trip!tonk:view`).
//!
//! The entity part resolves to a concrete URI before mounting:
//!
//! - **Entity URI** — anything containing a `:` (e.g. a `did:key:…`
//!   or `id:…` URI). Used verbatim. No lookup happens.
//! - **Name** — a bare bookmark (e.g. `demo`). The route queries the
//!   branch for a `Name` row whose `this` equals `id:<name>` and
//!   reads its `entity` field. Missing name ⇒ a 404 section is
//!   rendered and the element is never mounted. (Directory mode
//!   supplies no entity, so there is nothing to resolve.)
//!
//! The model and view parts pass through verbatim as the `model` /
//! `view` attributes.
//!
//! Doing name resolution at the route (rather than inside
//! `<tonk-display>`) keeps the element decoupled from the routing
//! layer's concept of "the URL says X, look up X in the branch
//! first." That separation also gives us a place to surface a 404
//! when the bookmark doesn't resolve, which the element — designed
//! to subscribe live, not to halt on missing data — couldn't
//! express cleanly.

use leptos::prelude::*;
use leptos_router::hooks::use_params;
use leptos_router::params::Params;
use reqwest::StatusCode;
use serde_json::json;

use crate::api;
use crate::error::TonkUiError;
use tonk_schema::parse_space;

/// The model a bare `/space/{name}/` renders — the space's primary
/// interface. A user can override it per repository; it presets to the
/// workspace concept from the wireframes.
const SPACE_CONCEPT: &str = "tonk/space";

#[derive(Params, PartialEq, Clone, Debug)]
pub struct TonkDisplayParams {
    space: Option<String>,
    subject: Option<String>,
}

/// Display route. Parses the path's `subject` segment into
/// `entity`/`model`/`view` (see the module docs), resolves the
/// optional entity to a URI, and mounts a `<tonk-display>` with those
/// attributes. With no entity it is directory mode (only `model`).
#[component]
#[allow(clippy::unused_unit)]
pub fn TonkDisplayView() -> impl IntoView {
    let params = use_params::<TonkDisplayParams>();

    // `:space` is `{branch}@{name}` (branch defaults to `main`).
    let space_ref = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.space)
            .filter(|s| !s.is_empty())
            .and_then(|s| parse_space(&s))
    });
    let space_name = Signal::derive_local(move || space_ref.get().map(|s| s.name));
    let branch_name = Signal::derive_local(move || space_ref.get().map(|s| s.branch));
    // The `*subject` segment encodes up to three attributes:
    //   `{model}`                 → directory mode: only `model`.
    //   `{entity}@{model}`        → `entity` + `model`.
    //   `{entity}@{model}!{view}` → `entity` + `model` + `view`.
    // `@` separates the (optional) entity from the model; `!`
    // separates the (optional) view. The entity may be a bookmark
    // name (resolved below); the model/view pass through verbatim.
    // Absent (`/space/{name}/`) → the space's default model.
    let segment = Signal::derive_local(move || {
        params
            .get()
            .ok()
            .and_then(|p| p.subject)
            .filter(|s| !s.is_empty())
    });
    let parsed = Signal::derive_local(move || match segment.get() {
        Some(s) => parse_subject(&s),
        None => Subject {
            entity: None,
            concept: SPACE_CONCEPT.to_owned(),
            view: None,
        },
    });
    let entity_name = Signal::derive_local(move || parsed.get().entity.filter(|s| !s.is_empty()));
    let concept_name =
        Signal::derive_local(move || Some(parsed.get().concept).filter(|s| !s.is_empty()));
    let view_name = Signal::derive_local(move || parsed.get().view.filter(|s| !s.is_empty()));

    // Run background sync for the space while the workspace is shown —
    // a local write reaches the remote (and remote changes reach this
    // tab) without anyone clicking Pull/Push, and the top-bar sync chip
    // can track remote drift. Registered under this component's owner,
    // so it tears down when the route unmounts.
    crate::sync_controller::mount(space_name);

    // Resolve the entity part (if present) → entity URI. URIs pass
    // through; names hit the worker via a `Name` query. In directory
    // mode (no `@`, so no entity) this resolves to `None` and the
    // element mounts with only `model`. The Suspense below waits on
    // the resolve before rendering anything substantive.
    let resolved_entity = LocalResource::new(move || {
        let space = space_name.get();
        let branch = branch_name.get();
        let entity = entity_name.get();
        async move {
            let Some(entity) = entity else {
                // Directory mode: no entity to resolve.
                return Ok::<Option<String>, TonkUiError>(None);
            };
            let (Some(space), Some(branch)) = (space, branch) else {
                return Ok(None);
            };
            if looks_like_uri(&entity) {
                return Ok(Some(entity));
            }
            resolve_name(&space, &branch, &entity).await
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
        // An entity name was given in the path but did not resolve →
        // 404. (Directory mode supplies no entity name, so `None` there
        // is expected, not a miss.)
        let unresolved = entity_name.get().is_some() && matches!(result, Ok(None));
        match result {
            Ok(uri) if !unresolved => {
                let host = match document.create_element("tonk-display") {
                    Ok(el) => el,
                    Err(_) => return,
                };
                // Space and branch come from the surrounding
                // <tonk-repository> / <tonk-branch> ancestors via
                // event-annotation; no attributes needed on the
                // element itself. `entity` is set only when present
                // (absent in directory mode).
                if let Some(uri) = uri {
                    let _ = host.set_attribute("entity", &uri);
                }
                if let Some(m) = concept_name.get() {
                    let _ = host.set_attribute("concept", &m);
                }
                if let Some(v) = view_name.get() {
                    let _ = host.set_attribute("view", &v);
                }
                let _ = slot.append_child(&host);
            }
            Ok(_) => {
                // Unresolved bookmark — render a 404 inline.
                if let Ok(section) = document.create_element("section") {
                    let _ = section.set_attribute("class", "not-found");
                    let label = format!(
                        "No entity is named {}",
                        entity_name.get().unwrap_or_default()
                    );
                    section.set_text_content(Some(&label));
                    let _ = slot.append_child(&section);
                }
            }
            Err(error) => {
                // Resolution failed — surface it inline rather than
                // swallowing it (the bare route has no ErrorBoundary).
                if let Ok(section) = document.create_element("section") {
                    let _ = section.set_attribute("class", "error");
                    section.set_text_content(Some(&format!("{error}")));
                    let _ = slot.append_child(&section);
                }
            }
        }
    });

    // Bare render: just the routing-context chain and the
    // `<tonk-display>` mount, no shell chrome (no banner, no
    // `<wa-page>`, no toolbar). The document's global stylesheet still
    // loads, so the rendered view is styled; this route is the page.
    view! {
        <tonk-repository class="display-route" name=move || space_name.get().unwrap_or_default()>
            <tonk-branch name=move || branch_name.get().unwrap_or_default()>
                <div class="display-view-slot" node_ref=mount></div>
            </tonk-branch>
        </tonk-repository>
    }
}

/// The `<tonk-display>` attributes encoded in a display-route path
/// segment. `model` is always present; `entity` and `view` are
/// optional.
#[derive(Clone)]
struct Subject {
    entity: Option<String>,
    concept: String,
    view: Option<String>,
}

/// Parse a display-route path segment into its attributes. Grammar:
///
/// - `{model}`                 → `entity: None, model, view: None`
/// - `{entity}@{model}`        → `entity, model, view: None`
/// - `{entity}@{model}!{view}` → `entity, model, view`
///
/// `@` separates an optional entity from the model; `!` separates an
/// optional view. Entity URIs / model / view names may contain `:`
/// and `/`, but not `@` or `!`, so the split is unambiguous. The view
/// is split off first so a `!` after the model is honored even when
/// there is no `@`.
fn parse_subject(segment: &str) -> Subject {
    let (head, view) = match segment.split_once('!') {
        Some((head, view)) => (head, Some(view.to_owned())),
        None => (segment, None),
    };
    let (entity, concept) = match head.split_once('@') {
        Some((entity, concept)) => (Some(entity.to_owned()), concept.to_owned()),
        None => (None, head.to_owned()),
    };
    Subject {
        entity,
        concept,
        view,
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
