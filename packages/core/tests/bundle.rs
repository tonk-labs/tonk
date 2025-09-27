mod common;

use std::io::Write;
use std::time::Duration;
use tokio::time::sleep;
use tonk_core::{Bundle, TonkCore};

#[tokio::test]
async fn test_basic_bundle_round_trip() {
    // Create a TonkCore instance with some content
    let tonk1 = TonkCore::new().await.unwrap();
    let vfs1 = tonk1.vfs();

    // Add various types of content
    vfs1.create_document("/readme.txt", "Hello from bundle test".to_string())
        .await
        .unwrap();
    vfs1.create_document("/config.json", r#"{"version": "1.0"}"#.to_string())
        .await
        .unwrap();
    vfs1.create_directory("/docs").await.unwrap();
    vfs1.create_document("/docs/guide.md", "# User Guide\nWelcome!".to_string())
        .await
        .unwrap();

    // Save to bundle
    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();
    assert!(!bundle_bytes.is_empty(), "Bundle should not be empty");

    // Debug: save bundle to file so we can inspect it
    // std::fs::write("/tmp/debug_bundle.zip", &bundle_bytes).unwrap();
    // println!("Saved debug bundle to /tmp/debug_bundle.zip");

    // Load from bundle into a new TonkCore
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let vfs2 = tonk2.vfs();

    // Verify all content is preserved
    assert!(vfs2.exists("/readme.txt").await.unwrap());
    assert!(vfs2.exists("/config.json").await.unwrap());
    assert!(vfs2.exists("/docs").await.unwrap());
    assert!(vfs2.exists("/docs/guide.md").await.unwrap());

    // Verify content matches
    let readme = vfs2
        .find_document("/readme.txt")
        .await
        .unwrap()
        .expect("Should find readme");

    // Use the proper VFS API to read document content
    use tonk_core::vfs::backend::AutomergeHelpers;
    use tonk_core::vfs::types::DocNode;

    let doc_node: DocNode<String> = AutomergeHelpers::read_document(&readme).unwrap();
    assert_eq!(doc_node.content, "Hello from bundle test");
}

#[tokio::test]
async fn test_empty_bundle() {
    // Create empty TonkCore
    let tonk1 = TonkCore::new().await.unwrap();

    // Save empty state to bundle
    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();

    // Load from bundle
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let vfs2 = tonk2.vfs();

    // Verify root exists and is empty
    let root_id = vfs2.root_id();
    assert!(!root_id.to_string().is_empty());

    // List root directory - should be empty
    let entries = vfs2.list_directory("/").await.unwrap();
    assert!(entries.is_empty(), "Root directory should be empty");
}

#[tokio::test]
async fn test_bundle_with_complex_structure() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    // Create complex directory structure
    let paths = vec![
        ("/project/src/main.rs", "fn main() {}"),
        ("/project/src/lib.rs", "pub mod utils;"),
        ("/project/src/utils/mod.rs", "pub fn helper() {}"),
        ("/project/Cargo.toml", "[package]\nname = \"test\""),
        ("/project/README.md", "# Test Project"),
        ("/data/users.json", r#"[{"id": 1}]"#),
        ("/data/config/app.yml", "debug: true"),
        ("/logs/2024/01/app.log", "INFO: Started"),
    ];

    // Create all directories and files
    for (path, content) in &paths {
        // Create parent directories if needed
        let parts: Vec<&str> = path.split('/').collect();
        let mut current = String::new();
        for part in &parts[1..parts.len() - 1] {
            current.push('/');
            current.push_str(part);
            if !vfs.exists(&current).await.unwrap() {
                vfs.create_directory(&current).await.unwrap();
            }
        }

        // Create the file
        vfs.create_document(path, content.to_string())
            .await
            .unwrap();
    }

    // Save to bundle
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();

    // Load in new instance
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let vfs2 = tonk2.vfs();

    // Verify all paths exist
    for (path, _) in &paths {
        assert!(
            vfs2.exists(path).await.unwrap(),
            "Path {} should exist after bundle load",
            path
        );
    }

    // Verify directory structure
    let project_files = vfs2.list_directory("/project").await.unwrap();
    assert_eq!(project_files.len(), 3); // src/, Cargo.toml, README.md

    let src_files = vfs2.list_directory("/project/src").await.unwrap();
    assert_eq!(src_files.len(), 3); // main.rs, lib.rs, utils/
}

#[tokio::test]
async fn test_bundle_file_persistence() {
    use tempfile::NamedTempFile;

    // Create a bundle with content
    let tonk1 = TonkCore::new().await.unwrap();
    tonk1
        .vfs()
        .create_document("/test.txt", "Persistent content".to_string())
        .await
        .unwrap();

    // Save to a temporary file
    let temp_file = NamedTempFile::new().unwrap();
    let file_path = temp_file.path();
    tonk1.to_file(file_path).await.unwrap();

    // Verify file was created and has content
    let file_size = std::fs::metadata(file_path).unwrap().len();
    assert!(file_size > 0, "Bundle file should not be empty");

    // Load from file
    let tonk2 = TonkCore::from_file(file_path).await.unwrap();

    // Verify content
    assert!(tonk2.vfs().exists("/test.txt").await.unwrap());
}

#[tokio::test]
async fn test_multiple_save_load_cycles() {
    let mut tonk = TonkCore::new().await.unwrap();

    // Cycle 1: Add initial content
    tonk.vfs()
        .create_document("/cycle1.txt", "First cycle".to_string())
        .await
        .unwrap();

    let bytes1 = tonk.to_bytes(None).await.unwrap();
    tonk = TonkCore::from_bytes(bytes1).await.unwrap();

    // Verify cycle 1 content exists
    assert!(tonk.vfs().exists("/cycle1.txt").await.unwrap());

    // Cycle 2: Add more content
    tonk.vfs()
        .create_document("/cycle2.txt", "Second cycle".to_string())
        .await
        .unwrap();

    let bytes2 = tonk.to_bytes(None).await.unwrap();
    tonk = TonkCore::from_bytes(bytes2).await.unwrap();

    // Verify both files exist
    assert!(tonk.vfs().exists("/cycle1.txt").await.unwrap());
    assert!(tonk.vfs().exists("/cycle2.txt").await.unwrap());

    // Cycle 3: Modify existing and add new
    // Since update_document doesn't exist, we'll remove and recreate
    tonk.vfs().remove_document("/cycle1.txt").await.unwrap();
    tonk.vfs()
        .create_document("/cycle1.txt", "First cycle - modified".to_string())
        .await
        .unwrap();
    tonk.vfs()
        .create_document("/cycle3.txt", "Third cycle".to_string())
        .await
        .unwrap();

    let bytes3 = tonk.to_bytes(None).await.unwrap();
    let final_tonk = TonkCore::from_bytes(bytes3).await.unwrap();

    // Verify all changes
    assert!(final_tonk.vfs().exists("/cycle1.txt").await.unwrap());
    assert!(final_tonk.vfs().exists("/cycle2.txt").await.unwrap());
    assert!(final_tonk.vfs().exists("/cycle3.txt").await.unwrap());
}

#[tokio::test]
async fn test_bundle_preserves_timestamps() {
    let tonk1 = TonkCore::new().await.unwrap();

    // Create a file and wait a bit
    tonk1
        .vfs()
        .create_document("/timed.txt", "Content with timestamp".to_string())
        .await
        .unwrap();

    // Get the original timestamps
    let metadata1 = tonk1.vfs().metadata("/timed.txt").await.unwrap();
    let created1 = metadata1.timestamps.created;
    let modified1 = metadata1.timestamps.modified;

    // Save and load
    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Get timestamps after load
    let metadata2 = tonk2.vfs().metadata("/timed.txt").await.unwrap();

    // Timestamps should be preserved
    assert_eq!(
        metadata2.timestamps.created, created1,
        "Created timestamp should be preserved"
    );
    assert_eq!(
        metadata2.timestamps.modified, modified1,
        "Modified timestamp should be preserved"
    );
}

#[tokio::test]
async fn test_bundle_with_special_characters() {
    let tonk = TonkCore::new().await.unwrap();
    let vfs = tonk.vfs();

    // Create files with special characters and unicode
    let special_files = vec![
        ("/file with spaces.txt", "Spaces in filename"),
        ("/special-chars!@#.txt", "Special characters"),
        ("/unicode-文件.txt", "Unicode filename"),
        ("/emoji-🎉.txt", "Emoji in filename"),
    ];

    for (path, content) in &special_files {
        vfs.create_document(path, content.to_string())
            .await
            .unwrap();
    }

    // Bundle and reload
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify all files exist with correct names
    for (path, _) in &special_files {
        assert!(
            tonk2.vfs().exists(path).await.unwrap(),
            "File {} should exist after bundle load",
            path
        );
    }
}

#[tokio::test]
async fn test_peer_id_regeneration() {
    // Create TonkCore and save to bundle
    let tonk1 = TonkCore::new().await.unwrap();
    let peer_id1 = tonk1.peer_id();

    tonk1
        .vfs()
        .create_document("/test.txt", "Content".to_string())
        .await
        .unwrap();

    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();

    // Load from bundle multiple times
    let tonk2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let peer_id2 = tonk2.peer_id();

    let tonk3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let peer_id3 = tonk3.peer_id();

    // All peer IDs should be different
    assert_ne!(peer_id1, peer_id2, "Peer IDs should be regenerated");
    assert_ne!(peer_id2, peer_id3, "Each load should generate new peer ID");
    assert_ne!(peer_id1, peer_id3, "All peer IDs should be unique");

    // But content should be the same
    assert!(tonk2.vfs().exists("/test.txt").await.unwrap());
    assert!(tonk3.vfs().exists("/test.txt").await.unwrap());
}

#[tokio::test]
async fn test_concurrent_bundle_operations() {
    use futures::future::join_all;

    // Create a bundle with some content
    let tonk = TonkCore::new().await.unwrap();
    for i in 0..10 {
        tonk.vfs()
            .create_document(&format!("/file{}.txt", i), format!("Content {}", i))
            .await
            .unwrap();
    }

    let bundle_bytes = tonk.to_bytes(None).await.unwrap();

    // Load the same bundle concurrently
    let futures = (0..5).map(|_| {
        let bytes = bundle_bytes.clone();
        async move { TonkCore::from_bytes(bytes).await }
    });

    let results = join_all(futures).await;

    // All loads should succeed
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Concurrent load {} should succeed", i);

        // Verify content
        let tonk = result.as_ref().unwrap();
        for j in 0..10 {
            assert!(tonk.vfs().exists(&format!("/file{}.txt", j)).await.unwrap());
        }
    }
}

