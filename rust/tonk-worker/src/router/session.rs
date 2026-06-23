//! Per-tab request context: stamp the [`HostContext`] facts a request carries
//! onto its host-id entity, in the Level-0-resolved branch's overlay.
//!
//! The host (top page or sealed guest) stamps three things on every host-relative
//! `/api` request: the document path (via `Referer`), the fragment (via
//! `X-Tonk-Hash`), and a per-tab id (via `X-Tonk-Session`). Level 0 resolves the
//! document path to a `(repo, branch)` target; the SW then asserts the path/hash
//! onto the host-id entity in that branch's overlay, exactly the `state:here`
//! pattern the sync chip uses (see [`super::sync`]) but keyed per tab instead of
//! a singleton. Multiple tabs coexist as distinct entities in one overlay.
//!
//! For a space, the SW also matches the remaining (Level 1) path against the
//! branch's durable `router/route` table with `matchit` and stamps the matched
//! page model as `router/active` on the same entity, so the shell can render
//! through that indirection. Production still uses the Leptos router (Stage 4
//! switches the shell); this stands the data-driven path up alongside it.

use ::axum::http::HeaderMap;

use tonk_schema::{RouteTarget, resolve_path};

/// Name of the meta branch on the profile repository — mirrors the private copy
/// in [`super::profile`] / [`super::repository`]. Keeping a local copy rather
/// than exporting one avoids a cross-module coupling for a one-character string.
const META_BRANCH: &str = "meta";

/// Read a header as a `&str`, empty when absent or non-ASCII.
fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// The pathname of a `Referer` URL. `Referer` is an absolute URL
/// (`https://host/space/x?q#f`); Level 0 only cares about the path, so strip the
/// scheme/authority and any query/fragment. Returns `""` when there is no
/// `Referer` (a hard navigation carries none) — the caller treats that as "no
/// document to resolve" and skips stamping.
fn referer_pathname(referer: &str) -> &str {
    if referer.is_empty() {
        return "";
    }
    // Drop the scheme + authority: everything up to the first `/` after `//`.
    let after_scheme = referer
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(referer);
    let path = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "/",
    };
    // Drop a query and/or fragment.
    path.split(['?', '#']).next().unwrap_or(path)
}

/// Stamp the request's [`HostContext`] (and, for a space, the matched
/// [`RouterActive`]) onto its host-id entity in the Level-0-resolved branch's
/// overlay.
///
/// Best-effort and silent on every miss: no host-id, an unparseable id, no
/// resolvable document path, or an unacquirable branch all skip stamping rather
/// than fail the request. Nothing in production routes on these facts yet
/// (Stage 3 stands the path up alongside the Leptos router), so a miss only
/// means a tab's context is absent, never a broken request.
pub async fn stamp_host_context(tonk: &crate::worker::TonkState, headers: &HeaderMap) {
    use std::sync::Arc;
    use tonk_schema::HostContext;

    let session = header(headers, "x-tonk-session");
    if session.is_empty() {
        return;
    }
    let Ok(entity): Result<dialog_artifacts::Entity, _> = session.parse() else {
        return;
    };

    let path = referer_pathname(header(headers, "referer"));
    let Some(target) = resolve_path(path) else {
        return;
    };
    let hash = header(headers, "x-tonk-hash").to_owned();

    let branch = match &target {
        RouteTarget::Space { space, .. } => {
            tonk.reactor.repository(&space.name).branch(&space.branch)
        }
        RouteTarget::Profile => tonk.reactor.profile_repository().branch(META_BRANCH),
    };

    let state = match branch.acquire(&tonk.operator).await {
        Ok(session) => session,
        Err(e) => {
            tonk_common::log!("stamp_host_context: failed to acquire branch for {path}: {e}");
            return;
        }
    };

    // `path`/`hash` are cardinality-one attributes, so asserting supersedes the
    // prior values on this host-id entity rather than accumulating — a
    // navigation re-stamp leaves exactly the latest location.
    let stamp = HostContext::new(entity.clone(), path.to_owned(), hash);
    state.state.assert_overlay(stamp);

    // Level 1: match the remaining path against the space's route table and
    // stamp the matched page model so the shell can render through the
    // `router/active` indirection. The profile has no Level 1 routes yet.
    if let RouteTarget::Space { rest, .. } = &target
        && let Some(model) = match_route(tonk, &state, rest).await
    {
        state
            .state
            .assert_overlay(tonk_schema::RouterActive::new(entity, model));
    }

    tonk.reactor.schedule_poll(Arc::clone(&state.state));
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
}

/// Match `rest` (the Level 1 remaining path) against the branch's durable
/// `router/route` table, returning the matched page model entity.
///
/// Builds a fresh `matchit::Router` per request from the queried routes. Routes
/// are inserted in stable entity-URI order so a `matchit` conflict (two
/// structurally identical patterns) resolves deterministically — the loser is
/// skipped with a log line (Stage 3 defers the queryable `router/conflict`
/// fact). Returns `None` when nothing matches.
async fn match_route(
    tonk: &crate::worker::TonkState,
    state: &dialog_reactor::BranchSession,
    rest: &str,
) -> Option<dialog_artifacts::Entity> {
    use dialog_query::{Output as _, Query, Term};
    use tonk_schema::RouterRoute;

    let mut routes: Vec<RouterRoute> = state
        .handle()
        .query()
        .select(Query::<RouterRoute> {
            this: Term::var("this"),
            path: Term::var("path"),
            model: Term::var("model"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .unwrap_or_default();

    // Stable order by entity URI so conflict resolution is deterministic.
    routes.sort_by(|a, b| a.this.to_string().cmp(&b.this.to_string()));

    let mut router = matchit::Router::new();
    for route in &routes {
        if let Err(e) = router.insert(route.path.0.clone(), route.model.0.clone()) {
            tonk_common::log!(
                "match_route: skipping conflicting route {}: {e}",
                route.path.0
            );
        }
    }

    router.at(rest).ok().map(|matched| matched.value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_extracts_the_pathname_from_a_referer_url() {
        assert_eq!(
            referer_pathname("https://hub.tonk.xyz/space/home:z6Mk"),
            "/space/home:z6Mk",
        );
    }

    #[dialog_common::test]
    async fn it_drops_query_and_fragment_from_a_referer() {
        assert_eq!(
            referer_pathname("https://hub.tonk.xyz/space/home?q=1#frag"),
            "/space/home",
        );
    }

    #[dialog_common::test]
    async fn it_yields_root_for_a_bare_origin_referer() {
        assert_eq!(referer_pathname("https://hub.tonk.xyz"), "/");
    }

    #[dialog_common::test]
    async fn it_yields_empty_for_a_missing_referer() {
        assert_eq!(referer_pathname(""), "");
    }
}
