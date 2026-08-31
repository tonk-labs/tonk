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
    "/api",
    "/api/account",
    "/api/account/attach",
    "/api/account/delete",
    "/api/account/deletion/plan",
    "/api/account/devices",
    "/api/account/devices/register",
    "/api/account/devices/revoke",
    "/api/account/display-name",
    "/api/account/spaces/delete",
    "/api/account/summary",
    "/api/custody/provision",
    "/api/custody/queue",
    "/api/customer",
    "/api/customer/pending",
    "/api/identify",
    "/api/identity/root",
    "/api/inspect/repository/{repo}/archive/index/{hash}",
    "/api/inspect/repository/{repo}/branch/{branch}",
    "/api/inspect/repository/{repo}/remote/{remote}",
    "/api/inspect/repository/{repo}/remote/{remote}/archive/index/{hash}",
    "/api/inspect/repository/{repo}/remote/{remote}/branch/{branch}",
    "/api/migrate/repo-vs-profile",
    "/api/profile",
    "/api/profile/branch/{branch}/evaluate",
    "/api/profile/branch/{branch}/query",
    "/api/profile/branch/{branch}/site",
    "/api/profile/branch/{branch}/transact",
    "/api/profile/join",
    "/api/profile/repository",
    "/api/profiles",
    "/api/profiles/activate",
    "/api/profiles/add",
    "/api/repository/{repo}",
    "/api/repository/{repo}/branch/{branch}/blob",
    "/api/repository/{repo}/branch/{branch}/blob/{entity}",
    "/api/repository/{repo}/branch/{branch}/claim/assert/{entity}/{attr_ns}/{attr_name}",
    "/api/repository/{repo}/branch/{branch}/claim/retract/{entity}/{attr_ns}/{attr_name}",
    "/api/repository/{repo}/branch/{branch}/claim/select",
    "/api/repository/{repo}/branch/{branch}/evaluate",
    "/api/repository/{repo}/branch/{branch}/export",
    "/api/repository/{repo}/branch/{branch}/host/{host}/{entity}",
    "/api/repository/{repo}/branch/{branch}/import",
    "/api/repository/{repo}/branch/{branch}/query",
    "/api/repository/{repo}/branch/{branch}/site",
    "/api/repository/{repo}/branch/{branch}/sync",
    "/api/repository/{repo}/branch/{branch}/sync/pull",
    "/api/repository/{repo}/branch/{branch}/sync/push",
    "/api/repository/{repo}/branch/{branch}/sync/status",
    "/api/repository/{repo}/branch/{branch}/transact",
    "/api/repository/{repo}/invite",
    "/api/repository/{repo}/invites",
    "/api/repository/{repo}/invites/{target_cid}/revoke",
    "/api/repository/{repo}/remote",
    "/api/site",
    "/api/sync",
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

/// Concrete instances of every route/method pair whose handler changes
/// durable or worker-owned state. Keeping examples here makes a route-table
/// review also review the stale-page write barrier. Unknown non-read methods
/// are tested separately because they must fail safe without appearing here.
const STATE_CHANGING_ROUTES: &[(&str, &str)] = &[
    ("DELETE", "/api/account"),
    ("GET", "/api/migrate/repo-vs-profile"),
    ("HEAD", "/api/migrate/repo-vs-profile"),
    ("POST", "/api/account/attach"),
    ("POST", "/api/account/delete"),
    ("POST", "/api/account/devices/register"),
    ("POST", "/api/account/devices/revoke"),
    ("POST", "/api/account/display-name"),
    ("POST", "/api/account/spaces/delete"),
    ("POST", "/api/custody/provision"),
    ("POST", "/api/custody/queue"),
    ("POST", "/api/customer/activated"),
    ("POST", "/api/identity/root"),
    ("POST", "/api/language-server"),
    ("POST", "/api/profile/branch/main/evaluate"),
    ("POST", "/api/profile/branch/main/site"),
    ("POST", "/api/profile/branch/main/transact"),
    ("POST", "/api/profile/join"),
    ("POST", "/api/profiles/activate"),
    ("POST", "/api/profiles/add"),
    ("POST", "/api/repository/space/branch/main/blob"),
    (
        "POST",
        "/api/repository/space/branch/main/claim/assert/entity/ns/name",
    ),
    (
        "POST",
        "/api/repository/space/branch/main/claim/retract/entity/ns/name",
    ),
    ("POST", "/api/repository/space/branch/main/evaluate"),
    ("POST", "/api/repository/space/branch/main/import"),
    ("POST", "/api/repository/space/branch/main/site"),
    ("POST", "/api/repository/space/branch/main/sync"),
    ("POST", "/api/repository/space/branch/main/sync/pull"),
    ("POST", "/api/repository/space/branch/main/sync/push"),
    ("POST", "/api/repository/space/branch/main/transact"),
    ("POST", "/api/repository/space/invite"),
    ("POST", "/api/repository/space/invites/invite/revoke"),
    ("POST", "/api/repository/space/remote"),
    ("POST", "/api/site"),
    ("POST", "/api/sync"),
    ("PUT", "/api/repository/space"),
];

#[dialog_common::test]
fn it_classifies_every_declared_state_changing_route() {
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::Method;

    for &(method, uri) in STATE_CHANGING_ROUTES {
        let request = Request::builder()
            .method(Method::from_bytes(method.as_bytes()).expect("valid test method"))
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        assert!(
            super::is_mutating(&request),
            "{method} {uri} changes state and must be gated"
        );
    }
}

#[dialog_common::test]
fn it_preserves_only_explicit_read_like_posts() {
    use axum::body::Body;
    use axum::extract::Request;

    for uri in [
        "/api/profile/branch/main/query",
        "/api/repository/space/branch/main/query",
        "/api/profile/branch/main/evaluate?transact=false",
        "/api/repository/space/branch/main/evaluate?transact=false",
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        assert!(
            !super::is_mutating(&request),
            "POST {uri} is an explicit read-like operation"
        );
    }

    for uri in [
        "/api/profile/branch/main/evaluate",
        "/api/profile/branch/main/evaluate?transact=true",
        "/api/profile/branch/main/evaluate?transact=false&transact=false",
        "/api/profile/branch/main/evaluate?transact=maybe",
        "/api/profile/branch/main/evaluate?transact=0",
        "/api/profile/branch/main/evaluate?transact=no",
        "/api/profile/branch/main/evaluate?transact=False",
        "/api/profile/branch//query",
        "/api/repository//branch/main/query",
        "/api/repository/space/branch//evaluate?transact=false",
        "/api/not-a-declared-route",
        "/api/repository/space/branch/main/query/near-miss",
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        assert!(
            super::is_mutating(&request),
            "POST {uri} is not an unambiguous read exception"
        );
    }

    for method in ["PUT", "PATCH", "DELETE"] {
        let request = Request::builder()
            .method(method)
            .uri("/api/not-a-declared-route")
            .body(Body::empty())
            .unwrap();
        assert!(
            super::is_mutating(&request),
            "unknown {method} requests must fail safe"
        );
    }
}
