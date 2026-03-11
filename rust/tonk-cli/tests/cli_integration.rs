//! Integration tests for the tonk CLI.
//!
//! These tests exercise the CLI through its library API, using isolated
//! filesystem environments for each test. Every test gets its own temporary
//! directory set as `TONK_HOME` (the tonk data directory, equivalent to
//! `~/.tonk/`) with a programmatically bootstrapped session.
//!
//! Tests are marked `#[serial]` because they modify process-global env vars
//! (`TONK_HOME`, `TONK_OPERATOR_KEY`).

mod common;

use common::TestEnv;
use serial_test::serial;

// ═══════════════════════════════════════════════════════════════════════════
// Bootstrap & Status
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_operator_generate_produces_valid_did() {
    // operator::generate() just prints to stdout; test that Operator::generate
    // produces a valid did:key DID.
    let operator = tonk_cli::crypto::Operator::generate();
    let did = operator.did().to_string();
    assert!(
        did.starts_with("did:key:z"),
        "DID should start with did:key:z, got: {}",
        did
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Delegation round-trip
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_delegation_roundtrip() {
    // Verify that delegations built by TestEnv survive a CBOR serialization
    // round-trip and are recognized as valid powerline delegations.
    use dialog_credentials::Ed25519Signer;
    use dialog_ucan::Delegation as UcanDelegation;
    use dialog_ucan::subject::Subject;
    use dialog_varsig::Did;
    use dialog_varsig::eddsa::Ed25519Signature;
    use tonk_cli::delegation::Delegation;

    let authority = tonk_cli::crypto::Operator::generate();
    let operator = tonk_cli::crypto::Operator::generate();

    let authority_signer = Ed25519Signer::from(&authority);
    let operator_did: Did = operator.did().to_string().parse().unwrap();

    let ucan: UcanDelegation<Ed25519Signature> = UcanDelegation::builder()
        .issuer(authority_signer)
        .audience(&operator_did)
        .subject(Subject::Any)
        .command(vec![]) // Empty = root access "/"
        .try_build()
        .await
        .unwrap();

    let delegation = Delegation::from_ucan(ucan);
    assert!(delegation.is_powerline(), "Should be powerline");
    assert!(delegation.is_valid(), "Should be valid");
    assert_eq!(delegation.expiration(), None, "No expiration set");

    // Verify CBOR round-trip
    let cbor = delegation.to_cbor_bytes().expect("serialize");
    let reparsed = Delegation::from_cbor_bytes(&cbor).expect("deserialize");
    assert!(
        reparsed.is_powerline(),
        "Should be powerline after roundtrip"
    );
    assert!(reparsed.is_valid(), "Should be valid after roundtrip");
    assert_eq!(reparsed.issuer(), delegation.issuer());
    assert_eq!(reparsed.audience(), delegation.audience());
}

#[tokio::test]
#[serial]
async fn test_bootstrap_creates_session() {
    let env = TestEnv::new().await.expect("Failed to create test env");

    // Verify active session is set
    let session = tonk_cli::state::get_active_session().expect("Failed to get active session");
    assert_eq!(session, Some(env.authority_did.clone()));

    // Verify the keystore returns the same operator
    let keystore = tonk_cli::keystore::Keystore::new().expect("keystore");
    let op = keystore.get_or_create_keypair().expect("keypair");
    assert_eq!(
        op.did().to_string(),
        env.operator_did,
        "Keystore should return same operator"
    );

    // Verify the authority is discoverable
    let authorities = tonk_cli::authority::get_authorities().expect("Failed to get authorities");
    assert!(
        !authorities.is_empty(),
        "Should have at least one authority"
    );
    assert_eq!(authorities[0].did, env.authority_did);
}

#[tokio::test]
#[serial]
async fn test_status_with_space() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space_did = env
        .create_space("test-space")
        .await
        .expect("Failed to create space");

    // status::execute should not error when a space is active
    let result = tonk_cli::status::execute(true).await;
    assert!(result.is_ok(), "status should succeed: {:?}", result.err());
}

#[tokio::test]
#[serial]
async fn test_status_json_output() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space_did = env
        .create_space("json-space")
        .await
        .expect("Failed to create space");

    // JSON mode should not error
    let result = tonk_cli::status::execute(true).await;
    assert!(
        result.is_ok(),
        "status --json should succeed: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Space Management
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_space_create_and_list() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let space_did = env
        .create_space("my-space")
        .await
        .expect("Failed to create space");

    assert!(
        space_did.starts_with("did:key:"),
        "Space DID should be a did:key"
    );

    // List spaces (JSON mode) — should contain our space
    // We can verify via state
    let spaces = tonk_cli::state::list_spaces_for_session(&env.authority_did)
        .expect("Failed to list spaces");
    assert!(
        spaces.contains(&space_did),
        "Space should be in session list"
    );
}

#[tokio::test]
#[serial]
async fn test_space_create_sets_active() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let space_did = env
        .create_space("active-space")
        .await
        .expect("Failed to create space");

    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(active, Some(space_did));
}

#[tokio::test]
#[serial]
async fn test_space_create_multiple_and_switch() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let space1 = env
        .create_space("space-one")
        .await
        .expect("Failed to create space 1");
    let space2 = env
        .create_space("space-two")
        .await
        .expect("Failed to create space 2");

    // space-two should be active (most recently created)
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(active, Some(space2.clone()));

    // Switch back to space-one by name
    tonk_cli::space::load("space-one".to_string())
        .await
        .expect("Failed to load space");

    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(active, Some(space1));
}

#[tokio::test]
#[serial]
async fn test_space_current() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space_did = env
        .create_space("current-space")
        .await
        .expect("Failed to create space");

    // show_current should not error
    let result = tonk_cli::space::show_current(true).await;
    assert!(
        result.is_ok(),
        "space current should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_space_delete() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space1 = env
        .create_space("keep-me")
        .await
        .expect("Failed to create space 1");
    let space2 = env
        .create_space("delete-me")
        .await
        .expect("Failed to create space 2");

    // Switch to space 1 so we're deleting a non-active space
    tonk_cli::space::load("keep-me".to_string())
        .await
        .expect("Failed to load space");

    // Delete space 2 with force (skip confirmation)
    tonk_cli::space::delete("delete-me".to_string(), true)
        .await
        .expect("Failed to delete space");

    let spaces = tonk_cli::state::list_spaces_for_session(&env.authority_did)
        .expect("Failed to list spaces");
    assert!(
        !spaces.contains(&space2),
        "Deleted space should not be in list"
    );
}

#[tokio::test]
#[serial]
async fn test_space_delete_active_clears_active() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let space_did = env
        .create_space("doomed-space")
        .await
        .expect("Failed to create space");

    // Verify it's active
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(active, Some(space_did.clone()));

    // Delete the active space
    tonk_cli::space::delete("doomed-space".to_string(), true)
        .await
        .expect("Failed to delete space");

    // Active space should be cleared
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(active, None);
}

#[tokio::test]
#[serial]
async fn test_space_create_duplicate_name_fails() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("dup-name")
        .await
        .expect("Failed to create first space");

    // Creating a second space with the same name should fail
    let result = tonk_cli::space::create("dup-name".to_string(), Some(vec![]), None, true).await;
    assert!(
        result.is_err(),
        "creating a space with a duplicate name should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("already exists"),
        "error should mention 'already exists', got: {}",
        err_msg
    );
}

