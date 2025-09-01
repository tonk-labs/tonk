use tempfile::NamedTempFile;
use tonk_core::{error::VfsError, vfs::NodeType, TonkCore};

#[tokio::test]
async fn test_complete_lifecycle() {
    // Create, populate, save, load, verify
    let mut tonk = TonkCore::new().await.unwrap();

    // Create complex structure
    tonk.vfs()
        .create_document("/README.md", "# My App".to_string())
        .await
        .unwrap();
    tonk.vfs().create_directory("/src").await.unwrap();
    tonk.vfs()
        .create_document("/src/index.js", "console.log('hello')".to_string())
        .await
        .unwrap();

    // Save to file
    let temp = NamedTempFile::new().unwrap();
    tonk.to_file(temp.path()).await.unwrap();

    // Load from file
    let tonk2 = TonkCore::from_file(temp.path()).await.unwrap();

    // Verify structure preserved
    let files = tonk2.vfs().list_directory("/src").await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "index.js");

    // Verify root directory contents
    let root_files = tonk2.vfs().list_directory("/").await.unwrap();
    assert_eq!(root_files.len(), 2);
    let names: Vec<String> = root_files.iter().map(|f| f.name.clone()).collect();
    assert!(names.contains(&"README.md".to_string()));
    assert!(names.contains(&"src".to_string()));
}

#[tokio::test]
async fn test_bundle_metadata_persistence() {
    let mut tonk = TonkCore::new().await.unwrap();

    // Create content
    tonk.vfs()
        .create_document("/app.js", "// app code".to_string())
        .await
        .unwrap();

    // Export and reimport
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Verify manifest preserved
    assert_eq!(tonk2.manifest().root, "root");
    assert_eq!(tonk2.manifest().network_uris.len(), 0);
    assert_eq!(tonk2.manifest().manifest_version, 1);
    assert_eq!(tonk2.manifest().version.major, 1);
    assert_eq!(tonk2.manifest().version.minor, 0);
}

#[tokio::test]
async fn test_error_handling() {
    let tonk = TonkCore::new().await.unwrap();

    // Test duplicate file creation
    tonk.vfs()
        .create_document("/file.txt", "content".to_string())
        .await
        .unwrap();
    let result = tonk
        .vfs()
        .create_document("/file.txt", "new".to_string())
        .await;
    assert!(matches!(result, Err(VfsError::DocumentExists(_))));

    // Test invalid paths
    let result = tonk.vfs().create_document("/", "root".to_string()).await;
    assert!(matches!(result, Err(VfsError::RootPathError)));

    // Test creating directory at root
    let result = tonk.vfs().create_directory("/").await;
    assert!(matches!(result, Err(VfsError::RootPathError)));
}

#[tokio::test]
async fn test_concurrent_operations() {
    use futures::future::join_all;

    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    // Create multiple files concurrently
    let futures = vec![
        vfs.create_document("/file1.txt", "1".to_string()),
        vfs.create_document("/file2.txt", "2".to_string()),
        vfs.create_document("/file3.txt", "3".to_string()),
    ];

    let results = join_all(futures).await;
    assert!(results.iter().all(|r| r.is_ok()));

    // Verify all files exist
    let files = vfs.list_directory("/").await.unwrap();
    assert_eq!(files.len(), 3);
}

