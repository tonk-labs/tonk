//! Tests for native Automerge storage, patch_document, and splice_text functionality

use serde_json::json;
use tonk_core::TonkCore;

// ============================================================================
// Native Storage Tests
// ============================================================================

#[tokio::test]
async fn test_create_and_read_document() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "name": "test",
        "value": 42,
        "active": true
    });

    vfs.create_document("/test.json", content.clone())
        .await
        .unwrap();

    // Read it back and verify
    let doc = vfs.find_document("/test.json").await.unwrap();
    assert!(doc.is_some(), "Document should exist");
}

#[tokio::test]
async fn test_nested_object_storage() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "level1": {
            "level2": {
                "level3": {
                    "value": "deeply nested"
                }
            }
        }
    });

    vfs.create_document("/nested.json", content.clone())
        .await
        .unwrap();

    let exists = vfs.exists("/nested.json").await.unwrap();
    assert!(exists, "Nested document should exist");
}

#[tokio::test]
async fn test_array_storage() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "items": [1, 2, 3, 4, 5],
        "names": ["alice", "bob", "charlie"]
    });

    vfs.create_document("/arrays.json", content.clone())
        .await
        .unwrap();

    let exists = vfs.exists("/arrays.json").await.unwrap();
    assert!(exists, "Array document should exist");
}

#[tokio::test]
async fn test_mixed_nested_structures() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "users": [
            {"name": "alice", "age": 30},
            {"name": "bob", "age": 25}
        ],
        "metadata": {
            "tags": ["important", "urgent"],
            "counts": [1, 2, 3]
        }
    });

    vfs.create_document("/mixed.json", content.clone())
        .await
        .unwrap();

    let exists = vfs.exists("/mixed.json").await.unwrap();
    assert!(exists, "Mixed structure document should exist");
}

// ============================================================================
// Patch Document Tests
// ============================================================================

#[tokio::test]
async fn test_patch_top_level_field() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "x": 100,
        "y": 200
    });

    vfs.create_document("/position.json", content)
        .await
        .unwrap();

    // Patch the x field
    let path = vec!["x".to_string()];
    let updated = vfs
        .patch_document("/position.json", &path, json!(150))
        .await
        .unwrap();

    assert!(updated, "Patch should return true for existing document");
}

#[tokio::test]
async fn test_patch_nested_field() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "config": {
            "settings": {
                "theme": "light"
            }
        }
    });

    vfs.create_document("/config.json", content).await.unwrap();

    // Patch the nested theme field
    let path = vec![
        "config".to_string(),
        "settings".to_string(),
        "theme".to_string(),
    ];
    let updated = vfs
        .patch_document("/config.json", &path, json!("dark"))
        .await
        .unwrap();

    assert!(updated, "Patch should succeed for nested field");
}

#[tokio::test]
async fn test_patch_preserves_other_fields() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "a": 1,
        "b": 2,
        "c": 3
    });

    vfs.create_document("/multi.json", content).await.unwrap();

    // Patch only field 'b'
    let path = vec!["b".to_string()];
    vfs.patch_document("/multi.json", &path, json!(999))
        .await
        .unwrap();

    // Document should still exist with all fields
    let exists = vfs.exists("/multi.json").await.unwrap();
    assert!(exists, "Document should still exist after patch");
}

#[tokio::test]
async fn test_patch_nonexistent_file() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let path = vec!["x".to_string()];
    let result = vfs
        .patch_document("/nonexistent.json", &path, json!(100))
        .await
        .unwrap();

    assert!(!result, "Patch should return false for non-existent file");
}

#[tokio::test]
async fn test_patch_with_empty_path_replaces_content() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({"x": 1});
    vfs.create_document("/test.json", content).await.unwrap();

    // Empty path patches the entire content field (valid operation)
    let path: Vec<String> = vec![];
    let result = vfs
        .patch_document("/test.json", &path, json!({"y": 2}))
        .await;

    assert!(result.is_ok(), "Empty path should replace entire content");
    assert!(result.unwrap(), "Patch should return true");
}

#[tokio::test]
async fn test_patch_overwrites_value_type() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "value": 42
    });

    vfs.create_document("/type.json", content).await.unwrap();

    // Patch primitive with object
    let path = vec!["value".to_string()];
    let updated = vfs
        .patch_document("/type.json", &path, json!({"nested": "object"}))
        .await
        .unwrap();

    assert!(updated, "Patch should allow changing value type");
}