#[tokio::test]
#[serial]
async fn test_space_load_nonexistent_fails() {
    let _env = TestEnv::new().await.expect("Failed to create test env");

    let result = tonk_cli::space::load("no-such-space".to_string()).await;
    assert!(result.is_err(), "loading a nonexistent space should fail");
}

#[tokio::test]
#[serial]
async fn test_space_open_creates_when_missing() {
    let env = TestEnv::new().await.expect("Failed to create test env");

    // open should create the space when it doesn't exist
    let result = tonk_cli::space::open("fresh-space".to_string(), Some(vec![]), None, true).await;
    assert!(
        result.is_ok(),
        "open should succeed when space doesn't exist: {:?}",
        result.err()
    );

    // Verify it was created and set as active
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert!(active.is_some(), "space should be active after open");
}

#[tokio::test]
#[serial]
async fn test_space_open_loads_when_exists() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let original_did = env
        .create_space("existing-space")
        .await
        .expect("Failed to create space");

    // Create a second space so the active one changes
    let _other = env
        .create_space("other-space")
        .await
        .expect("Failed to create second space");
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_ne!(
        active,
        Some(original_did.clone()),
        "active space should be the second one now"
    );

    // open should load the existing space, not create a new one
    let result =
        tonk_cli::space::open("existing-space".to_string(), Some(vec![]), None, true).await;
    assert!(
        result.is_ok(),
        "open should succeed for existing space: {:?}",
        result.err()
    );

    // Active space should be the original one
    let active =
        tonk_cli::state::get_active_space(&env.authority_did).expect("Failed to get active space");
    assert_eq!(
        active,
        Some(original_did),
        "open should have loaded the original space, not created a new one"
    );
}

#[tokio::test]
#[serial]
async fn test_space_open_is_idempotent() {
    let env = TestEnv::new().await.expect("Failed to create test env");

    // First open creates
    tonk_cli::space::open("idem-space".to_string(), Some(vec![]), None, true)
        .await
        .expect("First open should succeed");

    let first_did = tonk_cli::state::get_active_space(&env.authority_did)
        .expect("get active")
        .expect("should be set");

    // Second open loads the same space
    tonk_cli::space::open("idem-space".to_string(), Some(vec![]), None, true)
        .await
        .expect("Second open should succeed");

    let second_did = tonk_cli::state::get_active_space(&env.authority_did)
        .expect("get active")
        .expect("should be set");

    assert_eq!(
        first_did, second_did,
        "open called twice should yield the same space DID"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Concept Management
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_concept_define_and_list() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("concept-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define a concept
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to track".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // List concepts — should not error and should find our concept
    let result = tonk_cli::concept::list(&ctx, true).await;
    assert!(
        result.is_ok(),
        "concept list should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_concept_show() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("show-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Contact".to_string(),
        vec!["name".to_string(), "email".to_string(), "phone".to_string()],
        "A contact entry".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Show the concept
    let result = tonk_cli::concept::show(&ctx, "Contact".to_string(), true).await;
    assert!(
        result.is_ok(),
        "concept show should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_concept_extend() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("extend-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Note".to_string(),
        vec!["title".to_string()],
        "A simple note".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Extend with new attributes
    tonk_cli::concept::extend(
        &ctx,
        "Note".to_string(),
        vec!["body".to_string(), "tags".to_string()],
        true,
    )
    .await
    .expect("Failed to extend concept");

    // Show should succeed and include new attributes
    let result = tonk_cli::concept::show(&ctx, "Note".to_string(), true).await;
    assert!(result.is_ok(), "concept show after extend should succeed");
}

#[tokio::test]
#[serial]
async fn test_concept_delete() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("delete-concept-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Temporary".to_string(),
        vec!["data".to_string()],
        "A temporary concept".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Delete it
    tonk_cli::concept::delete(&ctx, "Temporary".to_string(), false, true)
        .await
        .expect("Failed to delete concept");

    // Show should fail (concept no longer exists)
    let result = tonk_cli::concept::show(&ctx, "Temporary".to_string(), true).await;
    assert!(result.is_err(), "concept show after delete should fail");
}

#[tokio::test]
#[serial]
async fn test_concept_name_validation() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("validation-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Invalid name with special characters
    let result = tonk_cli::concept::define(
        &ctx,
        "bad name!".to_string(),
        vec!["x".to_string()],
        "Test concept".to_string(),
        true,
    )
    .await;
    assert!(result.is_err(), "concept with special chars should fail");

    // Empty name
    let result = tonk_cli::concept::define(
        &ctx,
        "".to_string(),
        vec!["x".to_string()],
        "Test concept".to_string(),
        true,
    )
    .await;
    assert!(result.is_err(), "concept with empty name should fail");
}

#[tokio::test]
#[serial]
async fn test_concept_duplicate_rejected() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("dup-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Unique".to_string(),
        vec!["a".to_string()],
        "A unique concept".to_string(),
        true,
    )
    .await
    .expect("Failed to define first concept");

    // Defining same name with different attributes returns a conflict (not an error)
    // in JSON mode — the caller decides how to resolve it.
    let result = tonk_cli::concept::define(
        &ctx,
        "Unique".to_string(),
        vec!["b".to_string()],
        "Another unique concept".to_string(),
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "conflict define should succeed (prints conflict JSON)"
    );

    // The original concept should still be intact (conflict doesn't modify it)
    let session = tonk_cli::schema::open_session(&ctx)
        .await
        .expect("Failed to open session");
    let entity = tonk_cli::schema::lookup_concept_by_name(
        &session,
        &tonk_cli::schema::ConceptName::new("Unique".to_string()).unwrap(),
    )
    .await
    .expect("Failed to lookup concept");
    assert!(entity.is_some(), "original concept should still exist");

    let attrs = tonk_cli::schema::fetch_string_values(
        &session,
        &entity.unwrap(),
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch attributes");
    assert_eq!(
        attrs.len(),
        1,
        "original concept should still have 1 attribute"
    );
    assert!(
        attrs[0].ends_with("/a"),
        "original attribute should be unchanged"
    );
}

#[tokio::test]
#[serial]
async fn test_concept_define_converges() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("converge-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define a concept
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Re-defining with identical attributes should converge (noop)
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task".to_string(),
        true,
    )
    .await
    .expect("Convergent re-define should succeed");

    // Concept should still exist with the same attributes
    let session = tonk_cli::schema::open_session(&ctx)
        .await
        .expect("Failed to open session");
    let entity = tonk_cli::schema::lookup_concept_by_name(
        &session,
        &tonk_cli::schema::ConceptName::new("Task".to_string()).unwrap(),
    )
    .await
    .expect("Failed to lookup concept");
    assert!(
        entity.is_some(),
        "concept should still exist after convergent define"
    );

    let attrs = tonk_cli::schema::fetch_string_values(
        &session,
        &entity.unwrap(),
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch attributes");
    assert_eq!(attrs.len(), 2, "concept should still have 2 attributes");
}

// ═══════════════════════════════════════════════════════════════════════════
// Import
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_import_minimal_yaml() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-minimal")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    let file = TestEnv::example_file("minimal.yaml");
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import minimal.yaml");

    // Verify Task concept was created
    let result = tonk_cli::concept::show(&ctx, "Task".to_string(), true).await;
    assert!(result.is_ok(), "Task concept should exist after import");
}

#[tokio::test]
#[serial]
async fn test_import_cook_yaml() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-cook")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    let file = TestEnv::example_file("cook.yaml");
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Verify all 3 concepts were created
    for name in &["Recipe", "Ingredient", "RecipeStep"] {
        let result = tonk_cli::concept::show(&ctx, name.to_string(), true).await;
        assert!(result.is_ok(), "{} concept should exist after import", name);
    }
}

#[tokio::test]
#[serial]
async fn test_import_planner_yaml() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-planner")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Import cook.yaml first (planner rules reference cook concepts)
    let cook_file = TestEnv::example_file("cook.yaml");
    tonk_cli::import::import(&ctx, cook_file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // planner.yaml is a mixed file: concepts + rules
    let file = TestEnv::example_file("planner.yaml");
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import planner.yaml");

    // Verify concepts across both namespaces
    for name in &["Allergy", "Event", "Meal", "AllergyConflict"] {
        let result = tonk_cli::concept::show(&ctx, name.to_string(), true).await;
        assert!(result.is_ok(), "{} concept should exist after import", name);
    }
}

#[tokio::test]
#[serial]
async fn test_import_rules_yaml() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-rules")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Import prerequisite concepts first (cook before planner, since
    // planner's rules reference cook concepts)
    let cook_file = TestEnv::example_file("cook.yaml");
    tonk_cli::import::import(&ctx, cook_file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    let planner_file = TestEnv::example_file("planner.yaml");
    tonk_cli::import::import(&ctx, planner_file, false, true)
        .await
        .expect("Failed to import planner.yaml");

    // Now import rules (standalone rules file)
    let rules_file = TestEnv::example_file("rules.yaml");
    tonk_cli::import::import(&ctx, rules_file, false, true)
        .await
        .expect("Failed to import rules.yaml");

    // Verify rules were created
    let result = tonk_cli::rule::list(&ctx, true).await;
    assert!(result.is_ok(), "rule list should succeed after import");
}

#[tokio::test]
#[serial]
async fn test_import_force_overwrite() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-force")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    let file = TestEnv::example_file("minimal.yaml");

    // First import
    tonk_cli::import::import(&ctx, file.clone(), false, true)
        .await
        .expect("Failed to first import");

    // Second import without force should fail
    let result = tonk_cli::import::import(&ctx, file.clone(), false, true).await;
    assert!(result.is_err(), "re-import without --force should fail");

    // Second import with force should succeed
    tonk_cli::import::import(&ctx, file, true, true)
        .await
        .expect("Failed to force re-import");
}

#[tokio::test]
#[serial]
async fn test_import_nonexistent_file_errors() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("import-nofile")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    let result =
        tonk_cli::import::import(&ctx, "/nonexistent/path.yaml".to_string(), false, true).await;
    assert!(result.is_err(), "importing nonexistent file should fail");
}

