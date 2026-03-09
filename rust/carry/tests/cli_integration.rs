//! Integration tests for the carry CLI (tonk-cli crate).
//!
//! These tests exercise the CLI through its library API, using isolated
//! `.carry/` site directories for each test. Every test gets its own
//! temporary directory with a bootstrapped space.

mod common;

use std::f64::consts::PI;

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
    assert!(site.root().exists());
}

#[tokio::test]
async fn test_init_with_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let loc = common::unique_dir("carry-test");
    let repo_loc = common::unique_dir("carry-test-repo");
    carry::init::execute(
        Some("my-project".to_string()),
        vec![],
        Some(tmp.path()),
        Some(loc.clone()),
        Some(repo_loc.clone()),
    )
    .await
    .unwrap();

    let site = carry::site::Site::open(tmp.path(), Some(loc), Some(repo_loc.clone()))
        .await
        .unwrap();
    assert!(site.root().exists());
    assert!(site.root().exists());
}

#[tokio::test]
async fn test_init_idempotent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let loc = common::unique_dir("carry-test");
    let repo_loc = common::unique_dir("carry-test-repo");
    carry::init::execute(
        None,
        vec![],
        Some(tmp.path()),
        Some(loc.clone()),
        Some(repo_loc.clone()),
    )
    .await
    .unwrap();
    // Second init should succeed (idempotent)
    carry::init::execute(
        None,
        vec![],
        Some(tmp.path()),
        Some(loc.clone()),
        Some(repo_loc.clone()),
    )
    .await
    .unwrap();

    let site = carry::site::Site::open(tmp.path(), Some(loc), Some(repo_loc.clone()))
        .await
        .unwrap();
    assert!(site.root().exists());
}

// ═══════════════════════════════════════════════════════════════════════════
// Status
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_status_runs() {
    let env = TestEnv::new().await.unwrap();
    // Just verify it doesn't error
    carry::status_cmd::execute(
        Some(env.site_path.as_path()),
        "yaml",
        Some(env.profile_location.clone()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_status_json() {
    let env = TestEnv::new().await.unwrap();
    carry::status_cmd::execute(
        Some(env.site_path.as_path()),
        "json",
        Some(env.profile_location.clone()),
    )
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
    carry::assert_cmd::execute(ctx, target, None, None, fields, "yaml")
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
    carry::query_cmd::execute(ctx, query_target, query_fields, "yaml")
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
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
            ctx,
            FirstArg::Target(Target::Domain("io.test.person".to_string())),
            None,
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
        ctx,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity.to_string()),
        None,
        update_fields,
        "yaml",
    )
    .await
    .unwrap();
}

/// Asserting a new value for a cardinality-one attribute on an existing entity
/// should replace the old value, not accumulate both values.
#[tokio::test]
async fn test_assert_cardinality_one_replaces_value() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert name=Alice age=28
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Derive the entity
    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Alice".to_string()),
        ("io.test.person/age".to_string(), "28".to_string()),
    ])
    .unwrap();

    // Update age to 29 using this=<entity>
    let update_fields = vec![Field {
        name: "age".to_string(),
        value: Some("29".to_string()),
    }];
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        Some(entity.to_string()),
        None,
        update_fields,
        "yaml",
    )
    .await
    .unwrap();

    // Query: age should be [29], not [28, 29]
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(
        age_values.len(),
        1,
        "Cardinality-one attribute should have exactly one value after update, got: {:?}",
        age_values
    );
    assert_eq!(
        age_values[0],
        dialog_query::Value::UnsignedInt(29),
        "Age should be updated to 29"
    );

    // Name should still be Alice (unchanged)
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let name_values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(name_values.len(), 1);
    assert_eq!(
        name_values[0],
        dialog_query::Value::String("Alice".to_string())
    );
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
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
        ctx,
        FirstArg::File(json_path.clone()),
        None,
        None,
        vec![],
        "yaml",
    )
    .await
    .unwrap();

    std::fs::remove_file(&json_path).ok();
}

// ═══════════════════════════════════════════════════════════════════════════
// Piping: --format triples, asserted notation, stdin round-trips
// ═══════════════════════════════════════════════════════════════════════════

/// Helper: assert a file's YAML content and return the path
fn write_yaml_file(content: &str) -> (String, tempfile::NamedTempFile) {
    let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
    std::fs::write(tmp.path(), content).unwrap();
    (tmp.path().to_string_lossy().to_string(), tmp)
}

/// format_triples produces valid EAV YAML for a single entity.
#[tokio::test]
async fn test_format_triples_single_entity() {
    use dialog_query::Value;
    use std::collections::BTreeMap;

    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Alice".to_string())],
    );
    attrs.insert(
        "io.test.person/age".to_string(),
        vec![Value::UnsignedInt(28)],
    );

    let mut results = BTreeMap::new();
    results.insert(
        "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD".to_string(),
        attrs,
    );

    let yaml = carry::query_cmd::format_triples(&results).unwrap();
    assert!(!yaml.is_empty());

    // Parse back to verify it's valid YAML with expected structure
    let parsed: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(parsed.len(), 2); // Two triples (name + age)

    // Each triple should have the/of/is keys
    for triple in &parsed {
        assert!(triple["the"].as_str().is_some());
        assert!(triple["of"].as_str().is_some());
        assert!(!triple["is"].is_null());
    }
}