// #[tokio::test]
// async fn test_bundle_with_large_content() {
//     let tonk = TonkCore::new().await.unwrap();
//
//     // Create a large text file (1MB)
//     let large_content = "x".repeat(1024 * 1024 * 1024);
//     tonk.vfs()
//         .create_document("/large.txt", large_content.clone())
//         .await
//         .unwrap();
//
//     // Create many small files
//     for i in 0..100 {
//         tonk.vfs()
//             .create_document(&format!("/small{}.txt", i), format!("Small file {}", i))
//             .await
//             .unwrap();
//     }
//
//     // Save to bundle
//     let bundle_bytes = tonk.to_bytes(None).await.unwrap();
//     assert!(
//         bundle_bytes.len() > 1024 * 1024,
//         "Bundle should be larger than 1MB"
//     );
//
//     // Load and verify
//     let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
//
//     // Check large file
//     let large_doc = tonk2
//         .vfs()
//         .find_document("/large.txt")
//         .await
//         .unwrap()
//         .unwrap();
//     large_doc.with_document(|doc| {
//         use automerge::ReadDoc;
//         let content = doc.get(automerge::ROOT, "content").unwrap().unwrap().0;
//         assert_eq!(content.to_str().unwrap().len(), 1024 * 1024);
//     });
//
//     // Check small files
//     for i in 0..100 {
//         assert!(tonk2
//             .vfs()
//             .exists(&format!("/small{}.txt", i))
//             .await
//             .unwrap());
//     }
// }

