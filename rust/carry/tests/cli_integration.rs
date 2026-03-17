//! Integration tests for carry.
//!
//! These tests exercise the CLI through its library API, using isolated
//! `.carry/` site directories for each test. Every test gets its own
//! temporary directory with a bootstrapped space.

mod common;

use carry::target::{Field, FirstArg, Target};
use common::TestEnv;

// ═══════════════════════════════════════════════════════════════════════════
// Site & Init
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_init_creates_site() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    assert!(site.root().exists());
    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 1);
    assert!(spaces[0].did.starts_with("did:key:"));
}

#[tokio::test]
async fn test_init_with_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    carry::init::execute(Some("my-project".to_string()), Some(tmp.path()))
        .await
        .unwrap();

    let site = carry::site::Site::open(tmp.path()).unwrap();
    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 1);
    assert!(site.active_space_did().unwrap().is_some());
}

#[tokio::test]
async fn test_init_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    carry::init::execute(None, Some(tmp.path())).await.unwrap();
    carry::init::execute(None, Some(tmp.path())).await.unwrap();

    let site = carry::site::Site::open(tmp.path()).unwrap();
    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 1, "Should not create a second space");
}

// ═══════════════════════════════════════════════════════════════════════════
// Status
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_status_runs() {
    let env = TestEnv::new().await.unwrap();
    // Just verify it doesn't error
    carry::status_cmd::execute(Some(env.site_path.as_path()), "yaml")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_status_json() {
    let env = TestEnv::new().await.unwrap();
    carry::status_cmd::execute(Some(env.site_path.as_path()), "json")
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Assert & Query (domain targets)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_and_query_domain() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert a person
    let target = FirstArg::Target(Target::Domain("io.test.person".to_string()));
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("28".to_string()),
        },
    ];
    carry::assert_cmd::execute(&ctx, target, None, fields, "yaml")
        .await
        .unwrap();

    // Query back
    let query_target = Target::Domain("io.test.person".to_string());
    let query_fields = vec![
        Field {
            name: "name".to_string(),
            value: None,
        },
        Field {
            name: "age".to_string(),
            value: None,
        },
    ];
    carry::query_cmd::execute(&ctx, query_target, query_fields, "yaml")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_assert_multiple_entities() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert two different people
    let fields1 = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("28".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields1,
        "yaml",
    )
    .await
    .unwrap();

    let fields2 = vec![
        Field {
            name: "name".to_string(),
            value: Some("Bob".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("35".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields2,
        "yaml",
    )
    .await
    .unwrap();

    // Query all - should find both
    let query_fields = vec![Field {
        name: "name".to_string(),
        value: None,
    }];
    carry::query_cmd::execute(
        &ctx,
        Target::Domain("io.test.person".to_string()),
        query_fields,
        "yaml",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_query_with_filter() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert two people
    for (name, age) in [("Alice", "28"), ("Bob", "35")] {
        let fields = vec![
            Field {
                name: "name".to_string(),
                value: Some(name.to_string()),
            },
            Field {
                name: "age".to_string(),
                value: Some(age.to_string()),
            },
        ];
        carry::assert_cmd::execute(
            &ctx,
            FirstArg::Target(Target::Domain("io.test.person".to_string())),
            None,
            fields,
            "yaml",
        )
        .await
        .unwrap();
    }

    // Query with name filter
    let query_fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: None,
        },
    ];
    carry::query_cmd::execute(
        &ctx,
        Target::Domain("io.test.person".to_string()),
        query_fields,
        "yaml",
    )
    .await
    .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Assert with explicit entity
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_with_this_entity() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // First assert to get an entity
    let fields = vec![Field {
        name: "name".to_string(),
        value: Some("Alice".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Derive the same entity to update it
    let entity = carry::schema::derive_entity_from_fields(&[(
        "io.test.person/name".to_string(),
        "Alice".to_string(),
    )])
    .unwrap();

    // Update with this= to add age
    let update_fields = vec![Field {
        name: "age".to_string(),
        value: Some("28".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity.to_string()),
        update_fields,
        "yaml",
    )
    .await
    .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Retract
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_retract_specific_field() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("28".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Alice".to_string()),
        ("io.test.person/age".to_string(), "28".to_string()),
    ])
    .unwrap();

    // Retract only age
    let retract_fields = vec![Field {
        name: "age".to_string(),
        value: None,
    }];
    carry::retract_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity.to_string()),
        retract_fields,
        "yaml",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_retract_all_fields() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert
    let fields = vec![Field {
        name: "name".to_string(),
        value: Some("Alice".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    let entity = carry::schema::derive_entity_from_fields(&[(
        "io.test.person/name".to_string(),
        "Alice".to_string(),
    )])
    .unwrap();

    // Retract all
    carry::retract_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity.to_string()),
        vec![],
        "yaml",
    )
    .await
    .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Assert from file/stdin
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_from_json_content() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Write a JSON file with triples
    let json_content = r#"[
        {"the": "io.test.person/name", "of": "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD", "is": "Alice"},
        {"the": "io.test.person/age", "of": "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD", "is": 28}
    ]"#;

    let tmp_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp_file.path(), json_content).unwrap();

    let path_str = tmp_file.path().to_string_lossy().to_string();
    // Need to give it a .json extension for file detection
    let json_path = format!("{}.json", path_str);
    std::fs::copy(tmp_file.path(), &json_path).unwrap();

    carry::assert_cmd::execute(
        &ctx,
        FirstArg::File(json_path.clone()),
        None,
        vec![],
        "yaml",
    )
    .await
    .unwrap();

    std::fs::remove_file(&json_path).ok();
}

// ═══════════════════════════════════════════════════════════════════════════
// Target parsing
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_requires_fields() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let result = carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        vec![],
        "yaml",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_assert_requires_values() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let result = carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        vec![Field {
            name: "name".to_string(),
            value: None,
        }],
        "yaml",
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_retract_requires_this() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let result = carry::retract_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        vec![Field {
            name: "name".to_string(),
            value: None,
        }],
        "yaml",
    )
    .await;

    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// JSON output format
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_json_format() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let fields = vec![Field {
        name: "name".to_string(),
        value: Some("Alice".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "json",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_query_json_format() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let fields = vec![Field {
        name: "name".to_string(),
        value: Some("Alice".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    let query_fields = vec![Field {
        name: "name".to_string(),
        value: None,
    }];
    carry::query_cmd::execute(
        &ctx,
        Target::Domain("io.test.person".to_string()),
        query_fields,
        "json",
    )
    .await
    .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Site discovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_site_discovery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let site = carry::site::Site::init(tmp.path()).unwrap();
    let space = site.create_space().unwrap();
    site.set_active_space(&space.did).unwrap();

    // Create a nested directory
    let nested = tmp.path().join("deep").join("nested");
    std::fs::create_dir_all(&nested).unwrap();

    // Should find site from nested directory
    let found = carry::site::Site::discover(&nested).unwrap();
    assert_eq!(found.root(), tmp.path().join(".carry"));
}

#[tokio::test]
async fn test_site_context_resolve() {
    let env = TestEnv::new().await.unwrap();
    let ctx = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), None)
        .await
        .unwrap();
    assert_eq!(ctx.space.did, env.space_did);
}

// ═══════════════════════════════════════════════════════════════════════════
// Space — list
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_list_single() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Should list the single bootstrapped space without error
    carry::space_cmd::list(&site, "yaml").await.unwrap();
}

#[tokio::test]
async fn test_space_list_multiple_with_labels() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create two more spaces with labels
    carry::space_cmd::create(&site, Some("alpha".to_string()), "yaml")
        .await
        .unwrap();
    carry::space_cmd::create(&site, Some("beta".to_string()), "yaml")
        .await
        .unwrap();

    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 3, "Should have 3 spaces total");

    // list should run without error
    carry::space_cmd::list(&site, "yaml").await.unwrap();
}

#[tokio::test]
async fn test_space_list_json() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    carry::space_cmd::create(&site, Some("json-test".to_string()), "yaml")
        .await
        .unwrap();

    // JSON list should run without error
    carry::space_cmd::list(&site, "json").await.unwrap();
}