/// format_triples expands multi-valued attributes into separate triples.
#[tokio::test]
async fn test_format_triples_multivalued() {
    use dialog_query::Value;
    use std::collections::BTreeMap;

    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/tag".to_string(),
        vec![
            Value::String("engineer".to_string()),
            Value::String("leader".to_string()),
        ],
    );

    let mut results = BTreeMap::new();
    results.insert(
        "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD".to_string(),
        attrs,
    );

    let yaml = carry::query_cmd::format_triples(&results).unwrap();
    let parsed: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(
        parsed.len(),
        2,
        "Multi-valued attr should produce two triples"
    );

    // Both should reference the same attribute
    assert_eq!(parsed[0]["the"].as_str().unwrap(), "io.test.person/tag");
    assert_eq!(parsed[1]["the"].as_str().unwrap(), "io.test.person/tag");
}

/// format_triples handles multiple entities.
#[tokio::test]
async fn test_format_triples_multiple_entities() {
    use dialog_query::Value;
    use std::collections::BTreeMap;

    let mut results = BTreeMap::new();

    let mut attrs1 = BTreeMap::new();
    attrs1.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Alice".to_string())],
    );
    results.insert(
        "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD".to_string(),
        attrs1,
    );

    let mut attrs2 = BTreeMap::new();
    attrs2.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Bob".to_string())],
    );
    results.insert(
        "did:key:z6Mkf5rGMoatrSj1f4CyvuHqdjKN6pVpGGqruHMgfJBuRnQE".to_string(),
        attrs2,
    );

    let yaml = carry::query_cmd::format_triples(&results).unwrap();
    let parsed: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(parsed.len(), 2, "Two entities should produce two triples");
}

/// format_triples returns empty string for empty results.
#[tokio::test]
async fn test_format_triples_empty() {
    use std::collections::BTreeMap;

    let results: BTreeMap<String, BTreeMap<String, Vec<dialog_query::Value>>> = BTreeMap::new();
    let yaml = carry::query_cmd::format_triples(&results).unwrap();
    assert!(yaml.is_empty());
}

/// format_triples preserves various value types.
#[tokio::test]
async fn test_format_triples_value_types() {
    use dialog_query::Value;
    use std::collections::BTreeMap;

    let entity = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";

    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.data/text".to_string(),
        vec![Value::String("hello".to_string())],
    );
    attrs.insert(
        "io.test.data/uint".to_string(),
        vec![Value::UnsignedInt(42)],
    );
    attrs.insert("io.test.data/sint".to_string(), vec![Value::SignedInt(-7)]);
    attrs.insert("io.test.data/float".to_string(), vec![Value::Float(PI)]);
    attrs.insert("io.test.data/bool".to_string(), vec![Value::Boolean(true)]);

    let mut results = BTreeMap::new();
    results.insert(entity.to_string(), attrs);

    let yaml = carry::query_cmd::format_triples(&results).unwrap();
    let parsed: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(parsed.len(), 5);
}

/// Round-trip: assert data → derive entity → format as triples YAML → assert from file → verify.
#[tokio::test]
async fn test_roundtrip_triples_yaml() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // 1. Assert data using domain target
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // 2. Derive the entity DID (same deterministic derivation as the CLI uses)
    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Alice".to_string()),
        ("io.test.person/age".to_string(), "28".to_string()),
    ])
    .unwrap();

    // 3. Verify we can fetch the data

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);

    // 4. Build results map and format as triples YAML
    use dialog_query::Value;
    use std::collections::BTreeMap;
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Alice".to_string())],
    );
    attrs.insert(
        "io.test.person/age".to_string(),
        vec![Value::UnsignedInt(28)],
    );
    let mut results = BTreeMap::new();
    results.insert(entity.to_string(), attrs);

    let triples_yaml = carry::query_cmd::format_triples(&results).unwrap();

    // 5. Assert from the triples YAML file (idempotent in same space)
    let (yaml_path, _tmp) = write_yaml_file(&triples_yaml);
    carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // 6. Verify the data still exists (round-trip preserved it)

    let name_attr2 = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr2)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(carry::schema::format_value(&values[0]), "Alice");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(age_values.len(), 1);
    assert_eq!(carry::schema::format_value(&age_values[0]), "28");
}

/// Round-trip: asserted notation YAML → assert from file → verify.
#[tokio::test]
async fn test_roundtrip_asserted_notation_yaml() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // 1. Assert data
    let fields = vec![
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // 2. Derive entity and build asserted notation YAML
    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Bob".to_string()),
        ("io.test.person/age".to_string(), "35".to_string()),
    ])
    .unwrap();

    use dialog_query::Value;
    use std::collections::BTreeMap;
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Bob".to_string())],
    );
    attrs.insert(
        "io.test.person/age".to_string(),
        vec![Value::UnsignedInt(35)],
    );
    let mut results = BTreeMap::new();
    results.insert(entity.to_string(), attrs);

    let asserted_yaml = carry::query_cmd::format_asserted_yaml(&results, "io.test.person");

    // 3. Assert from the asserted notation YAML (idempotent in same space)
    let (yaml_path, _tmp) = write_yaml_file(&asserted_yaml);
    carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // 4. Verify data is still intact
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(carry::schema::format_value(&values[0]), "Bob");
}

/// Round-trip: asserted notation YAML with multi-valued fields.
#[tokio::test]
async fn test_roundtrip_asserted_notation_multivalued() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Construct asserted notation YAML with a list value
    let entity_did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    let yaml = format!(
        "{}:\n  io.test.person:\n    name: Alice\n    tag:\n      - engineer\n      - leader\n",
        entity_did
    );

    let (yaml_path, _tmp) = write_yaml_file(&yaml);
    carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let entity = dialog_query::Entity::from_str(entity_did).unwrap();
    let tag_attr = ClaimAttribute::from_str("io.test.person/tag").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, tag_attr)
        .await
        .unwrap();
    assert_eq!(
        values.len(),
        2,
        "Multi-valued field should produce two claims"
    );
}