#[tokio::test]
async fn test_bundle_manifest_metadata() {
    // Create bundle and verify manifest
    let tonk = TonkCore::new().await.unwrap();
    tonk.vfs()
        .create_document("/test.txt", "Test content".to_string())
        .await
        .unwrap();

    let bundle_bytes = tonk.to_bytes(None).await.unwrap();

    // Parse the bundle to check manifest
    let bundle = Bundle::from_bytes(bundle_bytes).unwrap();
    let manifest = bundle.manifest();

    assert_eq!(manifest.manifest_version, 1);
    assert_eq!(manifest.version.major, 1);
    assert_eq!(manifest.version.minor, 0);
    // root_id is a DocumentId string, just verify it exists and is not empty
    assert!(!manifest.root_id.is_empty());
    assert!(manifest.entrypoints.is_empty());
    assert!(manifest.network_uris.is_empty());

    // Check vendor metadata
    assert!(manifest.x_vendor.is_some());
    let vendor = manifest.x_vendor.as_ref().unwrap();
    assert!(vendor.get("xTonk").is_some());
}

// ============ Error Handling Tests ============

#[tokio::test]
async fn test_load_corrupted_bundle() {
    // Create invalid ZIP data
    let corrupted_data = vec![0xFF, 0xFE, 0xFD, 0xFC];

    let result = TonkCore::from_bytes(corrupted_data).await;
    assert!(result.is_err(), "Loading corrupted bundle should fail");
}

#[tokio::test]
async fn test_load_empty_bundle_data() {
    let empty_data = vec![];

    let result = TonkCore::from_bytes(empty_data).await;
    assert!(result.is_err(), "Loading empty data should fail");
}