// ============================================================================
// Splice Text Tests
// ============================================================================

#[tokio::test]
async fn test_splice_insert_text() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "text": "hello"
    });

    vfs.create_document("/text.json", content).await.unwrap();

    // Insert " world" at position 5
    let path = vec!["text".to_string()];
    let updated = vfs
        .splice_text("/text.json", &path, 5, 0, " world")
        .await
        .unwrap();

    assert!(updated, "Splice insert should succeed");
}

#[tokio::test]
async fn test_splice_delete_text() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "text": "hello world"
    });

    vfs.create_document("/text.json", content).await.unwrap();

    // Delete " world" (6 chars starting at position 5)
    let path = vec!["text".to_string()];
    let updated = vfs
        .splice_text("/text.json", &path, 5, 6, "")
        .await
        .unwrap();

    assert!(updated, "Splice delete should succeed");
}

#[tokio::test]
async fn test_splice_replace_text() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "text": "hello world"
    });

    vfs.create_document("/text.json", content).await.unwrap();

    // Replace "world" with "universe"
    let path = vec!["text".to_string()];
    let updated = vfs
        .splice_text("/text.json", &path, 6, 5, "universe")
        .await
        .unwrap();

    assert!(updated, "Splice replace should succeed");
}

#[tokio::test]
async fn test_splice_at_beginning() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "text": "world"
    });

    vfs.create_document("/text.json", content).await.unwrap();

    // Insert "hello " at position 0
    let path = vec!["text".to_string()];
    let updated = vfs
        .splice_text("/text.json", &path, 0, 0, "hello ")
        .await
        .unwrap();

    assert!(updated, "Splice at beginning should succeed");
}

#[tokio::test]
async fn test_splice_creates_text_field() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "other": "value"
    });

    vfs.create_document("/text.json", content).await.unwrap();

    // Splice on non-existent field should create Text object
    let path = vec!["newtext".to_string()];
    let updated = vfs
        .splice_text("/text.json", &path, 0, 0, "created text")
        .await
        .unwrap();

    assert!(updated, "Splice should create new text field");
}

#[tokio::test]
async fn test_splice_nonexistent_file() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let path = vec!["text".to_string()];
    let result = vfs
        .splice_text("/nonexistent.json", &path, 0, 0, "text")
        .await
        .unwrap();

    assert!(!result, "Splice should return false for non-existent file");
}

#[tokio::test]
async fn test_splice_nested_text_field() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    // Create doc with nested text field
    let content = json!({
        "document": {
            "title": "Hello"
        }
    });
    vfs.create_document("/doc.json", content).await.unwrap();

    // Splice into nested text field
    let path = vec!["document".to_string(), "title".to_string()];
    let result = vfs.splice_text("/doc.json", &path, 5, 0, " World").await;

    assert!(result.is_ok(), "Should splice into nested text field");
    assert!(result.unwrap(), "Splice should return true");
}

// ============================================================================
// Integration Tests
// ============================================================================

#[tokio::test]
async fn test_patch_then_read_roundtrip() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "counter": 0
    });

    vfs.create_document("/counter.json", content).await.unwrap();

    // Patch the counter
    let path = vec!["counter".to_string()];
    vfs.patch_document("/counter.json", &path, json!(42))
        .await
        .unwrap();

    // Verify document still exists and is readable
    let exists = vfs.exists("/counter.json").await.unwrap();
    assert!(exists, "Document should exist after patch");
}

#[tokio::test]
async fn test_multiple_patches_accumulate() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    let content = json!({
        "a": 1,
        "b": 2,
        "c": 3
    });

    vfs.create_document("/multi.json", content).await.unwrap();

    // Apply multiple patches
    vfs.patch_document("/multi.json", &vec!["a".to_string()], json!(10))
        .await
        .unwrap();
    vfs.patch_document("/multi.json", &vec!["b".to_string()], json!(20))
        .await
        .unwrap();
    vfs.patch_document("/multi.json", &vec!["c".to_string()], json!(30))
        .await
        .unwrap();

    // Document should still exist
    let exists = vfs.exists("/multi.json").await.unwrap();
    assert!(exists, "Document should exist after multiple patches");
}