// ═══════════════════════════════════════════════════════════════════════════
// Entity CRUD
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_create_entity() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("entity-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define a concept first
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create an entity
    let result = tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec!["title=Fix bug".to_string(), "status=todo".to_string()],
        None,
        false,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "create entity should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_query_entities() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("query-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Item".to_string(),
        vec!["name".to_string()],
        "A named item".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create 3 entities
    for name in &["Apple", "Banana", "Cherry"] {
        tonk_cli::entity::create(
            &ctx,
            "Item".to_string(),
            vec![format!("name={}", name)],
            None,
            false,
            true,
        )
        .await
        .expect("Failed to create entity");
    }

    // Query all entities
    let rows = tonk_cli::entity::query(&ctx, "Item".to_string(), vec![], true)
        .await
        .expect("query should succeed");
    assert_eq!(rows.len(), 3, "should return all 3 entities");

    // Verify each row has a non-null "name" attribute
    for row in &rows {
        let name_val = row.data.get("name").expect("row should have 'name' key");
        assert!(
            !name_val.is_null(),
            "name should not be null for entity {}",
            row.id
        );
    }
}

#[tokio::test]
#[serial]
async fn test_query_with_filter() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("filter-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create tasks with different statuses
    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec!["title=Task A".to_string(), "status=todo".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task A");

    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec!["title=Task B".to_string(), "status=done".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task B");

    // Query with filter
    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=todo".to_string()],
        true,
    )
    .await
    .expect("filtered query should succeed");

    assert_eq!(rows.len(), 1, "filter should match exactly 1 entity");
    assert_eq!(
        rows[0].data.get("title").and_then(|v| v.as_str()),
        Some("Task A"),
        "filtered entity should be Task A"
    );
    // The filtered attribute itself must be populated (not null).
    // This is the regression check for the "Unbound variable" bug.
    assert_eq!(
        rows[0].data.get("status").and_then(|v| v.as_str()),
        Some("todo"),
        "filtered attribute 'status' must be populated, not null"
    );
}

#[tokio::test]
#[serial]
async fn test_update_entity() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("update-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create an entity — we need to capture the ID.
    // entity::create in JSON mode prints JSON with the ID to stdout.
    // We'll use the fact layer to find it after creation.
    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec!["title=Update me".to_string(), "status=todo".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task");

    // Find the entity via query (we know there's exactly one)
    // Use the schema module to query the entity
    let branch = tonk_cli::schema::open_branch(&ctx)
        .await
        .expect("Failed to open branch");
    let concept_name = tonk_cli::schema::ConceptName::new("Task").unwrap();
    let concept_entity = tonk_cli::schema::lookup_concept_by_name(&branch, &concept_name)
        .await
        .expect("Failed to lookup concept")
        .expect("Concept 'Task' not found");
    let attrs = tonk_cli::schema::fetch_string_values(
        &branch,
        &concept_entity,
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch concept attributes");
    let entities = tonk_cli::schema::find_entities_by_concept(&branch, &attrs)
        .await
        .expect("Failed to find entities");

    assert_eq!(entities.len(), 1, "Should have exactly 1 entity");
    let entity_id = entities[0].to_string();

    // Update the entity
    let result = tonk_cli::entity::assert(
        &ctx,
        entity_id.clone(),
        vec!["status=done".to_string()],
        true,
    )
    .await;
    assert!(result.is_ok(), "update should succeed: {:?}", result.err());

    // Show should reflect the update
    let result = tonk_cli::entity::show(&ctx, entity_id, true).await;
    assert!(result.is_ok(), "show after update should succeed");
}

#[tokio::test]
#[serial]
async fn test_delete_entity() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("delete-inst-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec!["title=Delete me".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task");

    // Find the entity ID
    let branch = tonk_cli::schema::open_branch(&ctx)
        .await
        .expect("Failed to open branch");
    let concept_name = tonk_cli::schema::ConceptName::new("Task").unwrap();
    let concept_entity = tonk_cli::schema::lookup_concept_by_name(&branch, &concept_name)
        .await
        .expect("Failed to lookup concept")
        .expect("Concept 'Task' not found");
    let attrs = tonk_cli::schema::fetch_string_values(
        &branch,
        &concept_entity,
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch concept attributes");
    let entities = tonk_cli::schema::find_entities_by_concept(&branch, &attrs)
        .await
        .expect("Failed to find entities");
    let entity_id = entities[0].to_string();

    // Delete it
    tonk_cli::entity::retract(&ctx, entity_id, true)
        .await
        .expect("Failed to delete entity");

    // Query should return no entities
    let rows = tonk_cli::entity::query(&ctx, "Task".to_string(), vec![], true)
        .await
        .expect("query after delete should succeed");
    assert!(rows.is_empty(), "should have 0 entities after deletion");
}

#[tokio::test]
#[serial]
async fn test_create_entity_from_json_file() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("json-file-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create a temporary JSON file
    let json_path = env.home_path.join("entity.json");
    std::fs::write(&json_path, r#"{"title": "From file", "status": "todo"}"#)
        .expect("Failed to write JSON file");

    let result = tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec![],
        Some(json_path.to_string_lossy().into_owned()),
        false,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "create from JSON file should succeed: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Query result verification (regression tests for filter resolution)
// ═══════════════════════════════════════════════════════════════════════════

/// Helper: define a 3-attribute Task concept and create 3 entities with
/// distinct status and priority values. Returns the SpaceContext for
/// further queries.
async fn setup_task_entities(env: &TestEnv, space_name: &str) -> tonk_cli::schema::SpaceContext {
    let _space = env
        .create_space(space_name)
        .await
        .expect("Failed to create space");
    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec![
            "title".to_string(),
            "status".to_string(),
            "priority".to_string(),
        ],
        "A task for query tests".to_string(),
        true,
    )
    .await
    .expect("Failed to define Task concept");

    for (title, status, priority) in [
        ("Fix login bug", "todo", "high"),
        ("Write tests", "done", "medium"),
        ("Deploy to staging", "in-progress", "high"),
    ] {
        tonk_cli::entity::create(
            &ctx,
            "Task".to_string(),
            vec![
                format!("title={}", title),
                format!("status={}", status),
                format!("priority={}", priority),
            ],
            None,
            false,
            true,
        )
        .await
        .unwrap_or_else(|e| panic!("Failed to create task '{}': {}", title, e));
    }

    ctx
}

#[tokio::test]
#[serial]
async fn test_query_filter_returns_correct_count() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "filter-count-space").await;

    // status=todo matches 1 entity
    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=todo".to_string()],
        true,
    )
    .await
    .expect("query with status=todo should succeed");
    assert_eq!(rows.len(), 1, "status=todo should match 1 entity");

    // priority=high matches 2 entities
    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["priority=high".to_string()],
        true,
    )
    .await
    .expect("query with priority=high should succeed");
    assert_eq!(rows.len(), 2, "priority=high should match 2 entities");
}

