//! Behavioural tests for the route grammar: parse, format, round-trip, the
//! `tonk/person` slash case, intra-segment `@`/`!` params, and pattern errors.

use tonk_router::{FormatError, Kind, Params, ParseError, PatternError, Route, Term};
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

/// Build a `Params` from `(name, value)` pairs.
fn params(pairs: &[(&str, &str)]) -> Params {
    pairs.iter().copied().collect()
}

// ---- The daily directory routes: a single `{model}` segment ----

#[dialog_common::test]
async fn it_parses_a_single_segment_model() {
    // `/space/{space}/{model}` over `/space/home/inspector`.
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/inspector"),
        Ok(params(&[("space", "home"), ("model", "inspector")])),
    );
}

#[dialog_common::test]
async fn it_parses_a_colon_namespaced_model() {
    // `dialog:diagnose` — a `:` is fine inside a segment (only `/` is excluded).
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/dialog:diagnose"),
        Ok(params(&[("space", "home"), ("model", "dialog:diagnose")])),
    );
}

// ---- The slash-in-model case: `tonk/person` ----

#[dialog_common::test]
async fn it_captures_a_slash_containing_model_with_a_path_kind() {
    // A `{model:path}` param is slash-tolerant, so `tonk/person` is captured
    // whole rather than truncated at the first `/`.
    let route = Route::parse_pattern("/space/{space}/{model:path}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/tonk/person"),
        Ok(params(&[("space", "home"), ("model", "tonk/person")])),
    );
}

#[dialog_common::test]
async fn it_truncates_a_slash_for_a_segment_kind() {
    // The default `Segment` kind stops at `/`, so a slash-containing tail leaves
    // an unmatched remainder — a `Trailing` error, not a silent partial match.
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    match route.parse("/space/home/tonk/person") {
        Err(ParseError::Trailing { at }) => {
            assert_eq!(&"/space/home/tonk/person"[at..], "/person");
        }
        other => panic!("expected Trailing, got {other:?}"),
    }
}

// ---- Intra-segment params: `{entity}@{model}!{view}` ----

#[dialog_common::test]
async fn it_parses_entity_at_model() {
    let route = Route::parse_pattern("/space/{space}/{entity}@{model}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/id:x@trip"),
        Ok(params(&[
            ("space", "home"),
            ("entity", "id:x"),
            ("model", "trip"),
        ])),
    );
}

#[dialog_common::test]
async fn it_parses_entity_at_model_bang_view() {
    let route = Route::parse_pattern("/space/{space}/{entity}@{model}!{view}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/id:x@trip!tonk:view"),
        Ok(params(&[
            ("space", "home"),
            ("entity", "id:x"),
            ("model", "trip"),
            ("view", "tonk:view"),
        ])),
    );
}

#[dialog_common::test]
async fn it_requires_the_at_delimiter_when_the_pattern_has_one() {
    // `{entity}@{model}` against a tail with no `@` fails to find the literal.
    let route = Route::parse_pattern("/space/{space}/{entity}@{model}").expect("pattern");
    assert!(matches!(
        route.parse("/space/home/trip"),
        Err(ParseError::Expected { expected, .. }) if expected == "@",
    ));
}

// ---- Empty / trailing edge cases ----

#[dialog_common::test]
async fn it_rejects_an_empty_param() {
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    // `//` would bind an empty `model`.
    assert!(matches!(
        route.parse("/space/home/"),
        Err(ParseError::EmptyParam { name, .. }) if name == "model",
    ));
}

#[dialog_common::test]
async fn it_rejects_a_trailing_remainder() {
    let route = Route::parse_pattern("/space/{space}").expect("pattern");
    match route.parse("/space/home/extra") {
        Err(ParseError::Trailing { at }) => {
            assert_eq!(&"/space/home/extra"[at..], "/extra");
        }
        other => panic!("expected Trailing, got {other:?}"),
    }
}

