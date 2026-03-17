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
    carry::assert_cmd::execute(&ctx, target, None, None, fields, "yaml")
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
        None,
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    drop(session);

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
    carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // 6. Verify the data still exists (round-trip preserved it)
    let session = ctx.open_session().await.unwrap();
    let name_attr2 = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, name_attr2)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(carry::schema::format_value(&values[0]), "Alice");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_values = carry::schema::fetch_values(&session, &entity, age_attr)
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
        &ctx,
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
    carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // 4. Verify data is still intact
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, name_attr)
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
    carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let entity = dialog_query::Entity::from_str(entity_did).unwrap();
    let tag_attr = ClaimAttribute::from_str("io.test.person/tag").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, tag_attr)
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
    carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let entity = dialog_query::Entity::from_str(entity_did).unwrap();

    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(carry::schema::format_value(&values[0]), "Alice");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_values = carry::schema::fetch_values(&session, &entity, age_attr)
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let values = carry::schema::fetch_values(&session, &entity, age_attr.clone())
        .await
        .unwrap();
    assert_eq!(values.len(), 1);
    drop(session);

    // Now retract the age via EAV triple YAML file
    let yaml = format!("- the: io.test.person/age\n  of: {}\n  is: 28\n", entity);
    let (yaml_path, _tmp) = write_yaml_file(&yaml);
    carry::retract_cmd::execute(&ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify age is retracted
    let session = ctx.open_session().await.unwrap();
    let values = carry::schema::fetch_values(&session, &entity, age_attr)
        .await
        .unwrap();
    assert_eq!(values.len(), 0, "Age should be retracted");

    // Name should still exist
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let name_values = carry::schema::fetch_values(&session, &entity, name_attr)
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
        &ctx,
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

    carry::retract_cmd::execute(&ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify both fields are retracted
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let name_vals = carry::schema::fetch_values(&session, &entity, name_attr)
        .await
        .unwrap();
    assert_eq!(name_vals.len(), 0, "Name should be retracted");

    let age_attr = ClaimAttribute::from_str("io.test.person/age").unwrap();
    let age_vals = carry::schema::fetch_values(&session, &entity, age_attr)
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

    carry::assert_cmd::execute(&ctx, FirstArg::File(json_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify exists
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let entity = dialog_query::Entity::from_str(entity_did).unwrap();
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, name_attr.clone())
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    drop(session);

    // Retract via JSON
    let retract_json = format!(
        r#"[{{"the": "io.test.person/name", "of": "{}", "is": "Alice"}}]"#,
        entity_did
    );
    let tmp2 = tempfile::NamedTempFile::with_suffix(".json").unwrap();
    std::fs::write(tmp2.path(), &retract_json).unwrap();
    let json_path2 = tmp2.path().to_string_lossy().to_string();

    carry::retract_cmd::execute(&ctx, FirstArg::File(json_path2), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify retracted
    let session = ctx.open_session().await.unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, name_attr)
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
        &ctx,
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
    carry::retract_cmd::execute(&ctx, FirstArg::File(yaml_path), None, vec![], "yaml")
        .await
        .unwrap();

    // Verify both fields are retracted
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let name_attr = ClaimAttribute::from_str("io.test.person/name").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, name_attr)
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
    carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
        .await
        .unwrap();

    // Verify each value was stored
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;
    let session = ctx.open_session().await.unwrap();
    let entity = dialog_query::Entity::from_str(entity_did).unwrap();

    let text_attr = ClaimAttribute::from_str("io.test.data/text").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, text_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "hello world");

    let num_attr = ClaimAttribute::from_str("io.test.data/number").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, num_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "42");

    let neg_attr = ClaimAttribute::from_str("io.test.data/negative").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, neg_attr)
        .await
        .unwrap();
    assert_eq!(vals.len(), 1);
    assert_eq!(carry::schema::format_value(&vals[0]), "-7");

    let flag_attr = ClaimAttribute::from_str("io.test.data/flag").unwrap();
    let vals = carry::schema::fetch_values(&session, &entity, flag_attr)
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
        &ctx,
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
        &ctx,
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
        carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
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
        carry::retract_cmd::execute(&ctx, FirstArg::File(yaml_path), None, vec![], "yaml").await;
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
        carry::assert_cmd::execute(&ctx, FirstArg::File(yaml_path), None, None, vec![], "yaml")
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
        &ctx,
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
        &ctx,
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

// ═══════════════════════════════════════════════════════════════════════════
// Meta-schema: Bootstrap & Init
// ═══════════════════════════════════════════════════════════════════════════