#[tokio::test]
async fn test_space_list_empty_site() {
    let tmp = tempfile::TempDir::new().unwrap();
    let site = carry::site::Site::init(tmp.path()).unwrap();

    // No spaces created — list should run without error
    carry::space_cmd::list(&site, "yaml").await.unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Space — create
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_create_no_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    let initial_count = site.list_spaces().unwrap().len();

    carry::space_cmd::create(&site, None, "yaml").await.unwrap();

    let spaces = site.list_spaces().unwrap();
    assert_eq!(
        spaces.len(),
        initial_count + 1,
        "Space count should increase by 1"
    );
}

#[tokio::test]
async fn test_space_create_with_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    carry::space_cmd::create(&site, Some("research".to_string()), "yaml")
        .await
        .unwrap();

    // The new space is now active — retrieve it and check its label
    let active = site.active_space().unwrap();
    let label = site.space_label(&active).await.unwrap();
    assert_eq!(label, Some("research".to_string()));
}

#[tokio::test]
async fn test_space_create_switches_active() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    let original_did = env.space_did.clone();

    carry::space_cmd::create(&site, Some("new-space".to_string()), "yaml")
        .await
        .unwrap();

    let active_did = site.active_space_did().unwrap().unwrap();
    assert_ne!(
        active_did, original_did,
        "Active space should have changed to the newly created space"
    );
}

