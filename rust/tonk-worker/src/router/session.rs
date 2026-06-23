//! Per-tab site: stamp the [`Site`] facts a request carries onto its site
//! entity, in the Level-0-resolved branch's overlay.
//!
//! The host (top page or sealed guest) stamps three things on every host-relative
//! `/api` request: the document path (`X-Tonk-Path`), the fragment
//! (`X-Tonk-Hash`), and the per-tab site id (`X-Tonk-Site`). The path is an
//! explicit header rather than `Referer` because a service worker reads
//! `request.headers`, which never includes `Referer` (the browser exposes it
//! only as the separate `request.referrer` property). Level 0 resolves the path
//! to a `(repo, branch)` target and the remaining (Level 1) path.
//!
//! For a space, the SW derives the tab's replica, matches the remaining path
//! against the branch's durable `tonk:route` table with `matchit`, and asserts a
//! [`Site`] `{path, anchor, replica, route, concept}` onto the site entity in
//! that branch's overlay (the `state:here` pattern keyed per tab). The shell
//! mounts `<tonk-display entity=site:… model={concept}>`; the route model
//! resolves on the site entity and its view renders. Multiple tabs coexist as
//! distinct site entities in one overlay.

use ::axum::http::HeaderMap;

use tonk_schema::{RouteTarget, resolve_path};

/// Read a header as a `&str`, empty when absent or non-ASCII.
fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// Stamp the request's [`Site`] onto its site entity in the Level-0-resolved
/// branch's overlay.
///
/// Best-effort and silent on every miss: no site id, an unparseable id, an
/// unresolvable/non-space path, an unacquirable branch, or no matched route all
/// skip stamping rather than fail the request — a miss only means a tab's site
/// is absent, never a broken request. The profile (`/`, `/join`) is kept special
/// and not routed here yet.
pub async fn stamp_site(tonk: &crate::worker::TonkState, headers: &HeaderMap) {
    use std::sync::Arc;
    use tonk_schema::Site;

    let site = header(headers, "x-tonk-site");
    if site.is_empty() {
        return;
    }
    let Ok(entity): Result<dialog_artifacts::Entity, _> = site.parse() else {
        return;
    };

    let path = header(headers, "x-tonk-path");
    // Only spaces route here; the profile (`/`, `/join`) stays special for now.
    let Some(RouteTarget::Space { space, rest }) = resolve_path(path) else {
        return;
    };
    let anchor = header(headers, "x-tonk-hash").to_owned();

    let branch = tonk.reactor.repository(&space.name).branch(&space.branch);
    let state = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(e) => {
            tonk_common::log!("stamp_site: failed to acquire branch for {path}: {e}");
            return;
        }
    };

    // The tab's replica entity: the existing dialog `Origin` for this
    // (profile, subject), QUERIED rather than re-derived — `tonk/replica` /
    // `tonk:binder` live on it, and re-deriving risks drift (tonk's `Replica`
    // and dialog's `Origin` no longer hash to the same entity). Skip stamping if
    // it isn't on the branch yet.
    let Some(replica) = origin_entity(tonk, &state).await else {
        return;
    };

    // Match the remaining (Level 1) path against the space's route table; the
    // matched route entry carries the route model to mount.
    let Some((route, concept)) = match_route(tonk, &state, &rest).await else {
        return;
    };

    // All fields are cardinality one, so asserting supersedes the prior values
    // on this site entity rather than accumulating — a navigation re-stamp
    // leaves exactly the latest location + route.
    let stamp = Site::new(entity, path.to_owned(), anchor, replica, route, concept);
    state.state.assert_overlay(stamp);

    tonk.reactor.schedule_poll(Arc::clone(&state.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// The existing dialog [`Origin`](dialog_repository::schema::Origin) entity for
/// this device's `(profile, subject)` on the branch — the entity `tonk/replica`
/// and `tonk:binder` live on. Queried (not derived) so it stays correct even if
/// tonk's and dialog's hashing drift. `None` if no origin is on the branch yet.
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
/// Builds a fresh `matchit::Router` per request from the queried routes. Routes
/// are inserted in stable entity-URI order so a `matchit` conflict (two
/// structurally identical patterns) resolves deterministically — the loser is
/// skipped with a log line. Returns `None` when nothing matches.
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
        // The value is the (route entity, route model) the SW stamps on the site.
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