#[tokio::test]
async fn test_bundle_without_manifest() {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    // Create a ZIP without manifest.json
    let mut zip_data = Vec::new();
    {
        let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));
        zip_writer
            .start_file("some_file.txt", SimpleFileOptions::default())
            .unwrap();
        zip_writer.write_all(b"Hello").unwrap();
        zip_writer.finish().unwrap();
    }

    let result = TonkCore::from_bytes(zip_data).await;
    assert!(
        result.is_err(),
        "Bundle without manifest should fail to load"
    );
}

#[tokio::test]
async fn test_bundle_with_invalid_manifest() {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    // Create a ZIP with invalid manifest
    let mut zip_data = Vec::new();
    {
        let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));

        // Add invalid manifest
        zip_writer
            .start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip_writer.write_all(b"{ invalid json }").unwrap();

        // Add root document
        zip_writer
            .start_file("root", SimpleFileOptions::default())
            .unwrap();
        zip_writer.write_all(&[0u8; 100]).unwrap();

        zip_writer.finish().unwrap();
    }

    let result = TonkCore::from_bytes(zip_data).await;
    assert!(result.is_err(), "Bundle with invalid manifest should fail");
}

#[tokio::test]
async fn test_bundle_with_unsupported_manifest_version() {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    // Create a ZIP with unsupported manifest version
    let mut zip_data = Vec::new();
    {
        let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));

        // Add manifest with version 2
        let manifest = serde_json::json!({
            "manifestVersion": 2,
            "version": { "major": 1, "minor": 0 },
            "root": "root",
            "entrypoints": [],
            "networkUris": []
        });

        zip_writer
            .start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip_writer
            .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();

        zip_writer.finish().unwrap();
    }

    let result = TonkCore::from_bytes(zip_data).await;
    assert!(
        result.is_err(),
        "Bundle with unsupported manifest version should fail"
    );
}

#[tokio::test]
async fn test_bundle_without_root_document() {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    // Create a ZIP with manifest but no root document
    let mut zip_data = Vec::new();
    {
        let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));

        // Add valid manifest
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "root",
            "entrypoints": [],
            "networkUris": []
        });

        zip_writer
            .start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip_writer
            .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();

        // Don't add root document

        zip_writer.finish().unwrap();
    }

    let result = TonkCore::from_bytes(zip_data).await;
    assert!(result.is_err(), "Bundle without root document should fail");
}

#[tokio::test]
async fn test_bundle_with_corrupted_root_document() {
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    // Create a ZIP with corrupted root document
    let mut zip_data = Vec::new();
    {
        let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));

        // Add valid manifest
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "root",
            "entrypoints": [],
            "networkUris": []
        });

        zip_writer
            .start_file("manifest.json", SimpleFileOptions::default())
            .unwrap();
        zip_writer
            .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();

        // Add corrupted root document (not valid Automerge)
        zip_writer
            .start_file("root", SimpleFileOptions::default())
            .unwrap();
        zip_writer
            .write_all(b"This is not a valid Automerge document")
            .unwrap();

        zip_writer.finish().unwrap();
    }

    let result = TonkCore::from_bytes(zip_data).await;
    assert!(
        result.is_err(),
        "Bundle with corrupted root document should fail"
    );
}

#[tokio::test]
async fn test_file_operations_on_nonexistent_bundle_file() {
    use std::path::Path;

    let nonexistent_path = Path::new("/tmp/nonexistent_bundle_12345.bundle");
    let result = TonkCore::from_file(nonexistent_path).await;

    assert!(result.is_err(), "Loading nonexistent file should fail");
}

#[tokio::test]
async fn test_save_bundle_to_invalid_path() {
    let tonk = TonkCore::new().await.unwrap();

    // Try to save to an invalid path
    let invalid_path = "/nonexistent_directory/bundle.zip";
    let result = tonk.to_file(invalid_path).await;

    assert!(result.is_err(), "Saving to invalid path should fail");
}

#[tokio::test]
async fn test_bundle_size_limits() {
    // Test with a very large number of files to check memory handling
    let tonk = TonkCore::new().await.unwrap();

    // Create 1000 files
    for i in 0..1000 {
        let path = format!("/stress/file_{:04}.txt", i);
        let content = format!("File number {} with some content to make it non-trivial", i);

        // Create directory if needed
        if i == 0 {
            tonk.vfs().create_directory("/stress").await.unwrap();
        }

        tonk.vfs().create_document(&path, content).await.unwrap();
    }

    // Try to bundle - should succeed but might be slow
    let start = std::time::Instant::now();
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();
    let duration = start.elapsed();

    println!(
        "Bundle with 1000 files: {} bytes, took {:?}",
        bundle_bytes.len(),
        duration
    );

    // Verify we can load it back
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Spot check some files
    assert!(tonk2.vfs().exists("/stress/file_0000.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/stress/file_0500.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/stress/file_0999.txt").await.unwrap());
}

