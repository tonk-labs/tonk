//! The routing table: specificity ordering (static > param > catch-all),
//! order-independence, disambiguation by param type, and furthest-progress
//! errors.

use tonk_router::{NoMatch, Route, Router, Type, ValueType};
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

/// `as: unsigned` stand-in: all-ASCII-digits.
#[derive(Debug)]
struct Unsigned;
impl ValueType for Unsigned {
    fn name(&self) -> &str {
        "unsigned"
    }
    fn validate(&self, value: &str) -> bool {
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
    }
}

/// `as: entity` stand-in: contains a `:`.
#[derive(Debug)]
struct EntityUri;
impl ValueType for EntityUri {
    fn name(&self) -> &str {
        "entity"
    }
    fn validate(&self, value: &str) -> bool {
        value.contains(':')
    }
}

fn route(pattern: &str) -> Route {
    Route::parse_pattern(pattern).expect("pattern")
}

#[dialog_common::test]
async fn it_matches_the_single_route() {
    let router: Router<&str> = [(route("/space/{space}/{model}"), "directory")]
        .into_iter()
        .collect();
    let matched = router.recognize("/space/home/inspector").expect("match");
    assert_eq!(*matched.value, "directory");
    assert_eq!(matched.params.get("model"), Some("inspector"));
}

#[dialog_common::test]
async fn it_prefers_a_static_route_over_a_param_route() {
    // `/board` (all literal) must win over `/{model}` regardless of order.
    let router: Router<&str> = [
        (route("/space/{space}/{model}"), "directory"),
        (route("/space/{space}/board"), "board"),
    ]
    .into_iter()
    .collect();
    let matched = router.recognize("/space/home/board").expect("match");
    assert_eq!(
        *matched.value, "board",
        "the static /board route should win over /{{model}}",
    );
}

#[dialog_common::test]
async fn it_is_insertion_order_independent() {
    // Same two routes, opposite insertion order — same winner.
    let a: Router<&str> = [
        (route("/space/{space}/board"), "board"),
        (route("/space/{space}/{model}"), "directory"),
    ]
    .into_iter()
    .collect();
    let b: Router<&str> = [
        (route("/space/{space}/{model}"), "directory"),
        (route("/space/{space}/board"), "board"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        *a.recognize("/space/home/board").unwrap().value,
        *b.recognize("/space/home/board").unwrap().value,
    );
}

#[dialog_common::test]
async fn it_prefers_a_segment_route_over_a_catch_all() {
    // `/{model}` (Segment) beats `/{tail:rest}` (catch-all) for a single segment.
    let router: Router<&str> = [
        (route("/space/{space}/{tail:rest}"), "rest"),
        (route("/space/{space}/{model}"), "directory"),
    ]
    .into_iter()
    .collect();
    let matched = router.recognize("/space/home/inspector").expect("match");
    assert_eq!(*matched.value, "directory");
    // But a multi-segment tail only the catch-all can take falls through to it.
    let matched = router.recognize("/space/home/a/b/c").expect("match");
    assert_eq!(*matched.value, "rest");
    assert_eq!(matched.params.get("tail"), Some("a/b/c"));
}

#[dialog_common::test]
async fn it_disambiguates_overlapping_routes_by_param_type() {
    // Two structurally identical routes; only param type differs. The table tries
    // both and the type decides — this is the `/space/{s}/{page}` (unsigned) vs
    // `/space/{s}/{model}` (entity) case the design targets.
    let router: Router<&str> = [
        (
            route("/space/{space}/{page}")
                .with_types(|n| (n == "page").then(|| Type::new(Unsigned))),
            "page",
        ),
        (
            route("/space/{space}/{model}")
                .with_types(|n| (n == "model").then(|| Type::new(EntityUri))),
            "model",
        ),
    ]
    .into_iter()
    .collect();

    assert_eq!(*router.recognize("/space/home/42").unwrap().value, "page");
    assert_eq!(
        *router.recognize("/space/home/tonk:person").unwrap().value,
        "model",
    );
}

#[dialog_common::test]
async fn it_reports_empty_for_an_empty_table() {
    let router: Router<&str> = Router::new();
    assert_eq!(router.recognize("/anything"), Err(NoMatch::Empty));
}

#[dialog_common::test]
async fn it_reports_no_route_with_a_furthest_error() {
    let router: Router<&str> = [(route("/space/{space}/board"), "board")]
        .into_iter()
        .collect();
    match router.recognize("/space/home/other") {
        Err(NoMatch::NoRoute { furthest }) => {
            // It matched the `/space/` prefix and the `home` segment before
            // failing to find `/board` — the failure is at the `/other` tail.
            assert!(furthest.at() >= "/space/home".len());
        }
        other => panic!("expected NoRoute, got {other:?}"),
    }
}
