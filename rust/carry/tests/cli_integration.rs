//! Integration tests for the carry CLI.
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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = env.ctx();

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
    let ctx = carry::site::SiteContext::resolve(Some(env.site_path.as_path())).unwrap();
    assert_eq!(ctx.space.did, env.space_did);
}