#[tokio::test]
async fn test_bundle_with_deep_nesting() {
    let tonk = TonkCore::new().await.unwrap();

    // Create very deep directory structure
    let mut path = String::new();
    for i in 0..50 {
        path.push_str(&format!("/level{}", i));
        tonk.vfs().create_directory(&path).await.unwrap();
    }

    // Add a file at the deepest level
    path.push_str("/deep_file.txt");
    tonk.vfs()
        .create_document(&path, "Very deep content".to_string())
        .await
        .unwrap();

    // Bundle and reload
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify the deep file exists
    assert!(tonk2.vfs().exists(&path).await.unwrap());
}

#[tokio::test]
async fn test_bundle_partial_write_recovery() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    let tonk = TonkCore::new().await.unwrap();
    tonk.vfs()
        .create_document("/important.txt", "Important data".to_string())
        .await
        .unwrap();

    // Get bundle bytes
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();

    // Write only partial data to file
    let temp_file = NamedTempFile::new().unwrap();
    let mut file = temp_file.reopen().unwrap();
    file.write_all(&bundle_bytes[..bundle_bytes.len() / 2])
        .unwrap();
    drop(file);

    // Try to load the partial bundle
    let result = TonkCore::from_file(temp_file.path()).await;
    assert!(result.is_err(), "Loading partial bundle should fail");
}

// ============ Sync Integration Tests ============

#[tokio::test]
#[ignore] // Requires TypeScript server dependencies
async fn test_bundle_load_then_sync() {
    // Start TypeScript automerge-repo server
    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();

    // Create first TonkCore with content
    let tonk1 = TonkCore::new().await.unwrap();
    tonk1
        .vfs()
        .create_document("/pre-bundle.txt", "Created before bundling".to_string())
        .await
        .unwrap();

    // Save to bundle
    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();

    // Load bundle in new instance and connect to sync
    let tonk2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let _ = tonk2.connect_websocket(&server_url).await;

    // Load same bundle in another instance and connect
    let tonk3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let _ = tonk3.connect_websocket(&server_url).await;

    // Create new content in tonk2
    tonk2
        .vfs()
        .create_document(
            "/post-bundle.txt",
            "Created after loading bundle".to_string(),
        )
        .await
        .unwrap();

    // Wait for sync
    sleep(Duration::from_secs(1)).await;

    // Verify tonk3 sees both files
    assert!(tonk3.vfs().exists("/pre-bundle.txt").await.unwrap());
    assert!(tonk3.vfs().exists("/post-bundle.txt").await.unwrap());
}

#[tokio::test]
async fn test_offline_bundle_online_workflow() {
    // Simulate offline work -> bundle -> online sync workflow

    // Phase 1: Work offline
    let offline_tonk = TonkCore::new().await.unwrap();

    // Create complex offline structure
    offline_tonk
        .vfs()
        .create_directory("/project")
        .await
        .unwrap();
    offline_tonk
        .vfs()
        .create_document(
            "/project/README.md",
            "# My Project\nOffline work".to_string(),
        )
        .await
        .unwrap();
    offline_tonk
        .vfs()
        .create_document("/project/main.js", "console.log('offline');".to_string())
        .await
        .unwrap();
    offline_tonk
        .vfs()
        .create_directory("/project/src")
        .await
        .unwrap();
    offline_tonk
        .vfs()
        .create_document(
            "/project/src/utils.js",
            "export function util() {}".to_string(),
        )
        .await
        .unwrap();

    // Phase 2: Save to bundle
    let bundle_bytes = offline_tonk.to_bytes(None).await.unwrap();

    // Simulate transport (e.g., USB drive, email attachment)
    // ... time passes ...

    // Phase 3: Load bundle on different machine
    let online_tonk = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify all offline work is preserved
    assert!(online_tonk
        .vfs()
        .exists("/project/README.md")
        .await
        .unwrap());
    assert!(online_tonk.vfs().exists("/project/main.js").await.unwrap());
    assert!(online_tonk
        .vfs()
        .exists("/project/src/utils.js")
        .await
        .unwrap());

    // Phase 4: Continue working online
    online_tonk
        .vfs()
        .create_document("/project/config.json", r#"{"online": true}"#.to_string())
        .await
        .unwrap();

    // Could connect to sync server here
    // let _ = online_tonk.connect_websocket("wss://sync.example.com").await;
}

#[tokio::test]
async fn test_multiple_peers_from_same_bundle() {
    // Test that multiple peers loading the same bundle get different peer IDs
    // but same content and can sync together

    let original = TonkCore::new().await.unwrap();
    original
        .vfs()
        .create_document("/shared.txt", "Shared content".to_string())
        .await
        .unwrap();

    let bundle_bytes = original.to_bytes(None).await.unwrap();

    // Load same bundle into 3 different peers
    let peer1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let peer2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let peer3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // All should have different peer IDs
    let id1 = peer1.peer_id();
    let id2 = peer2.peer_id();
    let id3 = peer3.peer_id();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);

    // But all should have the same content
    assert!(peer1.vfs().exists("/shared.txt").await.unwrap());
    assert!(peer2.vfs().exists("/shared.txt").await.unwrap());
    assert!(peer3.vfs().exists("/shared.txt").await.unwrap());
}