#[tokio::test]
async fn test_space_create_json() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // JSON create should run without error
    carry::space_cmd::create(&site, Some("json-space".to_string()), "json")
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Space — switch
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_switch_by_did() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    let original_did = env.space_did.clone();

    // Create a second space (which becomes active)
    carry::space_cmd::create(&site, None, "yaml").await.unwrap();
    assert_ne!(site.active_space_did().unwrap().unwrap(), original_did);

    // Switch back to original by DID
    carry::space_cmd::switch(&site, &original_did)
        .await
        .unwrap();
    assert_eq!(site.active_space_did().unwrap().unwrap(), original_did);
}

#[tokio::test]
async fn test_space_switch_by_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    let original_did = env.space_did.clone();

    // Create a labeled space (becomes active)
    carry::space_cmd::create(&site, Some("labeled".to_string()), "yaml")
        .await
        .unwrap();
    let labeled_did = site.active_space_did().unwrap().unwrap();
    assert_ne!(labeled_did, original_did);

    // Switch back to original by DID
    carry::space_cmd::switch(&site, &original_did)
        .await
        .unwrap();
    assert_eq!(site.active_space_did().unwrap().unwrap(), original_did);

    // Switch to labeled space by label
    carry::space_cmd::switch(&site, "labeled").await.unwrap();
    assert_eq!(site.active_space_did().unwrap().unwrap(), labeled_did);
}

#[tokio::test]
async fn test_space_switch_already_active() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();
    let original_did = env.space_did.clone();

    // Switch to already-active space — should succeed (idempotent)
    carry::space_cmd::switch(&site, &original_did)
        .await
        .unwrap();
    assert_eq!(site.active_space_did().unwrap().unwrap(), original_did);
}

#[tokio::test]
async fn test_space_switch_nonexistent() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    let result = carry::space_cmd::switch(&site, "no-such-space").await;
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// Space — active
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_active_shows_current() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Should run without error when active space exists
    carry::space_cmd::active(&site, "yaml").await.unwrap();
}

#[tokio::test]
async fn test_space_active_json() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    carry::space_cmd::active(&site, "json").await.unwrap();
}

#[tokio::test]
async fn test_space_active_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let site = carry::site::Site::init(tmp.path()).unwrap();
    let _space = site.create_space().unwrap();
    // Don't set active — @active marker is absent

    // Should run without error, printing a helpful message
    carry::space_cmd::active(&site, "yaml").await.unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Space — delete
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_delete_by_did() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a second space (becomes active)
    carry::space_cmd::create(&site, None, "yaml").await.unwrap();
    let new_active_did = site.active_space_did().unwrap().unwrap();
    assert_eq!(site.list_spaces().unwrap().len(), 2);

    // Switch back to original so we can delete the new one
    carry::space_cmd::switch(&site, &env.space_did)
        .await
        .unwrap();

    // Delete the second space by DID (skip confirmation)
    carry::space_cmd::delete(&site, &new_active_did, true)
        .await
        .unwrap();

    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].did, env.space_did);
}

#[tokio::test]
async fn test_space_delete_by_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a labeled space (becomes active)
    carry::space_cmd::create(&site, Some("to-delete".to_string()), "yaml")
        .await
        .unwrap();
    assert_eq!(site.list_spaces().unwrap().len(), 2);

    // Switch back to original
    carry::space_cmd::switch(&site, &env.space_did)
        .await
        .unwrap();

    // Delete by label (skip confirmation)
    carry::space_cmd::delete(&site, "to-delete", true)
        .await
        .unwrap();

    let spaces = site.list_spaces().unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].did, env.space_did);
}

#[tokio::test]
async fn test_space_delete_active_fails() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Try to delete the active space — should fail
    let result = carry::space_cmd::delete(&site, &env.space_did, true).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Cannot delete the active space"),
        "Expected 'Cannot delete the active space' error, got: {}",
        err_msg
    );

    // Space should still exist
    assert_eq!(site.list_spaces().unwrap().len(), 1);
}

