//! The worker's HTTP route table, pinned.
//!
//! Every path registered in `router.rs` is listed here, and the test
//! fails when the two disagree. The point is not the list; it is that
//! adding a route becomes a deliberate edit to this file, reviewed as
//! such, rather than a drive-by `.route(...)`. Most things that feel
//! like they need an endpoint are commands: a transient concept the page
//! asserts, a handler the worker registers, and outcomes that land as
//! facts the page already subscribes to. See
//! `.claude/skills/commands-not-routes/SKILL.md` before adding a line.

/// Every route the worker serves, sorted. The data plane (branch query,
/// transact, evaluate, blob, sync) belongs here; account, membership,
/// invite, and custody operations are commands or are on their way to
/// becoming ones.
const ROUTES: &[&str] = &[
    "/api/account",
    "/api/account/connections",
    "/api/account/connections/{id}/revoke",
    "/api/account/display-name",
    "/api/identify",
    "/api/identity/root",
    "/api/local-space-link/approve",
    "/api/local-space-link/complete",
    "/api/local-space-link/describe",
    "/api/local-space-link/provision",
    "/api/profile",
    "/api/profile/branch/{branch}/evaluate",
    "/api/profile/branch/{branch}/query",
    "/api/profile/branch/{branch}/transact",
    "/api/profile/library",
    // Bootstrap must choose the initial route before the page mounts a site.
    "/api/profile/welcome",
    "/api/repository/{repo}/branch/{branch}/blob",
    "/api/repository/{repo}/branch/{branch}/blob/{entity}",
    "/api/repository/{repo}/branch/{branch}/evaluate",
    // Device-local continuation of the bundled snapshot import: hydrates fixed
    // data/schema bytes before the selected page can resolve its model. This
    // is data-plane import, not a user-authored operation or command.
    "/api/repository/{repo}/branch/{branch}/onboarding",
    "/api/repository/{repo}/branch/{branch}/query",
    "/api/repository/{repo}/branch/{branch}/sync/status",
    "/api/repository/{repo}/branch/{branch}/transact",
];

/// Every path literal passed to `.route(` in `router.rs`, sorted.
///
/// Read from the source rather than the built `Router` because axum
/// does not expose its route table. A source scan is enough: routes are
/// only ever registered through `.route("literal", ...)`, and the test
/// below fails loudly if that stops being true.
fn registered_routes() -> Vec<String> {
    const SOURCE: &str = include_str!("../router.rs");
    const CALL: &str = ".route(";
    let mut routes = Vec::new();
    let mut rest = SOURCE;
    while let Some(at) = rest.find(CALL) {
        rest = &rest[at + CALL.len()..];
        let literal = rest.trim_start();
        let Some(literal) = literal.strip_prefix('"') else {
            panic!(
                "every `.route(` in router.rs must take a string literal path; found `{}`",
                literal.chars().take(40).collect::<String>()
            );
        };
        let end = literal.find('"').expect("an unterminated string literal");
        routes.push(literal[..end].to_owned());
    }
    routes.sort_unstable();
    routes
}

#[dialog_common::test]
fn it_adds_no_http_routes_without_editing_the_pinned_table() {
    let registered = registered_routes();
    let pinned: Vec<String> = ROUTES.iter().map(|route| (*route).to_owned()).collect();
    let added: Vec<&String> = registered
        .iter()
        .filter(|route| !pinned.contains(route))
        .collect();
    let removed: Vec<&String> = pinned
        .iter()
        .filter(|route| !registered.contains(route))
        .collect();
    assert!(
        added.is_empty() && removed.is_empty(),
        "the worker's HTTP route table changed.\n\
         added: {added:?}\n\
         removed: {removed:?}\n\n\
         A new route is almost always the wrong shape: define a command instead \
         (a transient concept in tonk-schema, a handler registered in \
         router/command.rs, outcomes as facts the page subscribes to). See \
         .claude/skills/commands-not-routes/SKILL.md. If this really is data \
         plane, update ROUTES in router/route_table.rs in the same change and \
         say why in the PR.",
    );
}

#[dialog_common::test]
fn it_pins_a_sorted_deduplicated_table() {
    let mut sorted = ROUTES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ROUTES, sorted.as_slice(), "keep ROUTES sorted and unique");
}