#[tokio::test]
#[serial]
async fn test_query_filter_populates_filtered_attribute() {
    // Regression test: filtered attributes must appear with their actual
    // value in the result rows, not as null. Before the fix, dialog-db's
    // merge_parameters skipped constants, leaving the filtered variable
    // unbound in the Answer — causing "Unbound variable" warnings and null.
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "filter-populate-space").await;

    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=todo".to_string()],
        true,
    )
    .await
    .expect("filtered query should succeed");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    // The filtered attribute "status" must be the filter value, not null
    let status = row
        .data
        .get("status")
        .expect("row should have 'status' key");
    assert!(
        !status.is_null(),
        "filtered attribute 'status' must not be null"
    );
    assert_eq!(
        status.as_str(),
        Some("todo"),
        "filtered attribute 'status' should equal the filter value"
    );

    // Non-filtered attributes must also be populated
    let title = row.data.get("title").expect("row should have 'title' key");
    assert!(
        !title.is_null(),
        "non-filtered attribute 'title' must not be null"
    );
    assert_eq!(title.as_str(), Some("Fix login bug"));

    let priority = row
        .data
        .get("priority")
        .expect("row should have 'priority' key");
    assert!(
        !priority.is_null(),
        "non-filtered attribute 'priority' must not be null"
    );
    assert_eq!(priority.as_str(), Some("high"));
}

#[tokio::test]
#[serial]
async fn test_query_filter_populates_all_attributes() {
    // Every attribute in every returned row must be non-null, regardless
    // of whether it was used as a filter.
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "filter-all-attrs-space").await;

    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["priority=high".to_string()],
        true,
    )
    .await
    .expect("filtered query should succeed");

    assert_eq!(rows.len(), 2, "priority=high matches 2 entities");

    for row in &rows {
        for attr in &["title", "status", "priority"] {
            let val = row
                .data
                .get(*attr)
                .unwrap_or_else(|| panic!("row {} missing attribute '{}'", row.id, attr));
            assert!(
                !val.is_null(),
                "attribute '{}' is null for entity {} — possible unbound variable regression",
                attr,
                row.id
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn test_query_multi_filter() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "multi-filter-space").await;

    // Filter on two attributes simultaneously
    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=todo".to_string(), "priority=high".to_string()],
        true,
    )
    .await
    .expect("multi-filter query should succeed");

    assert_eq!(rows.len(), 1, "only 1 entity matches both filters");
    let row = &rows[0];
    assert_eq!(
        row.data.get("title").and_then(|v| v.as_str()),
        Some("Fix login bug")
    );

    // Both filtered attributes must be populated
    assert_eq!(
        row.data.get("status").and_then(|v| v.as_str()),
        Some("todo"),
        "filtered attribute 'status' must be populated"
    );
    assert_eq!(
        row.data.get("priority").and_then(|v| v.as_str()),
        Some("high"),
        "filtered attribute 'priority' must be populated"
    );
}

#[tokio::test]
#[serial]
async fn test_query_filter_no_matches_returns_empty() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "filter-empty-space").await;

    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=nonexistent".to_string()],
        true,
    )
    .await
    .expect("query with no matches should still succeed");

    assert!(
        rows.is_empty(),
        "query with nonexistent filter value should return 0 rows"
    );
}