/// Assert from EAV triple YAML file.
#[tokio::test]
async fn test_assert_from_eav_triple_yaml() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let entity_did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";
    let yaml = format!(
        "- the: io.test.person/name\n  of: {}\n  is: Alice\n- the: io.test.person/age\n  of: {}\n  is: 28\n",
        entity_did, entity_did
    );

    let (yaml_path, _tmp) = write_yaml_file(&yaml);
    carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let entity = dialog_query::Entity::from_str(entity_did).unwrap();

    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(carry::schema::format_value(&values[0]), "Alice");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(age_values.len(), 1);
    assert_eq!(carry::schema::format_value(&age_values[0]), "28");
}

/// Retract from EAV triple YAML file.
#[tokio::test]
async fn test_retract_from_eav_triple_yaml() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // First assert some data
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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

    // Verify data exists

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;
    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr.clone())
        .await
        .unwrap();
    assert_eq!(values.len(), 1);

    // Now retract the age via EAV triple YAML file
    let yaml = format!("- the: io.test.person/age\n  of: {}\n  is: 28\n", entity);
    let (yaml_path, _tmp) = write_yaml_file(&yaml);
    carry::retract_cmd::execute(ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify age is retracted

    let values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 0, "Age should be retracted");

    // Name should still exist
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let name_values = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(name_values.len(), 1, "Name should still exist");
}

/// Retract from asserted notation YAML file.
#[tokio::test]
async fn test_retract_from_asserted_yaml() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert data
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Charlie".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("40".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Charlie".to_string()),
        ("io.test.person/age".to_string(), "40".to_string()),
    ])
    .unwrap();

    // Build asserted notation YAML for retraction (matching query output)
    use dialog_query::Value;
    use std::collections::BTreeMap;
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Charlie".to_string())],
    );
    attrs.insert(
        "io.test.person/age".to_string(),
        vec![Value::UnsignedInt(40)],
    );
    let mut results = BTreeMap::new();
    results.insert(entity.to_string(), attrs);

    let asserted_yaml = carry::query_cmd::format_asserted_yaml(&results, "io.test.person");
    let (yaml_path, _tmp) = write_yaml_file(&asserted_yaml);

    carry::retract_cmd::execute(ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify both fields are retracted

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let name_vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(name_vals.len(), 0, "Name should be retracted");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(age_vals.len(), 0, "Age should be retracted");
}

/// Retract from JSON content (EAV triples).
#[tokio::test]
async fn test_retract_from_json_content() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let entity_did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";

    // Assert via JSON first
    let json_content = format!(
        r#"[{{"the": "io.test.person/name", "of": "{}", "is": "Alice"}}]"#,
        entity_did
    );
    let tmp = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    std::fs::write(tmp.path(), &json_content).unwrap();
    let json_path = tmp.path().to_string_lossy().to_string();

    carry::assert_cmd::execute(ctx, FirstArg::File(json_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify exists
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let entity = dialog_query::Entity::from_str(entity_did).unwrap();
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr.clone())
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);

    // Retract via JSON
    let retract_json = format!(
        r#"[{{"the": "io.test.person/name", "of": "{}", "is": "Alice"}}]"#,
        entity_did
    );
    let tmp2 = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    std::fs::write(tmp2.path(), &retract_json).unwrap();
    let json_path2 = tmp2.path().to_string_lossy().to_string();

    carry::retract_cmd::execute(ctx, FirstArg::File(json_path2), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify retracted

    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 0, "Should be retracted");
}

/// Round-trip: assert → format triples → retract from triples → verify gone.
#[tokio::test]
async fn test_roundtrip_query_retract_triples() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert data
    let fields = vec![
        Field {
            name: "name".to_string(),
            value: Some("Eve".to_string()),
        },
        Field {
            name: "age".to_string(),
            value: Some("25".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    let entity = carry::schema::derive_entity_from_fields(&[
        ("io.test.person/name".to_string(), "Eve".to_string()),
        ("io.test.person/age".to_string(), "25".to_string()),
    ])
    .unwrap();

    // Build triples YAML (simulating carry query --format triples output)
    use dialog_query::Value;
    use std::collections::BTreeMap;
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.person/name".to_string(),
        vec![Value::String("Eve".to_string())],
    );
    attrs.insert(
        "io.test.person/age".to_string(),
        vec![Value::UnsignedInt(25)],
    );
    let mut results = BTreeMap::new();
    results.insert(entity.to_string(), attrs);

    let triples_yaml = carry::query_cmd::format_triples(&results).unwrap();

    // Retract using the triples YAML
    let (yaml_path, _tmp) = write_yaml_file(&triples_yaml);
    carry::retract_cmd::execute(ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify both fields are retracted
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 0, "Name should be retracted");
}

/// format_triples YAML can be parsed back by assert (end-to-end format contract).
#[tokio::test]
async fn test_triples_format_contract() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Build triples with various value types
    use dialog_query::Value;
    use std::collections::BTreeMap;

    let entity_did = "did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD";

    let mut attrs = BTreeMap::new();
    attrs.insert(
        "io.test.data/text".to_string(),
        vec![Value::String("hello world".to_string())],
    );
    attrs.insert(
        "io.test.data/number".to_string(),
        vec![Value::UnsignedInt(42)],
    );
    attrs.insert(
        "io.test.data/negative".to_string(),
        vec![Value::SignedInt(-7)],
    );
    attrs.insert("io.test.data/flag".to_string(), vec![Value::Boolean(true)]);

    let mut results = BTreeMap::new();
    results.insert(entity_did.to_string(), attrs);

    let triples_yaml = carry::query_cmd::format_triples(&results).unwrap();

    // Assert from the triples YAML
    let (yaml_path, _tmp) = write_yaml_file(&triples_yaml);
    carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify each value was stored
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let entity = dialog_query::Entity::from_str(entity_did).unwrap();

    let text_attr = ClaimAttribute::from_str("io.test.data/text").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, text_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "hello world");

    let num_attr = ClaimAttribute::from_str("io.test.data/number").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, num_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "42");

    let neg_attr = ClaimAttribute::from_str("io.test.data/negative").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, neg_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "-7");

    let flag_attr = ClaimAttribute::from_str("io.test.data/flag").unwrap();
    let vals = carry::schema::fetch_values(&ctx.branch, &ctx.operator, &entity, flag_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "true");
}