#[tokio::test]
async fn test_bundle_crdt_merge_behavior() {
    // Test that CRDT merge behavior works correctly after bundle load

    // Create two TonkCores with conflicting changes to same document
    let tonk1 = TonkCore::new().await.unwrap();
    let tonk2 = TonkCore::new().await.unwrap();

    // Create same document in both
    tonk1
        .vfs()
        .create_document("/conflict.txt", "Version from tonk1".to_string())
        .await
        .unwrap();

    // Bundle tonk1
    let bundle1 = tonk1.to_bytes(None).await.unwrap();

    // Meanwhile, tonk2 creates its own version
    tonk2
        .vfs()
        .create_document("/conflict.txt", "Version from tonk2".to_string())
        .await
        .unwrap();

    // Now load tonk1's bundle into a new instance
    let tonk3 = TonkCore::from_bytes(bundle1).await.unwrap();

    // In a real scenario, tonk2 and tonk3 would sync through a server
    // Here we just verify they both have their respective versions
    assert!(tonk2.vfs().exists("/conflict.txt").await.unwrap());
    assert!(tonk3.vfs().exists("/conflict.txt").await.unwrap());

    // The actual merge would happen during sync, with CRDT resolving conflicts
}

#[tokio::test]
async fn test_bundle_with_network_uris_in_manifest() {
    // Test that we can include network URIs in bundle manifest
    // (Note: actual connection functionality is not yet implemented)

    let tonk = TonkCore::new().await.unwrap();
    tonk.vfs()
        .create_document("/networked.txt", "Content for networked bundle".to_string())
        .await
        .unwrap();

    // Save bundle (network URIs would be set via manifest)
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();

    // Load and verify bundle structure
    let bundle = Bundle::from_bytes(bundle_bytes.clone()).unwrap();
    let manifest = bundle.manifest();

    // Currently network_uris is empty, but the field exists
    assert_eq!(manifest.network_uris, Vec::<String>::new());

    // Load bundle into new TonkCore
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    assert!(tonk2.vfs().exists("/networked.txt").await.unwrap());
}

#[tokio::test]
async fn test_sync_after_bundle_modifications() {
    // Test that modifications after bundle load are properly synced

    // Create and bundle initial state
    let tonk1 = TonkCore::new().await.unwrap();
    tonk1
        .vfs()
        .create_document("/initial.txt", "Initial content".to_string())
        .await
        .unwrap();

    let bundle_bytes = tonk1.to_bytes(None).await.unwrap();

    // Load bundle and make modifications
    let tonk2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();

    // Modify existing file
    // Since update_document doesn't exist, we'll remove and recreate
    tonk2.vfs().remove_document("/initial.txt").await.unwrap();
    tonk2
        .vfs()
        .create_document("/initial.txt", "Modified content".to_string())
        .await
        .unwrap();

    // Add new file
    tonk2
        .vfs()
        .create_document("/added.txt", "Added after bundle load".to_string())
        .await
        .unwrap();

    // Save modified state to new bundle
    let bundle2_bytes = tonk2.to_bytes(None).await.unwrap();

    // Load in third instance
    let tonk3 = TonkCore::from_bytes(bundle2_bytes).await.unwrap();

    // Verify all modifications are present
    assert!(tonk3.vfs().exists("/initial.txt").await.unwrap());
    assert!(tonk3.vfs().exists("/added.txt").await.unwrap());

    // Verify content is updated
    let doc = tonk3
        .vfs()
        .find_document("/initial.txt")
        .await
        .unwrap()
        .unwrap();
    doc.with_document(|d| {
        use automerge::ReadDoc;
        let content = d.get(automerge::ROOT, "content").unwrap().unwrap().0;
        // Extract content properly by removing JSON quotes
        let content_str = content.to_str().unwrap();
        // content.to_str() returns JSON-serialized string with quotes
        assert_eq!(content_str, "\"Modified content\"");
    });
}

