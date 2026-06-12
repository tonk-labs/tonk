//! In-browser test for the harness render path: a daemon envelope
//! in, real-renderer HTML out.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn it_renders_a_template_against_supplied_conclusions() {
    let request = serde_json::json!({
        "id": 1,
        "capability": "render-preview",
        "payload": {
            "template": "<article><h1>{name}</h1></article>",
            "conclusions": [
                { "this": "did:key:zX", "fields": { "name": "Alice" } }
            ],
        },
    })
    .to_string();

    let reply = slide_preview::handle(&request).expect("handle succeeds");
    let parsed: serde_json::Value = serde_json::from_str(&reply).expect("reply is JSON");
    assert_eq!(parsed["id"], 1);
    let html = parsed["payload"]["html"].as_str().expect("html string");
    assert!(
        html.contains("Alice"),
        "expected Alice in rendered output, got: {html}"
    );
}