#[tokio::test]
async fn test_space_delete_nonexistent() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    let result = carry::space_cmd::delete(&site, "no-such-space", true).await;
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// --space flag (SiteContext resolution)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_flag_resolve_by_did() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a second space
    let space2 = site.create_space().unwrap();

    let ctx = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some(&space2.did))
        .await
        .unwrap();
    assert_eq!(ctx.space.did, space2.did);
}

#[tokio::test]
async fn test_space_flag_resolve_by_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a labeled space via the create command
    carry::space_cmd::create(&site, Some("flagged".to_string()), "yaml")
        .await
        .unwrap();
    let labeled_did = site.active_space_did().unwrap().unwrap();

    // Switch back to original
    site.set_active_space(&env.space_did).unwrap();

    // Resolve by label via --space flag
    let ctx = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("flagged"))
        .await
        .unwrap();
    assert_eq!(ctx.space.did, labeled_did);
}

#[tokio::test]
async fn test_space_flag_overrides_active() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a second space
    let space2 = site.create_space().unwrap();

    // Active is still the original (TestEnv sets it)
    assert_eq!(site.active_space_did().unwrap().unwrap(), env.space_did);

    // Resolve with --space pointing to space2 — should override active
    let ctx = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some(&space2.did))
        .await
        .unwrap();
    assert_eq!(ctx.space.did, space2.did);
    assert_ne!(ctx.space.did, env.space_did);

    // Active should not have changed (--space is read-only, doesn't switch)
    assert_eq!(site.active_space_did().unwrap().unwrap(), env.space_did);
}

#[tokio::test]
async fn test_space_flag_nonexistent_errors() {
    let env = TestEnv::new().await.unwrap();

    let result =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("nonexistent-space"))
            .await;
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// Space data isolation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_space_data_isolation() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Assert data in the original space
    let ctx_a =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some(&env.space_did))
            .await
            .unwrap();
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("28".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx_a,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify data is visible in space A
    let query_fields = vec![Field {
        name: "name".to_string(),
        value: None,
    }];
    // Query in space A should succeed (data exists)
    carry::query_cmd::execute(
        &ctx_a,
        Target::Domain("io.test.person".to_string()),
        query_fields.clone(),
        "yaml",
    )
    .await
    .unwrap();

    // Create space B and resolve a context for it
    carry::space_cmd::create(&site, Some("space-b".to_string()), "yaml")
        .await
        .unwrap();
    let ctx_b = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("space-b"))
        .await
        .unwrap();
    assert_ne!(ctx_b.space.did, ctx_a.space.did);

    // Query the same domain in space B — should find no entities
    // (query_cmd returns Ok even with no results, it just prints nothing)
    carry::query_cmd::execute(
        &ctx_b,
        Target::Domain("io.test.person".to_string()),
        query_fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify space B has no data by checking at the session level
    let session_b = ctx_b.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&session_b, attr)
        .await
        .unwrap();
    assert!(
        entities.is_empty(),
        "Space B should have no io.test.person entities, but found {}",
        entities.len()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Cross-space query/assert/retract via --space flag (regression tests)
//
// These tests reproduce the bug where --space was silently consumed by
// trailing_var_arg on the fields positional, so --space had no effect.
// ═══════════════════════════════════════════════════════════════════════════

/// Assert data in one space, switch to another, then query with --space flag
/// pointing back to the original. Verifies data is returned (not silently
/// ignored).
#[tokio::test]
async fn test_space_flag_cross_space_query() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Assert data in the default space (space A)
    let ctx_a = env.ctx().await;
    let fields_a = vec![
        Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("30".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx_a,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields_a,
        "yaml",
    )
    .await
    .unwrap();

    // Create space B and switch to it (so space A is no longer active)
    carry::space_cmd::create(&site, Some("space-b".to_string()), "yaml")
        .await
        .unwrap();
    // space B is now active
    let active = site.active_space_did().unwrap().unwrap();
    assert_ne!(active, env.space_did, "Should have switched to space B");

    // Query with --space pointing to space A by DID
    let ctx_via_flag =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some(&env.space_did))
            .await
            .unwrap();
    assert_eq!(
        ctx_via_flag.space.did, env.space_did,
        "--space flag should resolve to space A"
    );

    // Verify the data is actually accessible through this context
    let session = ctx_via_flag.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&session, attr)
        .await
        .unwrap();
    assert_eq!(
        entities.len(),
        1,
        "Should find 1 entity in space A via --space flag, but found {}",
        entities.len()
    );
}