/// After init, the pre-registered concepts (attribute, concept, bookmark)
/// should be discoverable by name via `dialog.meta/name`.
#[tokio::test]
async fn test_init_bootstraps_builtins() {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;
    let session = ctx.open_session().await.unwrap();

    for name in &["attribute", "concept", "bookmark"] {
        let entity = carry::schema::lookup_entity_by_name(&session, name)
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
    let session = ctx.open_session().await.unwrap();

    let concept = carry::schema::resolve_concept(&session, "attribute")
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
        &ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        None,
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify the attribute entity exists with the right dialog.attribute/id
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let attr_id = ClaimAttribute::from_str("dialog.attribute/id").unwrap();
    let entities = carry::schema::find_entities_by_attribute(&session, attr_id)
        .await
        .unwrap();

    // Should find at least the one we just created (plus bootstrapped ones)
    let mut found = false;
    for entity in &entities {
        let ids = carry::schema::fetch_string_values(
            &session,
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
        &ctx,
        FirstArg::Target(Target::Concept("attribute".to_string())),
        None,
        Some("person-name".to_string()),
        fields,
        "yaml",
    )
    .await
    .unwrap();

    // Verify the attribute entity is discoverable by name
    let session = ctx.open_session().await.unwrap();
    let entity = carry::schema::lookup_entity_by_name(&session, "person-name")
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
        &ctx,
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

    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let card_attr = ClaimAttribute::from_str("dialog.attribute/cardinality").unwrap();
    let values = carry::schema::fetch_string_values(&session, &entity, card_attr)
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
        &ctx,
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
        &ctx,
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    let concept_entity = carry::schema::lookup_entity_by_name(&session, "test-person")
        .await
        .unwrap();
    assert!(
        concept_entity.is_some(),
        "Concept should be discoverable as 'test-person'"
    );

    // Verify concept resolves with correct fields
    let resolved = carry::schema::resolve_concept(&session, "test-person")
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    let resolved = carry::schema::resolve_concept(&session, "test-widget")
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
async fn setup_concept_with_data() -> (TestEnv, carry::site::SiteContext) {
    let env = TestEnv::new().await.unwrap();
    let ctx = env.ctx().await;

    // Define attributes
    carry::assert_cmd::execute(
        &ctx,
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
        &ctx,
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
        &ctx,
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
        &ctx,
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
        &ctx,
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

    (env, ctx)
}

/// Concept query should resolve by name and return matching entities.
#[tokio::test]
async fn test_concept_query_resolves_by_name() {
    let (_env, ctx) = setup_concept_with_data().await;

    // Query using concept name — should succeed and find both entities
    carry::query_cmd::execute(
        &ctx,
        Target::Concept("cq-person".to_string()),
        vec![],
        "yaml",
    )
    .await
    .unwrap();

    // Verify at the data level that the concept resolved correctly
    let session = ctx.open_session().await.unwrap();
    let resolved = carry::schema::resolve_concept(&session, "cq-person")
        .await
        .unwrap();

    // Find entities that match the concept's attributes
    let selectors = carry::schema::concept_attribute_selectors(&resolved);
    let entities = carry::schema::find_entities_by_concept(&session, &selectors)
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
    let (_env, ctx) = setup_concept_with_data().await;

    // Query with filter name=Alice
    carry::query_cmd::execute(
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.cq/name").unwrap();
    let all_entities = carry::schema::find_entities_by_attribute(&session, name_attr.clone())
        .await
        .unwrap();

    let mut alice_count = 0;
    for entity in &all_entities {
        let names = carry::schema::fetch_string_values(&session, entity, name_attr.clone())
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
        &ctx,
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    let entity = carry::schema::lookup_entity_by_name(&session, "alice")
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
    let (_env, ctx) = setup_concept_with_data().await;

    // Find Alice's entity DID
    let session = ctx.open_session().await.unwrap();
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let name_attr = ClaimAttribute::from_str("io.test.cq/name").unwrap();
    let all_entities = carry::schema::find_entities_by_attribute(&session, name_attr.clone())
        .await
        .unwrap();

    let mut alice_entity = None;
    for entity in &all_entities {
        let names = carry::schema::fetch_string_values(&session, entity, name_attr.clone())
            .await
            .unwrap();
        if names.contains(&"Alice".to_string()) {
            alice_entity = Some(entity.clone());
            break;
        }
    }
    let alice_entity = alice_entity.expect("Alice should exist");
    drop(session);

    // Retract age using concept field name
    carry::retract_cmd::execute(
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    let age_attr = ClaimAttribute::from_str("io.test.cq/age").unwrap();
    let age_values = carry::schema::fetch_string_values(&session, &alice_entity, age_attr)
        .await
        .unwrap();
    assert!(
        age_values.is_empty(),
        "Age should have been retracted from Alice"
    );

    let name_values = carry::schema::fetch_string_values(&session, &alice_entity, name_attr)
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
            &ctx,
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
        &ctx,
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
        &ctx,
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
    let session = ctx.open_session().await.unwrap();
    let concept = carry::schema::resolve_concept(&session, "rt-book")
        .await
        .unwrap();
    assert_eq!(concept.with_fields.len(), 2);
    assert_eq!(concept.with_fields["title"].1, "io.test.book/title");
    assert_eq!(concept.with_fields["pages"].1, "io.test.book/pages");

    // 5. Find entities matching the concept
    let selectors = carry::schema::concept_attribute_selectors(&concept);
    let entities = carry::schema::find_entities_by_concept(&session, &selectors)
        .await
        .unwrap();
    assert_eq!(entities.len(), 1, "Should find exactly 1 book entity");

    // 6. Verify the data values
    use dialog_query::claim::Attribute as ClaimAttribute;
    use std::str::FromStr;

    let title_attr = ClaimAttribute::from_str("io.test.book/title").unwrap();
    let pages_attr = ClaimAttribute::from_str("io.test.book/pages").unwrap();

    let title_val = carry::schema::fetch_value(&session, &entities[0], title_attr)
        .await
        .unwrap();
    assert_eq!(
        title_val.map(|v| carry::schema::format_value(&v)),
        Some("Moby Dick".to_string())
    );

    let pages_val = carry::schema::fetch_value(&session, &entities[0], pages_attr)
        .await
        .unwrap();
    assert_eq!(
        pages_val.map(|v| carry::schema::format_value(&v)),
        Some("635".to_string())
    );

    // 7. Verify concept query works (doesn't error)
    carry::query_cmd::execute(&ctx, Target::Concept("rt-book".to_string()), vec![], "yaml")
        .await
        .unwrap();
}