#[dialog_common::test]
async fn it_matches_a_bare_literal_route() {
    let route = Route::parse_pattern("/").expect("pattern");
    assert_eq!(route.parse("/"), Ok(Params::new()));
}

// ---- A `rest` catch-all ----

#[dialog_common::test]
async fn it_captures_the_whole_tail_with_rest() {
    let route = Route::parse_pattern("/space/{space}/{tail:rest}").expect("pattern");
    assert_eq!(
        route.parse("/space/home/id:x@trip!view/extra"),
        Ok(params(&[
            ("space", "home"),
            ("tail", "id:x@trip!view/extra"),
        ])),
    );
}

// ---- Formatting (the other direction) ----

#[dialog_common::test]
async fn it_formats_params_into_a_url() {
    let route = Route::parse_pattern("/space/{space}/{entity}@{model}!{view}").expect("pattern");
    let bound = params(&[
        ("space", "home"),
        ("entity", "id:x"),
        ("model", "trip"),
        ("view", "tonk:view"),
    ]);
    assert_eq!(
        route.format(&bound).as_deref(),
        Ok("/space/home/id:x@trip!tonk:view"),
    );
}

#[dialog_common::test]
async fn it_rejects_formatting_a_missing_param() {
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    assert!(matches!(
        route.format(&params(&[("space", "home")])),
        Err(FormatError::Missing { name }) if name == "model",
    ));
}

#[dialog_common::test]
async fn it_rejects_formatting_a_slash_into_a_segment_param() {
    // A `/` in a `Segment` value would not round-trip, so formatting rejects it.
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    assert!(matches!(
        route.format(&params(&[("space", "home"), ("model", "tonk/person")])),
        Err(FormatError::Invalid { name, .. }) if name == "model",
    ));
}

// ---- Round-trip: format(parse(url)) == url ----

#[dialog_common::test]
async fn it_round_trips_every_shape() {
    let cases = [
        ("/space/{space}/{model}", "/space/home/inspector"),
        ("/space/{space}/{model:path}", "/space/home/tonk/person"),
        ("/space/{space}/{entity}@{model}", "/space/home/id:x@trip"),
        (
            "/space/{space}/{entity}@{model}!{view}",
            "/space/home/id:x@trip!tonk:view",
        ),
        ("/", "/"),
    ];
    for (pattern, url) in cases {
        let route = Route::parse_pattern(pattern).expect("pattern");
        let bound = route.parse(url).unwrap_or_else(|e| panic!("{url}: {e:?}"));
        assert_eq!(
            route.format(&bound).as_deref(),
            Ok(url),
            "round-trip failed for {pattern}",
        );
    }
}

// ---- Pattern compilation errors ----

#[dialog_common::test]
async fn it_rejects_an_unclosed_param() {
    assert!(matches!(
        Route::parse_pattern("/space/{space"),
        Err(PatternError::UnclosedParam { .. }),
    ));
}

#[dialog_common::test]
async fn it_rejects_an_unknown_kind() {
    assert!(matches!(
        Route::parse_pattern("/space/{space:weird}"),
        Err(PatternError::UnknownKind { kind, .. }) if kind == "weird",
    ));
}

#[dialog_common::test]
async fn it_rejects_a_rest_param_that_is_not_last() {
    assert!(matches!(
        Route::parse_pattern("/space/{tail:rest}/more"),
        Err(PatternError::RestNotLast { .. }),
    ));
}

#[dialog_common::test]
async fn it_compiles_to_a_minimal_term_list() {
    // Adjacent literal runs merge: `/space/` is one Text term, not three.
    let route = Route::parse_pattern("/space/{space}/{model}").expect("pattern");
    assert_eq!(
        route.terms(),
        &[
            Term::text("/space/"),
            Term::param("space", Kind::Segment),
            Term::text("/"),
            Term::param("model", Kind::Segment),
        ],
    );
}
