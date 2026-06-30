//! Param value types participate in matching: a captured value must pass its
//! type, so two structurally identical routes are told apart by param type — a
//! `{page}` typed `unsigned` vs a `{model}` typed `entity`. Types are injected by
//! the binding layer (`Route::with_types`), never written in the pattern.

use tonk_router::{Kind, ParseError, Route, Term, Type, ValueType};
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

// Types are NOT written in the pattern — they come from the route model field and
// are injected by the binding layer via `Route::with_types`. These stand-in
// validators play the role of the model's `as: unsigned` / `as: entity` fields.

/// A stand-in for the binding layer's `as: unsigned` validator: accepts only
/// all-ASCII-digit values. (The real one comes from the route model's field
/// descriptor; the engine only ever calls `validate`.)
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

/// A stand-in for `as: entity`: accepts only values containing a `:` (a URI).
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

/// `/space/{space}/{page}` with `page` typed `unsigned` by the binding layer —
/// the pattern carries no type; `with_types` injects it.
fn page_route() -> Route {
    Route::parse_pattern("/space/{space}/{page}")
        .expect("pattern")
        .with_types(|name| (name == "page").then(|| Type::new(Unsigned)))
}

/// `/space/{space}/{model}` with `model` typed `entity` by the binding layer.
fn model_route() -> Route {
    Route::parse_pattern("/space/{space}/{model}")
        .expect("pattern")
        .with_types(|name| (name == "model").then(|| Type::new(EntityUri)))
}

#[dialog_common::test]
async fn it_accepts_a_value_matching_the_type() {
    assert!(page_route().parse("/space/home/42").is_ok());
}

#[dialog_common::test]
async fn it_rejects_a_value_failing_the_type() {
    // `bob` is not unsigned — the param's type rejects it mid-parse.
    match page_route().parse("/space/home/bob") {
        Err(ParseError::InvalidType {
            name, ty, value, ..
        }) => {
            assert_eq!(name, "page");
            assert_eq!(ty, "unsigned");
            assert_eq!(value, "bob");
        }
        other => panic!("expected InvalidType, got {other:?}"),
    }
}

#[dialog_common::test]
async fn it_disambiguates_two_routes_by_param_type() {
    // The disambiguation scenario: both routes are structurally identical
    // (`/space/{s}/{one-segment}`); only the param type differs. A numeric tail
    // matches the unsigned route and not the entity route; a URI tail does the
    // reverse. A table tries candidates and keeps the one that parses.
    let routes = [page_route(), model_route()];

    let numeric = "/space/home/42";
    let matched: Vec<&str> = routes
        .iter()
        .filter(|route| route.parse(numeric).is_ok())
        .map(|route| match &route.terms()[3] {
            Term::Param { name, .. } => name.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(matched, ["page"], "42 should match only the unsigned route");

    let uri = "/space/home/tonk:person";
    let matched: Vec<&str> = routes
        .iter()
        .filter(|route| route.parse(uri).is_ok())
        .map(|route| match &route.terms()[3] {
            Term::Param { name, .. } => name.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        matched,
        ["model"],
        "tonk:person should match only the entity route",
    );
}

#[dialog_common::test]
async fn it_round_trips_a_typed_value() {
    let route = page_route();
    let params = route.parse("/space/home/42").expect("parse");
    assert_eq!(route.format(&params).as_deref(), Ok("/space/home/42"));
}

#[dialog_common::test]
async fn it_rejects_formatting_a_type_violating_value() {
    // Formatting a non-unsigned `page` is refused — it wouldn't round-trip.
    let route = page_route();
    let params: tonk_router::Params = [("space", "home"), ("page", "bob")].into_iter().collect();
    assert!(route.format(&params).is_err());
}

#[dialog_common::test]
async fn it_treats_default_param_type_as_text() {
    // A plain `param` accepts anything (the default `text` type).
    let route = Route::new([Term::text("/"), Term::param("anything", Kind::Segment)]);
    assert!(route.parse("/whatever").is_ok());
}
