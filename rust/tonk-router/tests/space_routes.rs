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