#[tokio::test]
async fn test_complex_directory_structure() {
    let mut tonk = TonkCore::new().await.unwrap();

    // Create complex structure
    let paths = vec![
        ("/README.md", "# My Project"),
        ("/package.json", r#"{"name": "my-app"}"#),
        ("/src/index.js", "import './components';"),
        ("/src/components/Button.js", "export default Button;"),
        ("/src/components/Form.js", "export default Form;"),
        ("/src/utils/helpers.js", "export const help = () => {};"),
        ("/tests/unit/button.test.js", "test('button', () => {});"),
        ("/tests/integration/app.test.js", "test('app', () => {});"),
    ];

    for (path, content) in paths {
        tonk.vfs()
            .create_document(path, content.to_string())
            .await
            .unwrap();
    }

    // Save and reload
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Verify structure
    assert!(tonk2.vfs().exists("/README.md").await.unwrap());
    assert!(tonk2
        .vfs()
        .exists("/src/components/Button.js")
        .await
        .unwrap());
    assert!(tonk2
        .vfs()
        .exists("/tests/integration/app.test.js")
        .await
        .unwrap());

    // Check directory listings
    let src_components = tonk2.vfs().list_directory("/src/components").await.unwrap();
    assert_eq!(src_components.len(), 2);

    let tests = tonk2.vfs().list_directory("/tests").await.unwrap();
    assert_eq!(tests.len(), 2); // unit and integration dirs
}

#[tokio::test]
async fn test_empty_bundle_roundtrip() {
    // Create empty, save, and reload
    let mut tonk = TonkCore::new().await.unwrap();
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Should still be functional
    let root_contents = tonk2.vfs().list_directory("/").await.unwrap();
    assert!(root_contents.is_empty());

    // Can add content after reload
    tonk2
        .vfs()
        .create_document("/after-reload.txt", "Still works!".to_string())
        .await
        .unwrap();

    let updated_contents = tonk2.vfs().list_directory("/").await.unwrap();
    assert_eq!(updated_contents.len(), 1);
}

#[tokio::test]
async fn test_file_overwrite_prevention() {
    let tonk = TonkCore::new().await.unwrap();

    // Create a file
    tonk.vfs()
        .create_document("/data.txt", "original content".to_string())
        .await
        .unwrap();

    // Try to create with same name (should fail)
    let result = tonk
        .vfs()
        .create_document("/data.txt", "new content".to_string())
        .await;

    assert!(matches!(result, Err(VfsError::DocumentExists(_))));
}

#[tokio::test]
async fn test_nonexistent_file_access() {
    let tonk = TonkCore::new().await.unwrap();

    // Check non-existent file
    let exists = tonk.vfs().exists("/doesnt-exist.txt").await.unwrap();
    assert!(!exists);

    // Try to find non-existent document
    let result = tonk.vfs().find_document("/no-such-file.txt").await.unwrap();
    assert!(result.is_none());

    // List non-existent directory
    let result = tonk.vfs().list_directory("/no-such-dir").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_deeply_nested_paths() {
    let mut tonk = TonkCore::new().await.unwrap();

    // Create deeply nested file (should auto-create parent dirs)
    tonk.vfs()
        .create_document(
            "/very/deeply/nested/folder/structure/file.txt",
            "Deep content".to_string(),
        )
        .await
        .unwrap();

    // Verify all parent directories were created
    assert!(tonk.vfs().exists("/very").await.unwrap());
    assert!(tonk.vfs().exists("/very/deeply").await.unwrap());
    assert!(tonk.vfs().exists("/very/deeply/nested").await.unwrap());
    assert!(tonk
        .vfs()
        .exists("/very/deeply/nested/folder")
        .await
        .unwrap());
    assert!(tonk
        .vfs()
        .exists("/very/deeply/nested/folder/structure")
        .await
        .unwrap());
    assert!(tonk
        .vfs()
        .exists("/very/deeply/nested/folder/structure/file.txt")
        .await
        .unwrap());

    // Save and reload to ensure persistence
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Verify structure is preserved
    assert!(tonk2
        .vfs()
        .exists("/very/deeply/nested/folder/structure/file.txt")
        .await
        .unwrap());
}

#[tokio::test]
async fn test_sync_engine_access() {
    let tonk = TonkCore::new().await.unwrap();

    // Verify we can access the sync engine
    let engine = tonk.engine();
    let peer_id = engine.peer_id();
    assert!(!peer_id.to_string().is_empty());

    // Verify we can access samod
    let samod = engine.samod();
    let samod_peer_id = samod.peer_id();
    assert_eq!(peer_id, samod_peer_id);
}

#[tokio::test]
async fn test_directory_and_file_types() {
    let tonk = TonkCore::new().await.unwrap();

    // Create mixed content
    tonk.vfs().create_directory("/config").await.unwrap();
    tonk.vfs()
        .create_document("/config/settings.json", "{}".to_string())
        .await
        .unwrap();
    tonk.vfs()
        .create_document("/index.html", "<html>".to_string())
        .await
        .unwrap();

    // List and verify types
    let root_items = tonk.vfs().list_directory("/").await.unwrap();

    for item in root_items {
        match item.name.as_str() {
            "config" => assert_eq!(item.node_type, NodeType::Directory),
            "index.html" => assert_eq!(item.node_type, NodeType::Document),
            _ => panic!("Unexpected item: {}", item.name),
        }
    }
}

#[tokio::test]
async fn test_save_load_preserves_timestamps() {
    let mut tonk = TonkCore::new().await.unwrap();

    // Create a file
    tonk.vfs()
        .create_document("/timestamped.txt", "content".to_string())
        .await
        .unwrap();

    // Get metadata before save
    let metadata_before = tonk.vfs().get_metadata("/timestamped.txt").await.unwrap();
    assert!(metadata_before.is_some());
    let (_, timestamps_before) = metadata_before.unwrap();

    // Save and reload
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Get metadata after load
    let metadata_after = tonk2.vfs().get_metadata("/timestamped.txt").await.unwrap();
    assert!(metadata_after.is_some());
    let (_, timestamps_after) = metadata_after.unwrap();

    // Timestamps should be preserved
    assert_eq!(timestamps_before.created, timestamps_after.created);
}

#[tokio::test]
async fn test_remove_document() {
    let tonk = TonkCore::new().await.unwrap();

    // Create and then remove a document
    tonk.vfs()
        .create_document("/temp.txt", "temporary".to_string())
        .await
        .unwrap();
    assert!(tonk.vfs().exists("/temp.txt").await.unwrap());

    let removed = tonk.vfs().remove_document("/temp.txt").await.unwrap();
    assert!(removed);
    assert!(!tonk.vfs().exists("/temp.txt").await.unwrap());

    // Try to remove again
    let removed_again = tonk.vfs().remove_document("/temp.txt").await.unwrap();
    assert!(!removed_again);
}

#[tokio::test]
async fn test_remove_directory_cascade() {
    let tonk = TonkCore::new().await.unwrap();

    // Create directory with contents
    tonk.vfs().create_directory("/to-remove").await.unwrap();
    tonk.vfs()
        .create_document("/to-remove/file1.txt", "1".to_string())
        .await
        .unwrap();
    tonk.vfs()
        .create_document("/to-remove/file2.txt", "2".to_string())
        .await
        .unwrap();
    tonk.vfs()
        .create_directory("/to-remove/subdir")
        .await
        .unwrap();
    tonk.vfs()
        .create_document("/to-remove/subdir/nested.txt", "nested".to_string())
        .await
        .unwrap();

    // Remove the directory
    let removed = tonk.vfs().remove_document("/to-remove").await.unwrap();
    assert!(removed);

    // Verify everything is gone
    assert!(!tonk.vfs().exists("/to-remove").await.unwrap());
    assert!(!tonk.vfs().exists("/to-remove/file1.txt").await.unwrap());
    assert!(!tonk
        .vfs()
        .exists("/to-remove/subdir/nested.txt")
        .await
        .unwrap());
}

