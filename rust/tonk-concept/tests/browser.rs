//! Browser integration tests for the templating + renderer
//! pipeline. Bypass the SSE layer — feed `Conclusion` frames
//! directly to `Renderer::apply` against a host element we mount
//! on the live document.
//!
//! Run via `nix develop -c test:web:debug` (or
//! `wasm-pack test --headless --chrome`).

#![cfg(target_arch = "wasm32")]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use tonk_concept::render::Renderer;
use tonk_concept::template::{extract_plan, snapshot_template};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

/// Mount a fresh `<div>` host with the given inner HTML and
/// build a `Renderer` over it.
fn mount(inner_html: &str) -> (Element, Renderer) {
    let document = window().expect("window").document().expect("document");
    let host: Element = document.create_element("div").expect("create div");
    host.set_inner_html(inner_html);
    document
        .body()
        .expect("body")
        .append_child(&host)
        .expect("attach host");
    let snapshot = snapshot_template(&host).expect("snapshot");
    let plan = extract_plan(&snapshot.fragment);
    (
        host,
        Renderer::new(plan, snapshot.fragment, snapshot.container),
    )
}

/// Build a [`Conclusion`] with the given `this` URI and string
/// fields. JSON values are wrapped automatically.
fn conclusion(this: &str, fields: &[(&str, &str)]) -> Conclusion {
    let mut map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (k, v) in fields {
        map.insert((*k).to_owned(), serde_json::Value::String((*v).to_owned()));
    }
    Conclusion {
        this: this.to_owned(),
        fields: map,
    }
}

#[dialog_common::test]
fn it_renders_template_per_conclusion() {
    let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
    let frame = vec![
        conclusion("did:key:zAlice", &[("name", "Alice")]),
        conclusion("did:key:zBob", &[("name", "Bob")]),
    ];
    renderer.apply(&frame);
    let html = host.inner_html();
    assert!(html.contains("Alice"), "expected Alice in {html}");
    assert!(html.contains("Bob"), "expected Bob in {html}");
    assert_eq!(
        host.query_selector_all("article").unwrap().length(),
        2,
        "expected two row articles, got: {html}",
    );
}

#[dialog_common::test]
fn it_updates_in_place_on_change() {
    let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    let article_before = host
        .query_selector("article")
        .unwrap()
        .expect("first article");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alicia")])]);
    let article_after = host
        .query_selector("article")
        .unwrap()
        .expect("second article");
    // Same DOM node — the renderer mutated text content rather
    // than replacing the row.
    assert!(article_before.is_same_node(Some(article_after.unchecked_ref())));
    assert!(host.inner_html().contains("Alicia"));
    assert!(!host.inner_html().contains("Alice<"));
}

#[dialog_common::test]
fn it_removes_rows_dropped_from_frame() {
    let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
    renderer.apply(&[
        conclusion("did:key:zAlice", &[("name", "Alice")]),
        conclusion("did:key:zBob", &[("name", "Bob")]),
    ]);
    assert_eq!(host.query_selector_all("article").unwrap().length(), 2);
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    let html = host.inner_html();
    assert!(html.contains("Alice"));
    assert!(!html.contains("Bob"), "Bob row should be gone: {html}");
    assert_eq!(host.query_selector_all("article").unwrap().length(), 1);
}

#[dialog_common::test]
fn it_appends_new_rows() {
    let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    renderer.apply(&[
        conclusion("did:key:zAlice", &[("name", "Alice")]),
        conclusion("did:key:zBob", &[("name", "Bob")]),
    ]);
    assert_eq!(host.query_selector_all("article").unwrap().length(), 2);
    assert!(host.inner_html().contains("Bob"));
}

#[dialog_common::test]
fn it_substitutes_into_attribute_values() {
    let (host, mut renderer) = mount(r#"<a href="/entity/{this}">link</a>"#);
    renderer.apply(&[conclusion("did:key:zAlice", &[])]);
    let a = host.query_selector("a").unwrap().expect("anchor");
    assert_eq!(
        a.get_attribute("href").as_deref(),
        Some("/entity/did:key:zAlice"),
    );
}

#[dialog_common::test]
fn it_uses_template_element_when_present() {
    let (host, mut renderer) = mount(
        r#"<table><thead><tr><th>Name</th></tr></thead>
           <tbody><template><tr><td>{name}</td></tr></template></tbody></table>"#,
    );
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    // Chrome stays static; rows live in <tbody>.
    assert!(
        host.query_selector("table > thead").unwrap().is_some(),
        "expected static <thead>",
    );
    let tbody = host
        .query_selector("table > tbody")
        .unwrap()
        .expect("tbody");
    assert_eq!(
        tbody
            .query_selector_all("tr")
            .unwrap()
            .length(),
        1,
        "expected one row in tbody",
    );
    assert!(tbody.inner_html().contains("Alice"));
}

#[dialog_common::test]
fn it_falls_back_to_first_child_when_no_template() {
    let (host, mut renderer) = mount("<article>{name}</article>");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    let article = host.query_selector("article").unwrap().expect("article");
    assert_eq!(article.text_content().as_deref(), Some("Alice"));
}

#[dialog_common::test]
fn it_dedupes_writes_when_field_value_unchanged() {
    let (host, mut renderer) = mount("<article><h1>{name}</h1></article>");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    let h1 = host.query_selector("h1").unwrap().expect("h1");
    let text_node_before = h1.first_child().expect("h1 text node");
    renderer.apply(&[conclusion("did:key:zAlice", &[("name", "Alice")])]);
    let h1_again = host.query_selector("h1").unwrap().expect("h1");
    let text_node_after = h1_again.first_child().expect("h1 text node 2");
    assert!(
        text_node_before.is_same_node(Some(text_node_after.unchecked_ref())),
        "unchanged frame should not touch the DOM",
    );
}

// `IndexMap` import is to stop dead-code warnings if a future
// iteration drops some helpers.
const _: fn() = || {
    let _: IndexMap<String, String> = IndexMap::new();
    let _: HtmlElement;
};
