//! `POST /api/site` — register/update the requesting client's site.
//!
//! The site is per-tab navigation state, NOT per-query. On first load the
//! navigation predates the service worker (the page is served before the SW
//! exists), so the SW never sees a navigation `FetchEvent` for it — the page
//! must announce itself. Once controlled, the page calls `POST /api/site` with
//! its current path; the SW reads the requesting **client id**, derives the site
//! entity (`site:<client-id>`), asserts a [`Site`] `{path, anchor, replica,
//! route, concept}` on the Level-0-resolved branch's overlay, and returns the
//! site id. The page renders `<tonk-display entity={site} model=tonk:site>`; the
//! `tonk:site` view nests into the matched `{concept}` and renders.
//!
//! The same endpoint handles navigation updates: the page re-calls it on each
//! client-side navigation, and the cardinality-one fields update in place. Read
//! queries never stamp, so a tab's displays re-querying never re-derive or
//! re-poll — the perf cost of stamping is paid once per navigation, not per read.

use ::axum::Json;
use ::axum::extract::{Request, State};
use ::axum::http::HeaderMap;
use axum_wasm_macros::wasm_compat;
use serde::Serialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::{AppState, ClientId};
use crate::TonkWorkerError;

/// `POST /api/site` response: the site entity the client should render against.
#[derive(Debug, Serialize)]
pub struct SiteResponse {
    /// The site entity URI (`site:<client-id>`).
    pub site: String,
}

/// Read a header as a `&str`, empty when absent or non-ASCII.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Register the requesting client's site: assert its [`Site`] and return the
/// site id. The client id (browser-assigned, one per document) keys the site, so
/// it is GC-able (the SW can reconcile against live clients) and needs no minted
/// uuid. Idempotent — re-calling on navigation supersedes the cardinality-one
/// fields in place.
#[wasm_compat]
pub async fn register_site(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
) -> Result<Json<SiteResponse>, TonkWorkerError> {
    let client_id = request
        .extensions()
        .get::<ClientId>()
        .map(|c| c.0.clone())
        .unwrap_or_default();
    if client_id.is_empty() {
        return Err(TonkWorkerError::Router("no client id on /api/site".into()));
    }
    let site = format!("site:{client_id}");

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let path = header(&headers, "x-tonk-path").to_owned();
        let anchor = header(&headers, "x-tonk-hash").to_owned();
        let tonk = state.read().await;
        stamp_site(&tonk, &site, &path, anchor).await;
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let _ = (&state, &headers);
    }

    Ok(Json(SiteResponse { site }))
}

/// Assert the [`Site`] for `site` (a `site:<client-id>` URI) from `path` on the
/// Level-0-resolved branch's overlay: derive the replica, match the remaining
/// path against the route table, and write `{path, anchor, replica, route,
/// concept}` through the overlay builder (which schedules a poll so subscribers
/// see the change — no inline whole-branch re-poll). Best-effort: a non-space
/// path, an unacquirable branch, an absent replica, or no matched route all skip
/// stamping rather than fail registration.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn stamp_site(tonk: &crate::worker::TonkState, site: &str, path: &str, anchor: String) {
    use tonk_schema::{RouteTarget, Site, resolve_path};

    let Ok(entity): Result<dialog_artifacts::Entity, _> = site.parse() else {
        return;
    };
    // Only spaces route here; the profile (`/`, `/join`) stays special for now.
    let Some(RouteTarget::Space { space, rest }) = resolve_path(path) else {
        return;
    };

    let branch = tonk.reactor.repository(&space.name).branch(&space.branch);
    let state = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(e) => {
            tonk_common::log!("register_site: failed to acquire branch for {path}: {e}");
            return;
        }
    };

    let Some(replica) = origin_entity(tonk, &state).await else {
        return;
    };
    let Some((route, concept)) = match_route(tonk, &state, &rest).await else {
        return;
    };

    // Write through the overlay builder: it asserts into the session overlay and
    // schedules a poll so subscribers are notified — the request dispatcher
    // drains the poll once. Cardinality-one fields supersede in place, so a
    // navigation re-call just updates this site's path/route/concept.
    let stamp = Site::new(
        entity,
        path.to_owned(),
        anchor,
        space.name.clone(),
        space.branch.clone(),
        replica,
        route,
        concept,
    );
    if let Err(e) = branch
        .overlay()
        .assert(stamp)
        .write()
        .perform(&tonk.operator)
        .await
    {
        tonk_common::log!("register_site: overlay write failed for {path}: {e}");
    }
}

/// The existing dialog [`Origin`](dialog_repository::schema::Origin) entity for
/// this device's `(profile, subject)` on the branch — the entity `tonk/replica`
/// and `tonk:binder` live on. Queried (not derived) so it stays correct even if
/// tonk's and dialog's hashing drift. `None` if no origin is on the branch yet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn origin_entity(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
) -> Option<dialog_artifacts::Entity> {
    use dialog_query::{Output as _, Query, Term};
    use dialog_repository::schema::origin::{Profile, Subject};
    use dialog_repository::schema::{DidExt as _, Origin};

    let subject = state.handle().of().this();
    let profile = tonk.profile.did().this();

    let origins: Vec<Origin> = state
        .handle()
        .query()
        .select(Query::<Origin> {
            this: Term::var("this"),
            subject: Term::from(Subject(subject)),
            profile: Term::from(Profile(profile)),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    origins.into_iter().next().map(|origin| origin.this)
}

/// Match `rest` (the Level 1 remaining path) against the branch's durable
/// `tonk:route` table, returning the matched `(route entity, route model)`.
///
/// Builds a fresh `matchit::Router` per call from the queried routes. Routes are
/// inserted in stable entity-URI order so a `matchit` conflict (two structurally
/// identical patterns) resolves deterministically — the loser is skipped with a
/// log line. Returns `None` when nothing matches.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn match_route(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
    rest: &str,
) -> Option<(dialog_artifacts::Entity, dialog_artifacts::Entity)> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::Route;

    let mut routes: Vec<Route> = state
        .handle()
        .query()
        .select(Query::<Route> {
            this: Term::var("this"),
            path: Term::var("path"),
            concept: Term::var("concept"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    // Stable order by entity URI so conflict resolution is deterministic.
    routes.sort_by(|a, b| a.this.to_string().cmp(&b.this.to_string()));

    let mut router = matchit::Router::new();
    for route in &routes {
        let value = (route.this.clone(), route.concept.0.clone());
        if let Err(e) = router.insert(route.path.0.clone(), value) {
            tonk_common::log!(
                "match_route: skipping conflicting route {}: {e}",
                route.path.0
            );
        }
    }

    router.at(rest).ok().map(|matched| matched.value.clone())
}
