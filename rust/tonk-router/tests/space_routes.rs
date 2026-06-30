use tonk_router::{Route, Router};
use wasm_bindgen_test::wasm_bindgen_test_configure;
wasm_bindgen_test_configure!(run_in_browser);

fn r(p: &str) -> Route {
    Route::parse_pattern(p).expect(p)
}

#[dialog_common::test]
async fn it_matches_inspector_against_the_real_table() {
    // The exact seeded table (sorted by entity URI as the SW does).
    let mut router = Router::new();
    router.insert(r("/{*entity}@{*model}!{*view}"), "adhoc");
    router.insert(r("/{*entity}@{*model}"), "artifact");
    router.insert(r("/{*model}"), "directory");
    router.insert(r("/"), "space");

    let m = router.recognize("/inspector");
    assert!(m.is_ok(), "inspector should match: {m:?}");
    assert_eq!(*m.unwrap().value, "directory");

    let m2 = router.recognize("/");
    assert_eq!(*m2.unwrap().value, "space");

    let m3 = router.recognize("/tonk/person");
    assert_eq!(*m3.expect("tonk/person").value, "directory");
}

#[dialog_common::test]
async fn it_treats_a_trailing_slash_as_insignificant() {
    // The profile's space table: a bare space root and any sub-path.
    let mut router = Router::new();
    router.insert(r("/space/{id}/{*rest}"), "sub");
    router.insert(r("/space/{id}"), "root");

    // No trailing slash hits the bare route.
    assert_eq!(*router.recognize("/space/abc").expect("bare").value, "root");
    // A trailing slash must resolve to the SAME route — not fall into the gap
    // between the bare route (leftover `/`) and the sub route (empty `rest`).
    assert_eq!(
        *router
            .recognize("/space/abc/")
            .expect("trailing slash")
            .value,
        "root",
    );
    // A real sub-path still routes to the sub route.
    assert_eq!(
        *router.recognize("/space/abc/inspector").expect("sub").value,
        "sub",
    );
    // Trailing slash on a sub-path collapses to the bare sub-path.
    assert_eq!(
        *router
            .recognize("/space/abc/inspector/")
            .expect("sub trailing")
            .value,
        "sub",
    );

    // The bare root `/` is itself a route and must be preserved, not stripped to
    // the empty string.
    let mut root_router = Router::new();
    root_router.insert(r("/"), "space");
    assert_eq!(*root_router.recognize("/").expect("root").value, "space");
}