#[tokio::test]
async fn test_bundle_storage_isolation() {
    // Test that each bundle load gets its own isolated storage

    let bundle_bytes = {
        let tonk = TonkCore::new().await.unwrap();
        tonk.vfs()
            .create_document("/test.txt", "Test content".to_string())
            .await
            .unwrap();
        tonk.to_bytes(None).await.unwrap()
    };

    // Load same bundle multiple times concurrently
    let tonk1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let tonk2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();

    // Make different modifications in each
    tonk1
        .vfs()
        .create_document("/only-in-1.txt", "Unique to tonk1".to_string())
        .await
        .unwrap();

    tonk2
        .vfs()
        .create_document("/only-in-2.txt", "Unique to tonk2".to_string())
        .await
        .unwrap();

    // Verify isolation - each should only see its own changes
    assert!(tonk1.vfs().exists("/only-in-1.txt").await.unwrap());
    assert!(!tonk1.vfs().exists("/only-in-2.txt").await.unwrap());

    assert!(tonk2.vfs().exists("/only-in-2.txt").await.unwrap());
    assert!(!tonk2.vfs().exists("/only-in-1.txt").await.unwrap());
}

// ============ Stress and Performance Tests ============

#[tokio::test]
#[ignore] // This test is slow, run with --ignored flag
async fn test_bundle_stress_many_small_files() {
    // Create 5000 small files to stress test bundle creation
    let tonk = TonkCore::new().await.unwrap();

    println!("Creating 5000 small files...");
    let start = std::time::Instant::now();

    // Create directories
    for i in 0..50 {
        tonk.vfs()
            .create_directory(&format!("/dir{}", i))
            .await
            .unwrap();
    }

    // Create files distributed across directories
    for i in 0..5000 {
        let dir = i % 50;
        let path = format!("/dir{}/file_{:04}.txt", dir, i);
        let content = format!("Small file {} with minimal content", i);
        tonk.vfs().create_document(&path, content).await.unwrap();
    }

    let creation_time = start.elapsed();
    println!("Created 5000 files in {:?}", creation_time);

    // Bundle creation
    let bundle_start = std::time::Instant::now();
    let bundle_bytes = tonk.to_bytes(None).await.unwrap();
    let bundle_time = bundle_start.elapsed();

    println!(
        "Bundle created: {} bytes in {:?}",
        bundle_bytes.len(),
        bundle_time
    );

    // Bundle loading
    let load_start = std::time::Instant::now();
    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    let load_time = load_start.elapsed();

    println!("Bundle loaded in {:?}", load_time);

    // Verify sampling of files
    assert!(tonk2.vfs().exists("/dir0/file_0000.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/dir25/file_2500.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/dir49/file_4999.txt").await.unwrap());
}

#[tokio::test]
#[ignore] // This test is memory intensive
async fn test_bundle_memory_stress() {
    // Test with progressively larger content to find memory limits
    let sizes = vec![1, 10, 50, 100]; // MB

    for size_mb in sizes {
        println!("\nTesting with {}MB content", size_mb);

        let tonk = TonkCore::new().await.unwrap();
        let content = "x".repeat(size_mb * 1024 * 1024);

        let create_start = std::time::Instant::now();
        tonk.vfs()
            .create_document(&format!("/large_{}mb.txt", size_mb), content)
            .await
            .unwrap();
        println!("Created {}MB file in {:?}", size_mb, create_start.elapsed());

        let bundle_start = std::time::Instant::now();
        let bundle_bytes = tonk.to_bytes(None).await.unwrap();
        println!(
            "Bundle size: {} bytes, created in {:?}",
            bundle_bytes.len(),
            bundle_start.elapsed()
        );

        let load_start = std::time::Instant::now();
        let _ = TonkCore::from_bytes(bundle_bytes).await.unwrap();
        println!("Bundle loaded in {:?}", load_start.elapsed());
    }
}

#[tokio::test]
async fn test_bundle_concurrent_modifications() {
    use futures::future::join_all;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let tonk = Arc::new(Mutex::new(TonkCore::new().await.unwrap()));

    // Create base structure
    {
        let tonk = tonk.lock().await;
        tonk.vfs().create_directory("/concurrent").await.unwrap();
    }

    // Spawn multiple tasks that create files concurrently
    let futures = (0..20).map(|i| {
        let tonk_clone = Arc::clone(&tonk);
        async move {
            let tonk = tonk_clone.lock().await;
            tonk.vfs()
                .create_document(
                    &format!("/concurrent/task_{}.txt", i),
                    format!("Created by task {}", i),
                )
                .await
        }
    });

    let results = join_all(futures).await;

    // All should succeed
    for (i, result) in results.iter().enumerate() {
        assert!(result.is_ok(), "Task {} should succeed", i);
    }

    // Bundle and verify
    let bundle_bytes = {
        let tonk = tonk.lock().await;
        tonk.to_bytes(None).await.unwrap()
    };

    let tonk2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify all files exist
    for i in 0..20 {
        assert!(tonk2
            .vfs()
            .exists(&format!("/concurrent/task_{}.txt", i))
            .await
            .unwrap());
    }
}

