use tonk_inspector::debug::{Probe, render_failure, render_repository};

#[test]
fn it_renders_full_local_and_remote_diagnostics() {
    let repository = serde_json::json!({
        "name": "did:key:zSpaceRoute",
        "label": "Research notes",
        "subject": "did:key:zSpaceSubject",
        "operator": "did:key:zOperator",
        "profile": "did:key:zProfile",
        "branch": {
            "main": {
                "revision": { "tree": [0, 1, 2, 3] },
                "upstream": { "remote": "origin", "branch": "main" }
            }
        },
        "remote": {
            "origin": {
                "address": {
                    "Ucan": { "endpoint": "https://access.example.test/ucan/" }
                },
                "subject": "did:key:zRemoteSubject"
            }
        }
    });

    let html = render_repository(
        "did:key:zSpaceRoute",
        "main",
        &repository.to_string(),
        Probe::Idle,
    )
    .expect("repository info should render");

    for expected in [
        "Research notes",
        "did:key:zSpaceRoute",
        "did:key:zSpaceSubject",
        "did:key:zProfile",
        "did:key:zOperator",
        "origin/main",
        "https://access.example.test/ucan/",
        "did:key:zRemoteSubject",
        "#1Ldp",
        "probe remote",
        "data-copy-value=\"#1Ldp\"",
    ] {
        assert!(html.contains(expected), "missing {expected:?} in {html}");
    }
}

#[test]
fn it_names_a_space_without_an_upstream_as_local_only() {
    let repository = serde_json::json!({
        "name": "local-space",
        "label": "Offline draft",
        "subject": "did:key:zLocal",
        "operator": "did:key:zOperator",
        "profile": "did:key:zProfile",
        "branch": { "main": { "revision": null, "upstream": null } }
    });

    let html = render_repository("local-space", "main", &repository.to_string(), Probe::Idle)
        .expect("local-only repository should render");

    assert!(html.contains("none — local only"), "{html}");
    assert!(html.contains("not configured"), "{html}");
    assert!(html.contains("no commits"), "{html}");
    assert!(!html.contains("probe remote"), "{html}");
}

#[test]
fn it_keeps_diagnostics_compact_until_the_user_expands_them() {
    let repository = serde_json::json!({
        "name": "local-space",
        "label": "Offline draft",
        "subject": "did:key:zLocal",
        "operator": "did:key:zOperator",
        "profile": "did:key:zProfile",
        "branch": {
            "main": {
                "revision": { "tree": "#local-head" },
                "upstream": null
            }
        }
    });

    let html = render_repository("local-space", "main", &repository.to_string(), Probe::Idle)
        .expect("local repository should render");

    assert!(
        html.starts_with("<details class=\"inspector-debug__disclosure\">"),
        "diagnostics should be collapsed by default: {html}"
    );
    for summary_value in ["branch diagnostics", "main", "#local-head", "local only"] {
        assert!(
            html.contains(summary_value),
            "compact summary is missing {summary_value:?}: {html}"
        );
    }
    assert!(
        html.contains("copy revision"),
        "expanded details lost actions"
    );
    assert!(
        html.contains("aria-live=\"polite\""),
        "copy confirmation should be announced from its button"
    );
    assert!(
        !html.contains("inspector-debug__feedback"),
        "copy confirmation should not add a panel footer"
    );
}

#[test]
fn it_keeps_local_metadata_while_rendering_remote_probe_results() {
    let repository = serde_json::json!({
        "name": "space",
        "label": "Space",
        "subject": "did:key:zSpace",
        "operator": "did:key:zOperator",
        "profile": "did:key:zProfile",
        "branch": {
            "main": {
                "revision": { "tree": "#local-head" },
                "upstream": { "remote": "origin", "branch": "main" }
            }
        },
        "remote": { "origin": { "address": { "Memory": {} } } }
    });
    let status = serde_json::json!({
        "state": "behind",
        "local": { "tree": "#local-head" },
        "remote": { "tree": "#remote-head" }
    });

    let html = render_repository(
        "space",
        "main",
        &repository.to_string(),
        Probe::Response(&status.to_string()),
    )
    .expect("sync response should render");

    for expected in ["behind", "#local-head", "#remote-head", "did:key:zProfile"] {
        assert!(html.contains(expected), "missing {expected:?} in {html}");
    }
}

#[test]
fn it_escapes_a_local_metadata_failure() {
    let html = render_failure("space", "main", "offline <script>alert(1)</script>");

    assert!(html.contains("offline &lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(html.contains("data-debug-action=\"refresh\""));
}
