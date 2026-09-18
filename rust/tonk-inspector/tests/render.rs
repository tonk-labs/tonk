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
async fn it_renders_a_byte_array_tree_as_a_base58_badge() {
    // The real wire `tree` is a TreeReference — a Blake3 hash serialized as a
    // byte SEQUENCE, not a string. The renderer base58-encodes it for the badge.
    // (Regression: a String-typed mirror made the whole response fail to decode
    // with "invalid type: sequence, expected a string".)
    let resp = response(serde_json::json!({
        "revision_after": { "tree": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] },
        "matches_before": [],
        "matches_after": [],
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains("wa-badge"),
        "byte-array tree still renders a badge"
    );
    // base58 of [0,1,2,3,4,5,6,7,8,9] starts with leading-zero '1's then chars.
    assert!(
        html.contains("title=\"#"),
        "badge title is the #base58 form: {html}",
    );
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

#[dialog_common::test]
async fn it_shows_a_published_name_in_place_of_the_entity() {
    // The whole point: an entity the branch names reads as the name, and
    // the URI is still right there — on `data-entity` for the click and
    // on `title` for the hover.
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "person",
            "results": [{
                "this": "did:key:z6MkfpAValice",
                "fields": { "employer": "did:key:z6MkfpAVacme" },
            }],
        }],
        "matches_after": [{
            "label": "person",
            "results": [{
                "this": "did:key:z6MkfpAValice",
                "fields": { "employer": "did:key:z6MkfpAVacme" },
            }],
        }],
        "names": {
            "did:key:z6MkfpAValice": "alice",
            "did:key:z6MkfpAVacme": "acme",
        },
    }));
    let html = render_result(None, Some(&resp));
    assert!(html.contains(">alice<"), "`this` reads as its name: {html}");
    assert!(
        html.contains(">acme<"),
        "an entity-valued field reads as its name too: {html}"
    );
    assert!(
        html.contains("data-entity=\"did:key:z6MkfpAValice\""),
        "the URI is carried for the reveal: {html}"
    );
    assert!(
        html.contains("title=\"did:key:z6MkfpAValice\""),
        "and on the hover: {html}"
    );
    assert!(
        html.contains("notation-named"),
        "marked for the click handler"
    );
}

#[dialog_common::test]
async fn it_falls_back_to_the_entity_uri_when_nothing_names_it() {
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "person",
            "results": [{ "this": "did:key:z6MkfpAVbob", "fields": {} }],
        }],
        "matches_after": [{
            "label": "person",
            "results": [{ "this": "did:key:z6MkfpAVbob", "fields": {} }],
        }],
        "names": { "did:key:z6MkfpAValice": "alice" },
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains("did:key:z6MkfpAVbob"),
        "an unnamed entity still shows its URI: {html}"
    );
    assert!(
        !html.contains("notation-named"),
        "and carries no reveal affordance: {html}"
    );
}

#[dialog_common::test]
async fn it_names_the_table_this_column_without_a_reveal() {
    // The table's `this` cell is a copy button; a second meaning on the
    // same click would make copying unpredictable, so the name is inert
    // there and the URI stays the copied value.
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "person",
            "results": [{ "this": "did:key:z6MkfpAValice", "fields": { "age": 41 } }],
        }],
        "matches_after": [{
            "label": "person",
            "results": [{ "this": "did:key:z6MkfpAValice", "fields": { "age": 41 } }],
        }],
        "names": { "did:key:z6MkfpAValice": "alice" },
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains("query-table-named"),
        "the table cell names the row: {html}"
    );
    assert!(
        html.contains("<wa-copy-button value=\"did:key:z6MkfpAValice\">"),
        "copying still yields the URI: {html}"
    );
}

#[dialog_common::test]
async fn it_names_entities_inside_an_expanded_concept_descriptor() {
    // A `concept:` body is where URIs are least readable — the
    // descriptor is expanded from a stringified `source`, and the
    // substitution has to reach into it.
    let resp = response(serde_json::json!({
        "matches_before": [{
            "label": "concept",
            "results": [{
                "this": "did:key:z6MkfpAVperson",
                "fields": {
                    "source": "{\"with\":{\"name\":{\"the\":\"did:key:z6MkfpAVattr\"}}}",
                },
            }],
        }],
        "matches_after": [{
            "label": "concept",
            "results": [{
                "this": "did:key:z6MkfpAVperson",
                "fields": {
                    "source": "{\"with\":{\"name\":{\"the\":\"did:key:z6MkfpAVattr\"}}}",
                },
            }],
        }],
        "names": {
            "did:key:z6MkfpAVperson": "person",
            "did:key:z6MkfpAVattr": "person-name",
        },
    }));
    let html = render_result(None, Some(&resp));
    assert!(
        html.contains(">person<"),
        "the concept reads as its name: {html}"
    );
    assert!(
        html.contains(">person-name<"),
        "so does the attribute it declares: {html}"
    );
}