#[tokio::test]
async fn test_bundle_rapid_save_load_cycles() {
    // Test rapid save/load cycles to check for resource leaks
    let mut tonk = TonkCore::new().await.unwrap();

    tonk.vfs()
        .create_document("/persistent.txt", "Initial content".to_string())
        .await
        .unwrap();

    for i in 0..50 {
        // Modify content
        // Since update_document doesn't exist, we'll remove and recreate
        tonk.vfs().remove_document("/persistent.txt").await.unwrap();
        tonk.vfs()
            .create_document("/persistent.txt", format!("Iteration {}", i))
            .await
            .unwrap();

        // Save to bundle
        let bytes = tonk.to_bytes(None).await.unwrap();

        // Load from bundle (creates new TonkCore)
        tonk = TonkCore::from_bytes(bytes).await.unwrap();

        // Verify content
        let doc = tonk
            .vfs()
            .find_document("/persistent.txt")
            .await
            .unwrap()
            .unwrap();
        doc.with_document(|d| {
            use automerge::ReadDoc;
            let content = d.get(automerge::ROOT, "content").unwrap().unwrap().0;
            // Extract content properly by removing JSON quotes
            let content_str = content.to_str().unwrap();
            let expected = format!("Iteration {}", i);
            // content.to_str() returns JSON-serialized string with quotes
            assert_eq!(content_str, format!("\"{}\"", expected));
        });
    }
}

#[tokio::test]
async fn test_load_bundle_from_blank_bundle_tonk_file() {
    use tonk_core::StorageConfig;
    
    // Load the blank.tonk file from the test data directory
    let bundle_bytes = std::fs::read("tests/data/blank.tonk")
        .expect("Should be able to read tests/data/blank.tonk");
    
    // Test the TonkCore::builder().from_bytes() code path with different storage configs
    
    // Test with in-memory storage
    let tonk_memory = TonkCore::builder()
        .with_storage(StorageConfig::InMemory)
        .from_bytes(bundle_bytes.clone())
        .await
        .expect("Should load blank.tonk with in-memory storage");
    
    // Verify the TonkCore instance was created successfully
    let vfs = tonk_memory.vfs();
    let root_id = vfs.root_id();
    assert!(!root_id.to_string().is_empty(), "Root ID should not be empty");
    
    // Test with filesystem storage (non-WASM only)
    #[cfg(not(target_arch = "wasm32"))]
    {
        use tempfile::tempdir;
        let temp_dir = tempdir().expect("Should create temp directory");
        
        let tonk_filesystem = TonkCore::builder()
            .with_storage(StorageConfig::Filesystem(temp_dir.path().to_path_buf()))
            .from_bytes(bundle_bytes.clone())
            .await
            .expect("Should load blank.tonk with filesystem storage");
        
        // Verify the TonkCore instance was created successfully
        let vfs_fs = tonk_filesystem.vfs();
        let root_id_fs = vfs_fs.root_id();
        assert!(!root_id_fs.to_string().is_empty(), "Root ID should not be empty");
        
        // Verify different instances have different peer IDs
        assert_ne!(
            tonk_memory.peer_id(), 
            tonk_filesystem.peer_id(), 
            "Different instances should have different peer IDs"
        );
    }
    
    // Test that we can use the VFS after loading
    let entries = vfs.list_directory("/").await.expect("Should be able to list root directory");
    // The blank.tonk file should have an empty or minimal structure
    println!("Root directory entries: {:?}", entries);
    
    // Verify we can create new content in the loaded VFS
    vfs.create_document("/test_after_load.txt", "Content added after loading blank.tonk".to_string())
        .await
        .expect("Should be able to create documents in loaded VFS");
    
    // Verify the document was created
    assert!(vfs.exists("/test_after_load.txt").await.expect("Should be able to check existence"));
    
    // Test that we can save the modified state back to bytes
    let new_bundle_bytes = tonk_memory.to_bytes(None).await
        .expect("Should be able to export modified state to bytes");
    
    // Verify the new bundle is larger (contains our new document)
    assert!(
        new_bundle_bytes.len() > bundle_bytes.len(),
        "Modified bundle should be larger than original slim bundle"
    );
    
    // Test loading the modified bundle
    let tonk_modified = TonkCore::builder()
        .with_storage(StorageConfig::InMemory)
        .from_bytes(new_bundle_bytes)
        .await
        .expect("Should load modified bundle");
    
    // Verify our added document exists in the reloaded bundle
    assert!(tonk_modified.vfs().exists("/test_after_load.txt").await.expect("Should be able to check existence"));
}