/// Query with --format triples doesn't error.
#[tokio::test]
async fn test_query_triples_format_runs() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Assert some data
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Query with triples format
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
    carry::query_cmd::execute(
        ctx,
        Target::Domain("io.test.person".to_string()),
        query_fields,
        "triples",
    )
    .await
    .unwrap();
}

/// Malformed YAML input gives a clear error.
#[tokio::test]
async fn test_assert_malformed_yaml_error() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let bad_yaml = "this is not valid YAML for triples: [[[";
    let (yaml_path, _tmp) = write_yaml_file(bad_yaml);
    let result =
        carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
            .await;
    assert!(result.is_err());
}

/// Malformed YAML input to retract gives a clear error.
#[tokio::test]
async fn test_retract_malformed_yaml_error() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let bad_yaml = "this is not valid YAML for triples: [[[";
    let (yaml_path, _tmp) = write_yaml_file(bad_yaml);
    let result =
        carry::retract_cmd::execute(ctx, FirstArg::File(yaml_path), None, vec![], "yaml").await;
    assert!(result.is_err());
}

/// EAV triple YAML with missing 'the' gives a clear error.
#[tokio::test]
async fn test_assert_eav_missing_the_error() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let yaml = "- of: did:key:z6MkihEpYC9Q7Qx46UTkepj9WmvEFzn8Hymeb6BKH95ehSWD\n  is: Alice\n";
    let (yaml_path, _tmp) = write_yaml_file(yaml);
    let result =
        carry::assert_cmd::execute(ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
            .await;
    assert!(result.is_err());
    // The error chain should mention the missing 'the' key
    let err = result.unwrap_err();
    let full_err = format!("{:#}", err);
    assert!(
        full_err.contains("the"),
        "Error should mention missing 'the': {}",
        full_err
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Target parsing
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_assert_requires_fields() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let result = carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
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
        ctx,
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
async fn test_site_resolve() {
    let env = TestEnv::new().await.unwrap();
    let site = carry::site::Site::resolve(
        Some(env.site_path.as_path()),
        Some(env.profile_location.clone()),
    )
    .await
    .unwrap();
    assert!(site.root().exists());
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Bootstrap & Init
// ═══════════════════════════════════════════════════════════════════════════

/// After init, the pre-registered concepts (attribute, concept, bookmark)
/// should be discoverable by name via `dialog.meta/name`.
#[tokio::test]
async fn test_init_bootstraps_builtins() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    for name in &["attribute", "concept", "bookmark"] {
        let entity = carry::schema::lookup_entity_by_name(&ctx.branch, &ctx.operator, name)
            .await
            .unwrap();
        assert!(
            entity.is_some(),
            "Builtin concept '{}' should be discoverable by name after init",
            name
        );
    }
}

/// The bootstrapped `attribute` concept should resolve with the expected
/// required fields: `the`, `as`, `cardinality`.
#[tokio::test]
async fn test_bootstrap_attribute_concept_has_fields() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let concept = carry::schema::resolve_concept(&ctx.branch, &ctx.operator, "attribute")
        .await
        .unwrap();

    let field_names: Vec<&String> = concept.with_fields.keys().collect();
    assert!(
        field_names.contains(&&"the".to_string()),
        "attribute concept should have 'the' field, got: {:?}",
        field_names
    );
    assert!(
        field_names.contains(&&"as".to_string()),
        "attribute concept should have 'as' field, got: {:?}",
        field_names
    );
    assert!(
        field_names.contains(&&"cardinality".to_string()),
        "attribute concept should have 'cardinality' field, got: {:?}",
        field_names
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Attribute assertion
// ═══════════════════════════════════════════════════════════════════════════

/// Asserting a builtin `attribute` concept should create an entity with
/// the expected dialog.attribute/* claims.
#[tokio::test]
async fn test_assert_attribute_creates_entity() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let fields = vec![
        Field {
            name: "the".to_string(),
            value: Some("io.test.person/name".to_string()),
        },
        Field {
            name: "as".to_string(),
            value: Some("Text".to_string()),
        },
        Field {
            name: "cardinality".to_string(),
            value: Some("one".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify the attribute entity exists with the right dialog.attribute/id

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let attr_id = ClaimAttribute::from_str("dialog.attribute/id").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&ctx.branch, &ctx.operator, attr_id)
        .await
        .unwrap();

    // Should find at least the one we just created (plus bootstrapped ones)
    let mut found = false;
    for entity in &entities {
        let ids = carry::schema::fetch_string_values(
            &ctx.branch,
            &ctx.operator,
            entity,
            ClaimAttribute::from_str("dialog.attribute/id").unwrap(),
        )
        .await
        .unwrap();
        if ids.contains(&"io.test.person/name".to_string()) {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "Should find an attribute entity with dialog.attribute/id = io.test.person/name"
    );
}

/// Asserting an attribute with `@name` should store `dialog.meta/name` on the entity.
#[tokio::test]
async fn test_assert_attribute_with_name() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let fields = vec![
        Field {
            name: "the".to_string(),
            value: Some("io.test.person/name".to_string()),
        },
        Field {
            name: "as".to_string(),
            value: Some("Text".to_string()),
        },
        Field {
            name: "cardinality".to_string(),
            value: Some("one".to_string()),
        },
    ];
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("person-name".to_string()),
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify the attribute entity is discoverable by name

    let entity = carry::schema::lookup_entity_by_name(&ctx.branch, &ctx.operator, "person-name")
        .await
        .unwrap();
    assert!(
        entity.is_some(),
        "Attribute should be discoverable as 'person-name' via dialog.meta/name"
    );
}

/// Asserting an attribute without `cardinality` should default to `one`.
#[tokio::test]
async fn test_assert_attribute_defaults_cardinality() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let fields = vec![
        Field {
            name: "the".to_string(),
            value: Some("io.test.thing/color".to_string()),
        },
        Field {
            name: "as".to_string(),
            value: Some("Text".to_string()),
        },
    ];
    // This should succeed without cardinality — defaults to "one"
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // The entity should have been derived with cardinality=one
    let entity =
        carry::schema::derive_attribute_entity("io.test.thing/color", "Text", "one").unwrap();

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let card_attr = ClaimAttribute::from_str("dialog.attribute/cardinality").unwrap();
    let values = carry::schema::fetch_string_values(&ctx.branch, &ctx.operator, &entity, card_attr)
        .await
        .unwrap();
    assert_eq!(
        values,
        vec!["one".to_string()],
        "Cardinality should default to 'one'"
    );
}

/// Same attribute definition (same the/as/cardinality) should produce the
/// same entity DID deterministically.
#[tokio::test]
async fn test_assert_attribute_deterministic_entity() {
    let entity_a =
        carry::schema::derive_attribute_entity("io.test.person/name", "Text", "one").unwrap();
    let entity_b =
        carry::schema::derive_attribute_entity("io.test.person/name", "Text", "one").unwrap();
    assert_eq!(
        entity_a, entity_b,
        "Same attribute definition should produce same entity"
    );

    // Different type should produce different entity
    let entity_c =
        carry::schema::derive_attribute_entity("io.test.person/name", "UnsignedInteger", "one")
            .unwrap();
    assert_ne!(
        entity_a, entity_c,
        "Different value type should produce different entity"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Concept assertion
// ═══════════════════════════════════════════════════════════════════════════

/// Define two named attributes, then assert a concept referencing them by
/// bookmark name. The concept entity should exist with the correct
/// `dialog.concept.with/*` claims.
#[tokio::test]
async fn test_assert_concept_with_named_attrs() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Define attribute @test-name
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("test-name".to_string()),
        vec![
            Field {
                name: "the".to_string(),
                value: Some("io.test.person/name".to_string()),
            },
            Field {
                name: "as".to_string(),
                value: Some("Text".to_string()),
            },
            Field {
                name: "cardinality".to_string(),
                value: Some("one".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Define attribute @test-age
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("test-age".to_string()),
        vec![
            Field {
                name: "the".to_string(),
                value: Some("io.test.person/age".to_string()),
            },
            Field {
                name: "as".to_string(),
                value: Some("UnsignedInteger".to_string()),
            },
            Field {
                name: "cardinality".to_string(),
                value: Some("one".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Define concept @test-person with.name=test-name with.age=test-age
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("concept".to_string())),
        None,
        Some("test-person".to_string()),
        vec![
            Field {
                name: "with.name".to_string(),
                value: Some("test-name".to_string()),
            },
            Field {
                name: "with.age".to_string(),
                value: Some("test-age".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Verify concept is discoverable by name

    let concept_entity =
        carry::schema::lookup_entity_by_name(&ctx.branch, &ctx.operator, "test-person")
            .await
            .unwrap();
    assert!(
        concept_entity.is_some(),
        "Concept should be discoverable as 'test-person'"
    );

    // Verify concept resolves with correct fields
    let resolved = carry::schema::resolve_concept(&ctx.branch, &ctx.operator, "test-person")
        .await
        .unwrap();
    assert_eq!(
        resolved.with_fields.len(),
        2,
        "Concept should have 2 required fields"
    );
    assert!(
        resolved.with_fields.contains_key("name"),
        "Concept should have 'name' field"
    );
    assert!(
        resolved.with_fields.contains_key("age"),
        "Concept should have 'age' field"
    );

    // Verify the attribute selectors are correct
    let (_, name_selector) = &resolved.with_fields["name"];
    let (_, age_selector) = &resolved.with_fields["age"];
    assert_eq!(name_selector, "io.test.person/name");
    assert_eq!(age_selector, "io.test.person/age");
}

/// Asserting a concept with selector-style attribute values (containing '/')
/// should auto-create the attribute if it doesn't exist.
#[tokio::test]
async fn test_assert_concept_with_selector_attrs() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Define concept with inline selectors — attributes auto-created
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("concept".to_string())),
        None,
        Some("test-widget".to_string()),
        vec![Field {
            name: "with.color".to_string(),
            value: Some("io.test.widget/color".to_string()),
        }],
        "yaml",
    )
    .await
    .unwrap();

    // Verify concept resolves

    let resolved = carry::schema::resolve_concept(&ctx.branch, &ctx.operator, "test-widget")
        .await
        .unwrap();
    assert_eq!(resolved.with_fields.len(), 1);
    let (_, color_selector) = &resolved.with_fields["color"];
    assert_eq!(color_selector, "io.test.widget/color");
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Concept query
// ═══════════════════════════════════════════════════════════════════════════

/// Helper: set up a concept with two attributes and assert some data.
/// Returns the ctx for further querying.
async fn setup_concept_with_data() -> TestEnv {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Define attributes
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("cq-name".to_string()),
        vec![
            Field {
                name: "the".to_string(),
                value: Some("io.test.cq/name".to_string()),
            },
            Field {
                name: "as".to_string(),
                value: Some("Text".to_string()),
            },
            Field {
                name: "cardinality".to_string(),
                value: Some("one".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("cq-age".to_string()),
        vec![
            Field {
                name: "the".to_string(),
                value: Some("io.test.cq/age".to_string()),
            },
            Field {
                name: "as".to_string(),
                value: Some("UnsignedInteger".to_string()),
            },
            Field {
                name: "cardinality".to_string(),
                value: Some("one".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Define concept
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("concept".to_string())),
        None,
        Some("cq-person".to_string()),
        vec![
            Field {
                name: "with.name".to_string(),
                value: Some("cq-name".to_string()),
            },
            Field {
                name: "with.age".to_string(),
                value: Some("cq-age".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Assert data using the domain (so concept query can find it)
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.cq".to_string())),
        None,
        None,
        vec![
            Field {
                name: "name".to_string(),
                value: Some("Alice".to_string()),
            },
            Field {
                name: "age".to_string(),
                value: Some("28".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.cq".to_string())),
        None,
        None,
        vec![
            Field {
                name: "name".to_string(),
                value: Some("Bob".to_string()),
            },
            Field {
                name: "age".to_string(),
                value: Some("35".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    env
}

/// Concept query should resolve by name and return matching entities.
#[tokio::test]
async fn test_concept_query_resolves_by_name() {
    let env = setup_concept_with_data().await;
    let ctx = env.site();

    // Query using concept name — should succeed and find both entities
    carry::query_cmd::execute(
        ctx,
        Target::Concept("cq-person".to_string()),
        vec![],
        "yaml",
    )
    .await
    .unwrap();

    // Verify at the data level that the concept resolved correctly

    let resolved = carry::schema::resolve_concept(&ctx.branch, &ctx.operator, "cq-person")
        .await
        .unwrap();

    // Find entities that match the concept's attributes
    let selectors = carry::schema::concept_attribute_selectors(&resolved);
    let entities = carry::schema::find_entities_by_concept(&ctx.branch, &ctx.operator, &selectors)
        .await
        .unwrap();
    assert_eq!(
        entities.len(),
        2,
        "Should find 2 entities matching the concept"
    );
}

/// Concept query with a filter should narrow results.
#[tokio::test]
async fn test_concept_query_with_filter() {
    let env = setup_concept_with_data().await;
    let ctx = env.site();

    // Query with filter name=Alice
    carry::query_cmd::execute(
        ctx,
        Target::Concept("cq-person".to_string()),
        vec![Field {
            name: "name".to_string(),
            value: Some("Alice".to_string()),
        }],
        "yaml",
    )
    .await
    .unwrap();

    // The query itself printed output; verify at data level that only Alice matches

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.cq/name").unwrap();
    let all_entities =
        carry::schema::find_entities_by_attribute(&ctx.branch, &ctx.operator, name_attr.clone())
            .await
            .unwrap();

    let mut alice_count = 0;
    for entity in &all_entities {
        let names = carry::schema::fetch_string_values(
            &ctx.branch,
            &ctx.operator,
            entity,
            name_attr.clone(),
        )
        .await
        .unwrap();
        if names.contains(&"Alice".to_string()) {
            alice_count += 1;
        }
    }
    assert_eq!(alice_count, 1, "Should find exactly 1 Alice entity");
}

/// Querying a concept that doesn't exist should return an error.
#[tokio::test]
async fn test_concept_query_unknown_concept_fails() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    let result = carry::query_cmd::execute(
        ctx,
        Target::Concept("nonexistent-concept".to_string()),
        vec![],
        "yaml",
    )
    .await;

    assert!(
        result.is_err(),
        "Querying a nonexistent concept should fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "Error should mention concept not found, got: {}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: @name syntax
// ═══════════════════════════════════════════════════════════════════════════

/// Using @name with a domain assert should store dialog.meta/name on the entity.
#[tokio::test]
async fn test_domain_assert_with_name() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.person".to_string())),
        None,
        Some("alice".to_string()),
        vec![
            Field {
                name: "name".to_string(),
                value: Some("Alice".to_string()),
            },
            Field {
                name: "age".to_string(),
                value: Some("28".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // Verify the entity is discoverable by the @name

    let entity = carry::schema::lookup_entity_by_name(&ctx.branch, &ctx.operator, "alice")
        .await
        .unwrap();
    assert!(
        entity.is_some(),
        "Entity should be discoverable as 'alice' via dialog.meta/name"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Concept retract
// ═══════════════════════════════════════════════════════════════════════════

/// Retracting a field by concept field name should work: the concept's
/// field name is resolved to its attribute selector for the retraction.
#[tokio::test]
async fn test_retract_concept_field() {
    let env = setup_concept_with_data().await;
    let ctx = env.site();

    // Find Alice's entity DID

    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.cq/name").unwrap();
    let all_entities =
        carry::schema::find_entities_by_attribute(&ctx.branch, &ctx.operator, name_attr.clone())
            .await
            .unwrap();

    let mut alice_entity = None;
    for entity in &all_entities {
        let names = carry::schema::fetch_string_values(
            &ctx.branch,
            &ctx.operator,
            entity,
            name_attr.clone(),
        )
        .await
        .unwrap();
        if names.contains(&"Alice".to_string()) {
            alice_entity = Some(entity.clone());
            break;
        }
    }
    let alice_entity = alice_entity.expect("Alice should exist");

    // Retract age using concept field name
    carry::retract_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("cq-person".to_string())),
        Some(alice_entity.to_string()),
        vec![Field {
            name: "age".to_string(),
            value: None,
        }],
        "yaml",
    )
    .await
    .unwrap();

    // Verify age is gone but name remains

    let age_attr = ClaimAttribute::from_str("io.test.cq/age").unwrap();
    let age_values =
        carry::schema::fetch_string_values(&ctx.branch, &ctx.operator, &alice_entity, age_attr)
            .await
            .unwrap();
    assert!(
        age_values.is_empty(),
        "Age should have been retracted from Alice"
    );

    let name_values =
        carry::schema::fetch_string_values(&ctx.branch, &ctx.operator, &alice_entity, name_attr)
            .await
            .unwrap();
    assert_eq!(
        name_values,
        vec!["Alice".to_string()],
        "Name should still exist after retracting age"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Full round-trip
// ═══════════════════════════════════════════════════════════════════════════

/// Full round-trip: define attributes → define concept → assert data via
/// domain → query via concept name → verify all field names and values.
#[tokio::test]
async fn test_attribute_concept_data_roundtrip() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // 1. Define attributes
    for (name, selector, val_type) in [
        ("rt-title", "io.test.book/title", "Text"),
        ("rt-pages", "io.test.book/pages", "UnsignedInteger"),
    ] {
        carry::assert_cmd::execute(
            ctx,
            FirstArg::Target(Target::Concept("attribute".to_string())),
            None,
            Some(name.to_string()),
            vec![
                Field {
                    name: "the".to_string(),
                    value: Some(selector.to_string()),
                },
                Field {
                    name: "as".to_string(),
                    value: Some(val_type.to_string()),
                },
                Field {
                    name: "cardinality".to_string(),
                    value: Some("one".to_string()),
                },
            ],
            "yaml",
        )
        .await
        .unwrap();
    }

    // 2. Define concept
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Concept("concept".to_string())),
        None,
        Some("rt-book".to_string()),
        vec![
            Field {
                name: "with.title".to_string(),
                value: Some("rt-title".to_string()),
            },
            Field {
                name: "with.pages".to_string(),
                value: Some("rt-pages".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // 3. Assert data via domain
    carry::assert_cmd::execute(
        ctx,
        FirstArg::Target(Target::Domain("io.test.book".to_string())),
        None,
        None,
        vec![
            Field {
                name: "title".to_string(),
                value: Some("Moby Dick".to_string()),
            },
            Field {
                name: "pages".to_string(),
                value: Some("635".to_string()),
            },
        ],
        "yaml",
    )
    .await
    .unwrap();

    // 4. Verify concept resolves correctly

    let concept = carry::schema::resolve_concept(&ctx.branch, &ctx.operator, "rt-book")
        .await
        .unwrap();
    assert_eq!(concept.with_fields.len(), 2);
    assert_eq!(concept.with_fields["title"].1, "io.test.book/title");
    assert_eq!(concept.with_fields["pages"].1, "io.test.book/pages");

    // 5. Find entities matching the concept
    let selectors = carry::schema::concept_attribute_selectors(&concept);
    let entities = carry::schema::find_entities_by_concept(&ctx.branch, &ctx.operator, &selectors)
        .await
        .unwrap();
    assert_eq!(entities.len(), 1, "Should find exactly 1 book entity");

    // 6. Verify the data values
    use carry::schema::ClaimAttribute;
    use std::str::FromStr;

    let title_attr = ClaimAttribute::from_str("io.test.book/title").unwrap();
    let pages_attr = ClaimAttribute::from_str("io.test.book/pages").unwrap();

    let title_val =
        carry::schema::fetch_value(&ctx.branch, &ctx.operator, &entities[0], title_attr)
            .await
            .unwrap();
    assert_eq!(
        title_val.map(|v| carry::schema::format_value(&v)),
        Some("Moby Dick".to_string())
    );

    let pages_val =
        carry::schema::fetch_value(&ctx.branch, &ctx.operator, &entities[0], pages_attr)
            .await
            .unwrap();
    assert_eq!(
        pages_val.map(|v| carry::schema::format_value(&v)),
        Some("635".to_string())
    );

    // 7. Verify concept query works (doesn't error)
    carry::query_cmd::execute(ctx, Target::Concept("rt-book".to_string()), vec![], "yaml")
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Invite & Join
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_invite_creates_scoped_url() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Scoped invite: delegate to a specific DID
    let invite = carry::invite_cmd::create_invite(site, Some(&site.profile.did()), None)
        .await
        .unwrap();

    // URL should contain the access param but no fragment
    assert!(invite.url.contains("?access="));
    assert!(!invite.url.contains('#'));

    // Parse roundtrip should succeed; scoped invite has no embedded seed.
    let decoded = tonk_invite::Invite::parse_url(&invite.url).await.unwrap();
    assert!(decoded.chain.to_bytes().is_ok());
    assert!(matches!(
        decoded.audience,
        tonk_invite::InviteAudience::Scoped
    ));
}

#[tokio::test]
async fn test_invite_creates_open_url() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Open invite: no member DID, generates ephemeral keypair
    let invite = carry::invite_cmd::create_invite(site, None, None)
        .await
        .unwrap();

    // URL should contain both access param and fragment
    assert!(invite.url.contains("?access="));
    assert!(invite.url.contains('#'));

    // Parse roundtrip should succeed with an open audience carrying a seed.
    let decoded = tonk_invite::Invite::parse_url(&invite.url).await.unwrap();
    assert!(decoded.chain.to_bytes().is_ok());
    let seed = match &decoded.audience {
        tonk_invite::InviteAudience::Open { seed } => seed,
        tonk_invite::InviteAudience::Scoped => panic!("expected open invite"),
    };
    assert_eq!(seed.len(), 32);
}

#[tokio::test]
async fn test_invite_parse_rejects_bad_urls() {
    assert!(tonk_invite::Invite::parse_url("not_a_url").await.is_err());
    assert!(
        tonk_invite::Invite::parse_url("https://example.com")
            .await
            .is_err()
    );
    assert!(
        tonk_invite::Invite::parse_url("https://example.com?access=badbase58!")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_invite_join_roundtrip_scoped() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Scoped invite for our own DID
    let invite = carry::invite_cmd::create_invite(site, Some(&site.profile.did()), None)
        .await
        .unwrap();

    // Join with the URL in a new directory
    let join_dir = tempfile::TempDir::new().unwrap();
    carry::join_cmd::execute(
        Some(&invite.url),
        Some(join_dir.path()),
        Some(env.profile_location.clone()),
    )
    .await
    .unwrap();

    // Verify the joined site exists
    assert!(join_dir.path().join(".carry").is_dir());
}

#[tokio::test]
async fn test_invite_join_roundtrip_open() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Open invite (bearer token)
    let invite = carry::invite_cmd::create_invite(site, None, None)
        .await
        .unwrap();

    // Join with the URL in a new directory
    let join_dir = tempfile::TempDir::new().unwrap();
    carry::join_cmd::execute(
        Some(&invite.url),
        Some(join_dir.path()),
        Some(env.profile_location.clone()),
    )
    .await
    .unwrap();

    assert!(join_dir.path().join(".carry").is_dir());
}

#[tokio::test]
async fn test_invite_execute_succeeds() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // execute() prints to stdout
    carry::invite_cmd::execute(site, Some(site.profile.did().as_ref()), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_invite_custom_base_url() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    let invite =
        carry::invite_cmd::create_invite(site, None, Some("https://custom.example.com/join"))
            .await
            .unwrap();

    assert!(
        invite
            .url
            .starts_with("https://custom.example.com/join?access=")
    );

    let decoded = tonk_invite::Invite::parse_url(&invite.url).await.unwrap();
    // No remote configured in test, so remote_url should be None
    assert!(decoded.remote_url.is_none());
}

#[tokio::test]
async fn test_invite_urls_are_unique() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Each open invite should produce a different URL (different ephemeral key)
    let invite1 = carry::invite_cmd::create_invite(site, None, None)
        .await
        .unwrap();
    let invite2 = carry::invite_cmd::create_invite(site, None, None)
        .await
        .unwrap();
    assert_ne!(invite1.url, invite2.url);
}

// ═══════════════════════════════════════════════════════════════════════════
// Site stability
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_site_reopen_preserves_dids() {
    let tmp = tempfile::TempDir::new().unwrap();
    let loc = common::unique_dir("carry-test");
    let repo_loc = common::unique_dir("carry-test-repo");
    carry::init::execute(
        None,
        vec![],
        Some(tmp.path()),
        Some(loc.clone()),
        Some(repo_loc.clone()),
    )
    .await
    .unwrap();

    let site1 = carry::site::Site::open(tmp.path(), Some(loc.clone()), Some(repo_loc.clone()))
        .await
        .unwrap();
    let profile_did = site1.did();
    let repo_did = site1.repo_did();
    drop(site1);

    let site2 = carry::site::Site::open(tmp.path(), Some(loc), Some(repo_loc.clone()))
        .await
        .unwrap();
    assert_eq!(
        site2.did(),
        profile_did,
        "Profile DID should be stable across reopens"
    );
    assert_eq!(
        site2.repo_did(),
        repo_did,
        "Repo DID should be stable across reopens"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Join + data roundtrip
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_join_site_can_write_and_read_data() {
    let env = TestEnv::new().await.unwrap();
    let site = env.site();

    // Invite → join (use the same test-scoped profile)
    let invite = carry::invite_cmd::create_invite(site, Some(&site.profile.did()), None)
        .await
        .unwrap();
    let join_dir = tempfile::TempDir::new().unwrap();
    carry::join_cmd::execute(
        Some(&invite.url),
        Some(join_dir.path()),
        Some(env.profile_location.clone()),
    )
    .await
    .unwrap();

    // Open the joined site and bootstrap builtins on it
    let join_repo_loc = common::unique_dir("carry-test-join-repo");
    let joined = carry::site::Site::open(
        join_dir.path(),
        Some(env.profile_location.clone()),
        Some(join_repo_loc),
    )
    .await
    .unwrap();

    carry::schema::bootstrap_builtins(&joined.branch, &joined.operator)
        .await
        .unwrap();

    // Assert data on the joined site
    let target = FirstArg::Target(Target::Domain("test.join".to_string()));
    let fields = vec![Field {
        name: "name".to_string(),
        value: Some("Alice".to_string()),
    }];
    carry::assert_cmd::execute(&joined, target, None, None, fields, "yaml")
        .await
        .unwrap();

    // Query back
    let query_fields = vec![Field {
        name: "name".to_string(),
        value: None,
    }];
    let (results, _) = carry::query_cmd::query(
        &joined,
        Target::Domain("test.join".to_string()),
        query_fields,
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1, "Should find one entity");
    let entity_attrs = results.values().next().unwrap();
    let name_values = entity_attrs.get("test.join/name").unwrap();
    assert_eq!(name_values.len(), 1);
    assert_eq!(
        name_values[0],
        dialog_query::Value::String("Alice".to_string())
    );
}