#[tokio::test]
#[serial]
async fn test_query_no_filter_returns_all() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let ctx = setup_task_entities(&env, "no-filter-space").await;

    let rows = tonk_cli::entity::query(&ctx, "Task".to_string(), vec![], true)
        .await
        .expect("unfiltered query should succeed");

    assert_eq!(
        rows.len(),
        3,
        "unfiltered query should return all 3 entities"
    );

    // All attributes populated for every row
    for row in &rows {
        for attr in &["title", "status", "priority"] {
            let val = row
                .data
                .get(*attr)
                .unwrap_or_else(|| panic!("row {} missing attribute '{}'", row.id, attr));
            assert!(
                !val.is_null(),
                "attribute '{}' is null for entity {}",
                attr,
                row.id
            );
        }
    }

    // Verify all expected titles are present
    let mut titles: Vec<&str> = rows
        .iter()
        .filter_map(|r| r.data.get("title").and_then(|v| v.as_str()))
        .collect();
    titles.sort();
    assert_eq!(
        titles,
        vec!["Deploy to staging", "Fix login bug", "Write tests"]
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Batch Operations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_batch_create() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("batch-create-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create batch JSON file
    let json_path = env.home_path.join("batch.json");
    std::fs::write(
        &json_path,
        r#"[
        {"title": "Task 1", "status": "todo"},
        {"title": "Task 2", "status": "in-progress"},
        {"title": "Task 3", "status": "done"}
    ]"#,
    )
    .expect("Failed to write batch JSON");

    let result = tonk_cli::batch::batch_create(
        &ctx,
        "Task".to_string(),
        Some(json_path.to_string_lossy().into_owned()),
        false,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "batch create should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_batch_delete() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("batch-del-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string()],
        "A task to complete".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // Create some entities first
    let json_path = env.home_path.join("batch_create.json");
    std::fs::write(
        &json_path,
        r#"[
        {"title": "A"},
        {"title": "B"}
    ]"#,
    )
    .expect("Failed to write batch JSON");

    tonk_cli::batch::batch_create(
        &ctx,
        "Task".to_string(),
        Some(json_path.to_string_lossy().into_owned()),
        false,
        true,
    )
    .await
    .expect("Failed to batch create");

    // Get entity IDs
    let branch = tonk_cli::schema::open_branch(&ctx)
        .await
        .expect("Failed to open branch");
    let concept_name = tonk_cli::schema::ConceptName::new("Task").unwrap();
    let concept_entity = tonk_cli::schema::lookup_concept_by_name(&branch, &concept_name)
        .await
        .expect("Failed to lookup concept")
        .expect("Concept 'Task' not found");
    let attrs = tonk_cli::schema::fetch_string_values(
        &branch,
        &concept_entity,
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch concept attributes");
    let entities = tonk_cli::schema::find_entities_by_concept(&branch, &attrs)
        .await
        .expect("Failed to find entities");

    let ids: Vec<String> = entities.iter().map(|e| e.to_string()).collect();
    let ids_json = serde_json::to_string(&ids).unwrap();

    let del_path = env.home_path.join("batch_delete.json");
    std::fs::write(&del_path, &ids_json).expect("Failed to write delete JSON");

    let result = tonk_cli::batch::batch_delete(
        &ctx,
        "Task".to_string(),
        Some(del_path.to_string_lossy().into_owned()),
        false,
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "batch delete should succeed: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Rule Management
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_rule_define_and_list() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("rule-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define prerequisite concepts
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec![
            "title".to_string(),
            "status".to_string(),
            "priority".to_string(),
        ],
        "A task with priority".to_string(),
        true,
    )
    .await
    .expect("Failed to define Task");

    tonk_cli::concept::define(
        &ctx,
        "HighPriority".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A high priority task".to_string(),
        true,
    )
    .await
    .expect("Failed to define HighPriority");

    // Create rule definition JSON (EAV-level premises with the/of/is)
    let rule_json = serde_json::json!({
        "conclusion": {
            "concept": "HighPriority",
            "bindings": {
                "title": "?title",
                "status": "?status"
            }
        },
        "when": [
            {"the": "task/title", "of": "?this", "is": "?title"},
            {"the": "task/status", "of": "?this", "is": "?status"},
            {"the": "task/priority", "of": "?this", "is": "high"}
        ]
    });

    let rule_path = env.home_path.join("rule.json");
    std::fs::write(
        &rule_path,
        serde_json::to_string_pretty(&rule_json).unwrap(),
    )
    .expect("Failed to write rule JSON");

    tonk_cli::rule::define(
        &ctx,
        Some("high-priority-tasks".to_string()),
        Some(rule_path.to_string_lossy().into_owned()),
        false,
        "Find high priority tasks".to_string(),
        true,
    )
    .await
    .expect("Failed to define rule");

    // List rules
    let result = tonk_cli::rule::list(&ctx, true).await;
    assert!(
        result.is_ok(),
        "rule list should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_rule_show() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("rule-show-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "A".to_string(),
        vec!["x".to_string()],
        "Concept A".to_string(),
        true,
    )
    .await
    .unwrap();
    // B must have a different attribute set than A to be structurally distinct
    tonk_cli::concept::define(
        &ctx,
        "B".to_string(),
        vec!["y".to_string()],
        "Concept B".to_string(),
        true,
    )
    .await
    .unwrap();

    let rule_json = serde_json::json!({
        "conclusion": { "concept": "B", "bindings": { "y": "?x" } },
        "when": [{ "the": "rule-show-space/x", "of": "?this", "is": "?x" }]
    });

    let rule_path = env.home_path.join("rule.json");
    std::fs::write(&rule_path, serde_json::to_string(&rule_json).unwrap()).unwrap();

    tonk_cli::rule::define(
        &ctx,
        Some("a-to-b".to_string()),
        Some(rule_path.to_string_lossy().into_owned()),
        false,
        "Maps A to B".to_string(),
        true,
    )
    .await
    .expect("Failed to define rule");

    let result = tonk_cli::rule::show(&ctx, "a-to-b".to_string(), true).await;
    assert!(
        result.is_ok(),
        "rule show should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_rule_delete() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("rule-del-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::concept::define(
        &ctx,
        "X".to_string(),
        vec!["v".to_string()],
        "Concept X".to_string(),
        true,
    )
    .await
    .unwrap();
    // Y must have a different attribute set than X to be structurally distinct
    tonk_cli::concept::define(
        &ctx,
        "Y".to_string(),
        vec!["w".to_string()],
        "Concept Y".to_string(),
        true,
    )
    .await
    .unwrap();

    let rule_json = serde_json::json!({
        "conclusion": { "concept": "Y", "bindings": { "w": "?v" } },
        "when": [{ "the": "rule-del-space/v", "of": "?this", "is": "?v" }]
    });

    let rule_path = env.home_path.join("rule.json");
    std::fs::write(&rule_path, serde_json::to_string(&rule_json).unwrap()).unwrap();

    tonk_cli::rule::define(
        &ctx,
        Some("temp-rule".to_string()),
        Some(rule_path.to_string_lossy().into_owned()),
        false,
        "Maps X to Y".to_string(),
        true,
    )
    .await
    .unwrap();

    // Delete the rule
    tonk_cli::rule::delete(&ctx, "temp-rule".to_string(), true)
        .await
        .expect("Failed to delete rule");

    // Show should fail
    let result = tonk_cli::rule::show(&ctx, "temp-rule".to_string(), true).await;
    assert!(result.is_err(), "rule show after delete should fail");
}

#[tokio::test]
#[serial]
async fn test_rule_validates_concept_exists() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("rule-validation-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define only one concept, not both
    tonk_cli::concept::define(
        &ctx,
        "Exists".to_string(),
        vec!["v".to_string()],
        "A concept that exists".to_string(),
        true,
    )
    .await
    .unwrap();

    let rule_json = serde_json::json!({
        "conclusion": { "concept": "DoesNotExist", "bindings": { "v": "?v" } },
        "when": [{ "the": "exists/v", "of": "?this", "is": "?v" }]
    });

    let rule_path = env.home_path.join("rule.json");
    std::fs::write(&rule_path, serde_json::to_string(&rule_json).unwrap()).unwrap();

    let result = tonk_cli::rule::define(
        &ctx,
        Some("bad-rule".to_string()),
        Some(rule_path.to_string_lossy().into_owned()),
        false,
        "Bad rule test".to_string(),
        true,
    )
    .await;
    assert!(
        result.is_err(),
        "rule referencing non-existent concept should fail"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Dev Fact Operations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_fact_assert_and_find() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Assert a fact
    tonk_cli::fact::assert(
        &ctx,
        "user/name".to_string(),
        "alice".to_string(),
        "Alice Smith".to_string(),
        true,
    )
    .await
    .expect("Failed to assert fact");

    // Find it
    let result =
        tonk_cli::fact::find(&ctx, Some("user/name".to_string()), None, None, None, true).await;
    assert!(
        result.is_ok(),
        "fact find should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_fact_retract() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("retract-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    tonk_cli::fact::assert(
        &ctx,
        "user/name".to_string(),
        "bob".to_string(),
        "Bob Jones".to_string(),
        true,
    )
    .await
    .expect("Failed to assert fact");

    // Retract it
    let result = tonk_cli::fact::retract(
        &ctx,
        "user/name".to_string(),
        "bob".to_string(),
        "Bob Jones".to_string(),
        true,
    )
    .await;
    assert!(
        result.is_ok(),
        "fact retract should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_fact_find_with_entity_filter() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-filter-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Assert multiple facts
    tonk_cli::fact::assert(
        &ctx,
        "user/name".to_string(),
        "alice".to_string(),
        "Alice".to_string(),
        true,
    )
    .await
    .expect("Failed to assert fact 1");

    tonk_cli::fact::assert(
        &ctx,
        "user/email".to_string(),
        "alice".to_string(),
        "alice@example.com".to_string(),
        true,
    )
    .await
    .expect("Failed to assert fact 2");

    tonk_cli::fact::assert(
        &ctx,
        "user/name".to_string(),
        "bob".to_string(),
        "Bob".to_string(),
        true,
    )
    .await
    .expect("Failed to assert fact 3");

    // Find by entity
    let result =
        tonk_cli::fact::find(&ctx, None, Some("alice".to_string()), None, None, true).await;
    assert!(result.is_ok(), "fact find by entity should succeed");

    // Find by attribute
    let result =
        tonk_cli::fact::find(&ctx, Some("user/name".to_string()), None, None, None, true).await;
    assert!(result.is_ok(), "fact find by attribute should succeed");
}

// ═══════════════════════════════════════════════════════════════════════════
// Dev Fact Batch Operations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_fact_batch_from_yaml_file() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-batch-yaml-space")
        .await
        .expect("Failed to create space");

    // Use the example YAML file (user-profile-data.yaml)
    let yaml_path = TestEnv::example_file("user-profile-data.yaml");

    let ctx = tonk_cli::schema::get_space_context().unwrap();
    let result = tonk_cli::fact::batch(&ctx, Some(yaml_path), true).await;
    assert!(
        result.is_ok(),
        "fact batch from YAML file should succeed: {:?}",
        result.err()
    );

    // Verify a fact was asserted by querying for it
    let find_result = tonk_cli::fact::find(
        &ctx,
        Some("carry.profile/name".to_string()),
        Some("keri-vasquez".to_string()),
        None,
        None,
        true,
    )
    .await;
    assert!(
        find_result.is_ok(),
        "should find facts asserted from YAML: {:?}",
        find_result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_fact_batch_yaml_with_explicit_op() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-batch-op-space")
        .await
        .expect("Failed to create space");

    // Write a small YAML file with explicit assert and retract ops
    let yaml_path = env.home_path.join("test-ops.yaml");
    std::fs::write(
        &yaml_path,
        r#"- the: test/color
  of: item-1
  is: "blue"
  op: assert
- the: test/size
  of: item-1
  is: "large"
"#,
    )
    .expect("Failed to write test YAML");

    let ctx = tonk_cli::schema::get_space_context().unwrap();
    let result =
        tonk_cli::fact::batch(&ctx, Some(yaml_path.to_string_lossy().into_owned()), true).await;
    assert!(
        result.is_ok(),
        "fact batch with explicit ops should succeed: {:?}",
        result.err()
    );

    // Verify both facts were asserted
    let find_result =
        tonk_cli::fact::find(&ctx, Some("test/color".to_string()), None, None, None, true).await;
    assert!(
        find_result.is_ok(),
        "should find color fact: {:?}",
        find_result.err()
    );

    let find_result =
        tonk_cli::fact::find(&ctx, Some("test/size".to_string()), None, None, None, true).await;
    assert!(
        find_result.is_ok(),
        "should find size fact (op defaulted to assert): {:?}",
        find_result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_fact_batch_yaml_file_not_found() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-batch-notfound-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().unwrap();
    let result =
        tonk_cli::fact::batch(&ctx, Some("/nonexistent/path.yaml".to_string()), true).await;
    assert!(
        result.is_err(),
        "fact batch with nonexistent file should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_fact_batch_yaml_invalid_content() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("fact-batch-invalid-space")
        .await
        .expect("Failed to create space");

    // Write invalid YAML (not an array of {the, of, is})
    let yaml_path = env.home_path.join("invalid.yaml");
    std::fs::write(&yaml_path, "this is not valid yaml: [[[")
        .expect("Failed to write invalid YAML");

    let ctx = tonk_cli::schema::get_space_context().unwrap();
    let result =
        tonk_cli::fact::batch(&ctx, Some(yaml_path.to_string_lossy().into_owned()), true).await;
    assert!(result.is_err(), "fact batch with invalid YAML should fail");
}

#[tokio::test]
#[serial]
#[allow(clippy::type_complexity)]
async fn test_fact_batch_function_signature() {
    // Compile-time check that `fact::batch` has the expected signature:
    //   (&SpaceContext, Option<String>, bool) -> impl Future<Output = Result<()>>
    //
    // This does not exercise runtime behavior. The actual stdin path
    // is verified by manual testing documented in the requirements.
    let _: fn(
        &tonk_cli::schema::SpaceContext,
        Option<String>,
        bool,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + '_>> =
        |ctx, file, json| Box::pin(tonk_cli::fact::batch(ctx, file, json));
}

// ═══════════════════════════════════════════════════════════════════════════
// Session Management
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_session_list() {
    let _env = TestEnv::new().await.expect("Failed to create test env");

    let result = tonk_cli::session::list(false, true).await;
    assert!(
        result.is_ok(),
        "session list should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_session_current() {
    let _env = TestEnv::new().await.expect("Failed to create test env");

    let result = tonk_cli::session::show_current(true).await;
    assert!(
        result.is_ok(),
        "session current should succeed: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Edge Cases & Error Handling
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_no_space_errors_gracefully() {
    let _env = TestEnv::new().await.expect("Failed to create test env");
    // No space created — get_space_context should fail gracefully

    let result = tonk_cli::schema::get_space_context();
    assert!(
        result.is_err(),
        "get_space_context without space should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_concept_operations_require_space() {
    let _env = TestEnv::new().await.expect("Failed to create test env");

    let result = tonk_cli::schema::get_space_context();
    assert!(result.is_err(), "concept define without space should fail");
}

#[tokio::test]
#[serial]
async fn test_entity_create_requires_concept() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("no-concept-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Try to create an entity of a concept that doesn't exist
    let result = tonk_cli::entity::create(
        &ctx,
        "NonExistent".to_string(),
        vec!["title=Foo".to_string()],
        None,
        false,
        true,
    )
    .await;
    assert!(
        result.is_err(),
        "creating entity of non-existent concept should fail"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// End-to-End Workflows
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_full_crud_workflow() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("workflow-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get context");

    // 1. Import concepts from YAML
    let file = TestEnv::example_file("minimal.yaml");
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import concepts");

    // 2. Create entities
    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec![
            "title=Write tests".to_string(),
            "status=todo".to_string(),
            "priority=high".to_string(),
        ],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task 1");

    tonk_cli::entity::create(
        &ctx,
        "Task".to_string(),
        vec![
            "title=Review PR".to_string(),
            "status=in-progress".to_string(),
            "priority=medium".to_string(),
        ],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create task 2");

    // 3. Query all tasks
    let rows = tonk_cli::entity::query(&ctx, "Task".to_string(), vec![], true)
        .await
        .expect("query all tasks should succeed");
    assert_eq!(rows.len(), 2, "should have 2 tasks");

    // 4. Query with filter
    let rows = tonk_cli::entity::query(
        &ctx,
        "Task".to_string(),
        vec!["status=todo".to_string()],
        true,
    )
    .await
    .expect("filtered query should succeed");
    assert_eq!(rows.len(), 1, "filter should match 1 task");

    // 5. Find an entity and update it
    let branch = tonk_cli::schema::open_branch(&ctx)
        .await
        .expect("Failed to open branch");
    let concept_name = tonk_cli::schema::ConceptName::new("Task").unwrap();
    let concept_entity = tonk_cli::schema::lookup_concept_by_name(&branch, &concept_name)
        .await
        .expect("Failed to lookup concept")
        .expect("Concept 'Task' not found");
    let attrs = tonk_cli::schema::fetch_string_values(
        &branch,
        &concept_entity,
        tonk_cli::schema::concept_attribute_selector(),
    )
    .await
    .expect("Failed to fetch concept attributes");
    let entities = tonk_cli::schema::find_entities_by_concept(&branch, &attrs)
        .await
        .expect("Failed to find entities");
    assert_eq!(entities.len(), 2, "Should have 2 entities");

    let entity_id = entities[0].to_string();
    tonk_cli::entity::assert(
        &ctx,
        entity_id.clone(),
        vec!["status=done".to_string()],
        true,
    )
    .await
    .expect("Failed to update entity");

    // 6. Show the updated entity
    tonk_cli::entity::show(&ctx, entity_id.clone(), true)
        .await
        .expect("Failed to show entity");

    // 7. Delete the entity
    tonk_cli::entity::retract(&ctx, entity_id, true)
        .await
        .expect("Failed to delete entity");

    // 8. Verify only 1 entity remains
    let branch2 = tonk_cli::schema::open_branch(&ctx)
        .await
        .expect("Failed to re-open branch");
    let remaining = tonk_cli::schema::find_entities_by_concept(&branch2, &attrs)
        .await
        .expect("Failed to find remaining entities");
    assert_eq!(remaining.len(), 1, "Should have 1 entity after deletion");
}

#[tokio::test]
#[serial]
async fn test_import_concepts_and_rules_full_workflow() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("full-workflow")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // 1. Import cook concepts first (planner rules reference cook concepts)
    let cook = TestEnv::example_file("cook.yaml");
    tonk_cli::import::import(&ctx, cook, false, true)
        .await
        .expect("Failed to import cook concepts");

    // 2. Import planner (mixed: 4 concepts + 2 rules across 2 namespaces)
    let planner = TestEnv::example_file("planner.yaml");
    tonk_cli::import::import(&ctx, planner, false, true)
        .await
        .expect("Failed to import planner");

    // 3. Verify all 7 concepts exist (4 planner + 3 cook)
    for name in &[
        "Allergy",
        "Event",
        "Meal",
        "AllergyConflict",
        "Recipe",
        "Ingredient",
        "RecipeStep",
    ] {
        let result = tonk_cli::concept::show(&ctx, name.to_string(), true).await;
        assert!(result.is_ok(), "{} should exist after import", name);
    }

    // 4. Import standalone rules file (rules referencing existing concepts)
    let rules = TestEnv::example_file("rules.yaml");
    tonk_cli::import::import(&ctx, rules, false, true)
        .await
        .expect("Failed to import rules");

    // 5. Verify rules exist
    let result = tonk_cli::rule::list(&ctx, true).await;
    assert!(result.is_ok(), "rule list should succeed after import");

    // 6. Create some data
    tonk_cli::entity::create(
        &ctx,
        "Recipe".to_string(),
        vec!["title=Pasta".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create recipe");

    tonk_cli::entity::create(
        &ctx,
        "Ingredient".to_string(),
        vec!["name=Peanuts".to_string(), "quantity=100".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create ingredient");

    // 7. Query entities — the queries should not error.
    // Note: Recipe has 3 required attributes (title, steps, ingredient) but
    // only title was provided, so the structural join returns 0 rows. Same
    // for Ingredient (needs name, quantity, unit but only name + quantity
    // were given). This is expected: dialog-db requires all attributes to
    // match. We verify the query executes without error, not that rows are
    // returned.
    let _rows = tonk_cli::entity::query(&ctx, "Recipe".to_string(), vec![], true)
        .await
        .expect("recipe query should succeed");

    let _rows = tonk_cli::entity::query(&ctx, "Ingredient".to_string(), vec![], true)
        .await
        .expect("ingredient query should succeed");
}

#[tokio::test]
#[serial]
async fn test_concept_define_with_many_attributes() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("many-attrs-space")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define a concept with many attributes
    tonk_cli::concept::define(
        &ctx,
        "Person".to_string(),
        vec![
            "first-name".to_string(),
            "last-name".to_string(),
            "email".to_string(),
            "phone".to_string(),
            "address".to_string(),
            "city".to_string(),
            "country".to_string(),
            "age".to_string(),
        ],
        "A person record".to_string(),
        true,
    )
    .await
    .expect("Failed to define Person concept");

    // Show should display all attributes
    let result = tonk_cli::concept::show(&ctx, "Person".to_string(), true).await;
    assert!(result.is_ok(), "show Person should succeed");
}

#[tokio::test]
#[serial]
async fn test_multiple_concepts_same_space() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("multi-concept")
        .await
        .expect("Failed to create space");

    let ctx = tonk_cli::schema::get_space_context().expect("Failed to get space context");

    // Define multiple concepts
    tonk_cli::concept::define(
        &ctx,
        "User".to_string(),
        vec!["name".to_string(), "email".to_string()],
        "A user account".to_string(),
        true,
    )
    .await
    .expect("Failed to define User");
    tonk_cli::concept::define(
        &ctx,
        "Post".to_string(),
        vec!["title".to_string(), "body".to_string()],
        "A blog post".to_string(),
        true,
    )
    .await
    .expect("Failed to define Post");
    tonk_cli::concept::define(
        &ctx,
        "Comment".to_string(),
        vec!["text".to_string()],
        "A comment on a post".to_string(),
        true,
    )
    .await
    .expect("Failed to define Comment");

    // List should show all 3
    let result = tonk_cli::concept::list(&ctx, true).await;
    assert!(result.is_ok(), "concept list should succeed");

    // Create entities of each
    tonk_cli::entity::create(
        &ctx,
        "User".to_string(),
        vec!["name=Alice".to_string(), "email=a@b.com".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create user");
    tonk_cli::entity::create(
        &ctx,
        "Post".to_string(),
        vec!["title=Hello".to_string(), "body=World".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create post");
    tonk_cli::entity::create(
        &ctx,
        "Comment".to_string(),
        vec!["text=Nice post".to_string()],
        None,
        false,
        true,
    )
    .await
    .expect("Failed to create comment");

    // Query each concept independently
    for concept in &["User", "Post", "Comment"] {
        let rows = tonk_cli::entity::query(&ctx, concept.to_string(), vec![], true)
            .await
            .unwrap_or_else(|e| panic!("{} query should succeed: {}", concept, e));
        assert!(
            !rows.is_empty(),
            "{} should have at least 1 entity",
            concept
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Attribute Introspection
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn test_attribute_list_empty_space() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-empty")
        .await
        .expect("Failed to create space");

    // List attributes in an empty space — should succeed (prints empty message)
    let result = tonk_cli::attribute::list(false).await;
    assert!(
        result.is_ok(),
        "attribute list on empty space should succeed: {:?}",
        result.err()
    );

    // JSON mode should also succeed
    let result = tonk_cli::attribute::list(true).await;
    assert!(
        result.is_ok(),
        "attribute list JSON on empty space should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_list_after_concept_define() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-define")
        .await
        .expect("Failed to create space");

    // Define a concept (no metadata — just attribute names)
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task with title and status".to_string(),
        true,
    )
    .await
    .expect("Failed to define concept");

    // List attributes — should show Task with title and status (no metadata)
    let result = tonk_cli::attribute::list(false).await;
    assert!(
        result.is_ok(),
        "attribute list should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result = tonk_cli::attribute::list(true).await;
    assert!(
        result.is_ok(),
        "attribute list JSON should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_list_after_import_with_metadata() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-import")
        .await
        .expect("Failed to create space");

    // Import cook.yaml which has full attribute metadata
    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // List attributes — should show Recipe, Ingredient, RecipeStep with metadata
    let result = tonk_cli::attribute::list(false).await;
    assert!(
        result.is_ok(),
        "attribute list after import should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result = tonk_cli::attribute::list(true).await;
    assert!(
        result.is_ok(),
        "attribute list JSON after import should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_qualified_name() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-qual")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Show by qualified name
    let result = tonk_cli::attribute::show("recipe/title".to_string(), None, false).await;
    assert!(
        result.is_ok(),
        "attribute show by qualified name should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result = tonk_cli::attribute::show("recipe/title".to_string(), None, true).await;
    assert!(
        result.is_ok(),
        "attribute show JSON by qualified name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_short_name_with_concept() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-short")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Show by short name with --concept
    let result =
        tonk_cli::attribute::show("title".to_string(), Some("Recipe".to_string()), false).await;
    assert!(
        result.is_ok(),
        "attribute show with --concept should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result =
        tonk_cli::attribute::show("title".to_string(), Some("Recipe".to_string()), true).await;
    assert!(
        result.is_ok(),
        "attribute show JSON with --concept should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_unambiguous_short_name() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-unambig")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "steps" only exists on Recipe, so it should resolve unambiguously
    let result = tonk_cli::attribute::show("steps".to_string(), None, false).await;
    assert!(
        result.is_ok(),
        "attribute show unambiguous short name should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_ambiguous_short_name_errors() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-ambig")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "name" exists on both Ingredient and possibly others — check if it's ambiguous
    // Actually in cook.yaml, "name" only exists on Ingredient.
    // Let's use a concept where we know there's overlap. Define a second concept
    // with a "name" attribute to create ambiguity.
    tonk_cli::concept::define(
        &ctx,
        "Person".to_string(),
        vec!["name".to_string(), "age".to_string()],
        "A person with name and age".to_string(),
        true,
    )
    .await
    .expect("Failed to define Person concept");

    // Now "name" exists on both Ingredient and Person
    let result = tonk_cli::attribute::show("name".to_string(), None, false).await;
    assert!(
        result.is_err(),
        "attribute show with ambiguous name should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_nonexistent_qualified() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-noexist")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Nonexistent qualified name
    let result = tonk_cli::attribute::show("recipe/nonexistent".to_string(), None, false).await;
    assert!(
        result.is_err(),
        "attribute show with nonexistent qualified name should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_nonexistent_short() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-noexist-short")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Nonexistent short name
    let result = tonk_cli::attribute::show("zzz_nonexistent".to_string(), None, false).await;
    assert!(
        result.is_err(),
        "attribute show with nonexistent short name should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_wrong_concept() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-wrongconcept")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "title" belongs to Recipe, not Ingredient
    let result =
        tonk_cli::attribute::show("title".to_string(), Some("Ingredient".to_string()), false).await;
    assert!(
        result.is_err(),
        "attribute show with wrong concept should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_nonexistent_concept() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-badconcept")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Concept doesn't exist
    let result =
        tonk_cli::attribute::show("title".to_string(), Some("Nonexistent".to_string()), false)
            .await;
    assert!(
        result.is_err(),
        "attribute show with nonexistent concept should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_enum_type() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-enum")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "unit" on Ingredient has enum type ["tsp","mls"]
    let result =
        tonk_cli::attribute::show("unit".to_string(), Some("Ingredient".to_string()), false).await;
    assert!(
        result.is_ok(),
        "attribute show for enum type should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result =
        tonk_cli::attribute::show("unit".to_string(), Some("Ingredient".to_string()), true).await;
    assert!(
        result.is_ok(),
        "attribute show JSON for enum type should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_many_cardinality() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-many")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "ingredient" on Recipe has cardinality: many
    let result =
        tonk_cli::attribute::show("ingredient".to_string(), Some("Recipe".to_string()), false)
            .await;
    assert!(
        result.is_ok(),
        "attribute show for many-cardinality should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_optional_attribute() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-optional")
        .await
        .expect("Failed to create space");

    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // "after" on RecipeStep has optional: true
    let result =
        tonk_cli::attribute::show("after".to_string(), Some("RecipeStep".to_string()), false).await;
    assert!(
        result.is_ok(),
        "attribute show for optional attribute should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_list_no_space_errors() {
    let _env = TestEnv::new().await.expect("Failed to create test env");
    // No space created — should fail gracefully

    let result = tonk_cli::attribute::list(false).await;
    assert!(
        result.is_err(),
        "attribute list without a space should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_no_space_errors() {
    let _env = TestEnv::new().await.expect("Failed to create test env");
    // No space created — should fail gracefully

    let result = tonk_cli::attribute::show("recipe/title".to_string(), None, false).await;
    assert!(
        result.is_err(),
        "attribute show without a space should fail"
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_list_after_import_and_define_mixed() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-mixed")
        .await
        .expect("Failed to create space");

    // Import cook.yaml (has metadata)
    let file = TestEnv::example_file("cook.yaml");
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::import::import(&ctx, file, false, true)
        .await
        .expect("Failed to import cook.yaml");

    // Also define a concept manually (no metadata)
    tonk_cli::concept::define(
        &ctx,
        "Task".to_string(),
        vec!["title".to_string(), "status".to_string()],
        "A task".to_string(),
        true,
    )
    .await
    .expect("Failed to define Task");

    // List should show both imported (with metadata) and defined (without)
    let result = tonk_cli::attribute::list(false).await;
    assert!(
        result.is_ok(),
        "attribute list with mixed concepts should succeed: {:?}",
        result.err()
    );

    // JSON mode
    let result = tonk_cli::attribute::list(true).await;
    assert!(
        result.is_ok(),
        "attribute list JSON with mixed concepts should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
#[serial]
async fn test_attribute_show_defined_concept_no_metadata() {
    let env = TestEnv::new().await.expect("Failed to create test env");
    let _space = env
        .create_space("attr-show-nometa")
        .await
        .expect("Failed to create space");

    // Define a concept without importing (no metadata)
    let ctx = tonk_cli::schema::get_space_context().unwrap();
    tonk_cli::concept::define(
        &ctx,
        "Note".to_string(),
        vec!["body".to_string(), "tags".to_string()],
        "A note with body and tags".to_string(),
        true,
    )
    .await
    .expect("Failed to define Note");

    // Show should work, just without metadata
    let result =
        tonk_cli::attribute::show("body".to_string(), Some("Note".to_string()), false).await;
    assert!(
        result.is_ok(),
        "attribute show without metadata should succeed: {:?}",
        result.err()
    );

    // Qualified form too
    let result = tonk_cli::attribute::show("note/body".to_string(), None, false).await;
    assert!(
        result.is_ok(),
        "attribute show by qualified name without metadata should succeed: {:?}",
        result.err()
    );
}