/// Assert data in one space, switch to another, then query with --space flag
/// using a label (not DID). Verifies label-based resolution works.
#[tokio::test]
async fn test_space_flag_cross_space_query_by_label() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create a labeled space and assert data into it
    carry::space_cmd::create(&site, Some("labeled".to_string()), "yaml")
        .await
        .unwrap();
    let labeled_did = site.active_space_did().unwrap().unwrap();

    let ctx_labeled =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("labeled"))
            .await
            .unwrap();
    let fields = vec![Field {
        name: "title".to_string(),
        value: Some("Engineer".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx_labeled,
        FirstArg::Target(Target::Domain("io.test.role".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Switch back to the original space
    site.set_active_space(&env.space_did).unwrap();
    assert_eq!(
        site.active_space_did().unwrap().unwrap(),
        env.space_did,
        "Should be back on original space"
    );

    // Query with --space "labeled" (by label, not DID)
    let ctx_via_label =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("labeled"))
            .await
            .unwrap();
    assert_eq!(ctx_via_label.space.did, labeled_did);

    // Verify data is accessible
    let session = ctx_via_label.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let attr = ClaimAttribute::from_str("io.test.role/title").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&session, attr)
        .await
        .unwrap();
    assert_eq!(
        entities.len(),
        1,
        "Should find 1 entity in labeled space via --space flag"
    );
}

/// Assert into a non-active space using --space flag, then verify the data
/// landed in the correct space.
#[tokio::test]
async fn test_space_flag_cross_space_assert() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create space B
    carry::space_cmd::create(&site, Some("target".to_string()), "yaml")
        .await
        .unwrap();
    let target_did = site.active_space_did().unwrap().unwrap();

    // Switch back to original space
    site.set_active_space(&env.space_did).unwrap();

    // Assert into space B via --space flag (while space A is active)
    let ctx_target =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("target"))
            .await
            .unwrap();
    assert_eq!(ctx_target.space.did, target_did);

    let fields = vec![Field {
        name: "color".to_string(),
        value: Some("blue".to_string()),
    }];
    carry::assert_cmd::execute(
        &ctx_target,
        FirstArg::Target(Target::Domain("io.test.pref".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Active space should still be space A (--space doesn't switch)
    assert_eq!(site.active_space_did().unwrap().unwrap(), env.space_did);

    // Verify data is NOT in space A
    let session_a = env.ctx().await.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let attr = ClaimAttribute::from_str("io.test.pref/color").unwrap();
    let entities_a = carry::schema::find_entities_by_attribute(&session_a, attr.clone())
        .await
        .unwrap();
    assert!(
        entities_a.is_empty(),
        "Space A should have no io.test.pref data"
    );

    // Verify data IS in space B
    let session_b = ctx_target.open_session().await.unwrap();
    let entities_b = carry::schema::find_entities_by_attribute(&session_b, attr)
        .await
        .unwrap();
    assert_eq!(
        entities_b.len(),
        1,
        "Space B should have 1 entity via --space assert"
    );
}

/// Retract from a non-active space using --space flag.
#[tokio::test]
async fn test_space_flag_cross_space_retract() {
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Create space B, assert data into it
    carry::space_cmd::create(&site, Some("retract-target".to_string()), "yaml")
        .await
        .unwrap();
    let target_did = site.active_space_did().unwrap().unwrap();

    let ctx_b = carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some(&target_did))
        .await
        .unwrap();
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Bob".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("40".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        &ctx_b,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Find the entity DID that was just created
    let session_b = ctx_b.open_session().await.unwrap();
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&session_b, name_attr.clone())
        .await
        .unwrap();
    assert_eq!(entities.len(), 1);
    let entity_did = entities[0].to_string();

    // Switch to space A
    site.set_active_space(&env.space_did).unwrap();

    // Retract from space B via --space flag
    let ctx_retract =
        carry::site::SiteContext::resolve(Some(env.site_path.as_path()), Some("retract-target"))
            .await
            .unwrap();
    assert_eq!(ctx_retract.space.did, target_did);

    let retract_fields = vec![Field {
        name: "age".to_string(),
        value: None,
    }];
    carry::retract_cmd::execute(
        &ctx_retract,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity_did),
        retract_fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify age was retracted in space B
    let session_b2 = ctx_retract.open_session().await.unwrap();
    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_entities = carry::schema::find_entities_by_attribute(&session_b2, age_attr)
        .await
        .unwrap();
    assert!(
        age_entities.is_empty(),
        "Age should have been retracted from space B"
    );

    // But name should still exist
    let name_entities = carry::schema::find_entities_by_attribute(&session_b2, name_attr)
        .await
        .unwrap();
    assert_eq!(
        name_entities.len(),
        1,
        "Name should still exist in space B after retracting age"
    );
}
