//! Render-port behaviour: the HTML the inspector builds from an evaluate
//! response. The render/response modules are wasm-gated (their only non-test
//! consumer is the wasm `element`), so this test compiles on wasm only.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use tonk_inspector::render::render_result;
use tonk_inspector::response::EvaluateResponse;
use wasm_bindgen_test::wasm_bindgen_test_configure;

wasm_bindgen_test_configure!(run_in_browser);

fn response(json: serde_json::Value) -> EvaluateResponse {
    serde_json::from_value(json).expect("response")
}

#[dialog_common::test]
async fn it_renders_a_failure_callout() {
    let html = render_result(Some("boom <bad>"), None);
    assert!(html.contains("wa-callout"), "failure renders a callout");
    assert!(html.contains("variant=\"danger\""));
    // The message is HTML-escaped.
    assert!(html.contains("boom &lt;bad&gt;"), "message escaped: {html}");
}

#[dialog_common::test]
async fn it_renders_an_empty_result_as_a_revision_badge() {
    let resp = response(serde_json::json!({
        "revision_after": { "tree": "#abcdefgh12345" },
        "matches_before": [],
        "matches_after": [],
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains("wa-badge"),
        "empty result shows a revision badge"
    );
    // Tree hash is truncated to 8 chars (the `#` stripped).
    assert!(html.contains(">abcdefgh<"), "short tree hash: {html}");
}

#[dialog_common::test]
async fn it_renders_a_generic_result_as_notation() {
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "person",
            "results": [{ "this": "id:alice", "fields": { "name": "Alice" } }],
        }],
        "matches_after": [{
            "label": "person",
            "results": [{ "this": "id:alice", "fields": { "name": "Alice" } }],
        }],
    }));
    let html = render_result(None, Some(&resp));
    // Notation head, the entity URI tinted as an entity, the string field.
    assert!(html.contains("person!:"), "notation head: {html}");
    assert!(html.contains("tonk-cm-entity"), "entity tint");
    assert!(html.contains("Alice"));
    // The tabbed result panel is present (before == after, single view).
    assert!(html.contains("evaluate-tabs"));
}

#[dialog_common::test]
async fn it_renders_a_comparison_when_the_commit_changed_results() {
    let resp = response(serde_json::json!({
        "revision_before": { "tree": "#before00" },
        "revision_after": { "tree": "#after000" },
        "matches_before": [],
        "matches_after": [{
            "label": "person",
            "results": [{ "this": "id:bob", "fields": {} }],
        }],
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains("wa-comparison"),
        "changed results → comparison: {html}"
    );
    assert!(html.contains("evaluate-side-before"));
    assert!(html.contains("evaluate-side-after"));
}

#[dialog_common::test]
async fn it_expands_a_concept_descriptor_from_stringified_source() {
    // A `concept:` block result carries its descriptor as a stringified JSON
    // `source` field; the renderer parses it and normalizes `as` discriminants.
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "concept",
            "results": [{
                "this": "concept:xyz",
                "fields": {
                    "source": "{\"description\":\"A person\",\"with\":{\"name\":{\"the\":\"x/name\",\"as\":\"Text\"}}}",
                },
            }],
        }],
        "matches_after": [{
            "label": "concept",
            "results": [{
                "this": "concept:xyz",
                "fields": {
                    "source": "{\"description\":\"A person\",\"with\":{\"name\":{\"the\":\"x/name\",\"as\":\"Text\"}}}",
                },
            }],
        }],
    }));
    let html = render_result(None, Some(&resp));
    assert!(html.contains("concept!:"), "concept head: {html}");
    assert!(html.contains("description"));
    // `Text` discriminant normalized to the kebab surface form `text`.
    assert!(
        html.contains(">text<"),
        "as discriminant normalized: {html}"
    );
}
