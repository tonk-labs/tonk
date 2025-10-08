#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use std::sync::Once;
    use tonk_core::{StorageConfig, TonkCore};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    static TRACING_INIT: Once = Once::new();

    fn init_tracing() {
        TRACING_INIT.call_once(|| {
            tracing_wasm::set_as_global_default_with_config(
                tracing_wasm::WASMLayerConfigBuilder::new()
                    .set_max_level(tracing::Level::TRACE)
                    .build(),
            );
        });
    }

    #[wasm_bindgen_test]
    async fn test_indexeddb_storage_persistence() {
        init_tracing();

        // Create TonkCore with IndexedDB storage
        let tonk1 = TonkCore::builder()
            .with_storage(StorageConfig::IndexedDB)
            .build()
            .await
            .expect("Failed to create TonkCore with IndexedDB storage");

        let vfs1 = tonk1.vfs();

        // Create test documents
        vfs1.create_document("/test1.txt", "IndexedDB test content 1".to_string())
            .await
            .expect("Failed to create test document 1");

        vfs1.create_document("/test2.txt", "IndexedDB test content 2".to_string())
            .await
            .expect("Failed to create test document 2");

        vfs1.create_directory("/folder")
            .await
            .expect("Failed to create directory");
        vfs1.create_document(
            "/folder/nested.txt",
            "Nested content in IndexedDB".to_string(),
        )
        .await
        .expect("Failed to create nested document");

        // Verify documents exist in first instance
        assert!(vfs1
            .exists("/test1.txt")
            .await
            .expect("Failed to check existence"));
        assert!(vfs1
            .exists("/test2.txt")
            .await
            .expect("Failed to check existence"));
        assert!(vfs1
            .exists("/folder/nested.txt")
            .await
            .expect("Failed to check existence"));

        // Export to bundle to test persistence through bundle round-trip
        let bundle_bytes = tonk1.to_bytes(None).await.expect("Failed to export bundle");

        // Load from the bundle to test persistence
        let tonk3 = TonkCore::from_bundle(
            tonk_core::Bundle::from_bytes(bundle_bytes).expect("Failed to parse bundle"),
            StorageConfig::IndexedDB,
        )
        .await
        .expect("Failed to load from bundle with IndexedDB");

        let vfs3 = tonk3.vfs();

        // Verify all documents are accessible after loading from bundle
        assert!(vfs3
            .exists("/test1.txt")
            .await
            .expect("Failed to check existence"));
        assert!(vfs3
            .exists("/test2.txt")
            .await
            .expect("Failed to check existence"));
        assert!(vfs3
            .exists("/folder/nested.txt")
            .await
            .expect("Failed to check existence"));

        // Verify content is preserved
        let handle1 = vfs3
            .find_document("/test1.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        let content1: String = handle1.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content1, "\"IndexedDB test content 1\"");

        let handle2 = vfs3
            .find_document("/test2.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        let content2: String = handle2.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content2, "\"IndexedDB test content 2\"");

        let handle3 = vfs3
            .find_document("/folder/nested.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        let content3: String = handle3.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content3, "\"Nested content in IndexedDB\"");
    }

    #[wasm_bindgen_test]
    async fn test_indexeddb_storage_modification_persistence() {
        // Create TonkCore with IndexedDB storage
        let tonk1 = TonkCore::builder()
            .with_storage(StorageConfig::IndexedDB)
            .build()
            .await
            .expect("Failed to create TonkCore with IndexedDB storage");

        let vfs1 = tonk1.vfs();

        // Create and modify a document
        vfs1.create_document("/modifiable.txt", "Original content".to_string())
            .await
            .expect("Failed to create document");

        // Modify the document
        let handle = vfs1
            .find_document("/modifiable.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        handle.with_document(|doc| {
            use automerge::transaction::Transactable;
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "content", "Modified content")
                .expect("Failed to update content");
            tx.commit();
        });

        // Export to bundle
        let bundle_bytes = tonk1.to_bytes(None).await.expect("Failed to export bundle");

        // Load from bundle in new instance
        let tonk2 = TonkCore::from_bundle(
            tonk_core::Bundle::from_bytes(bundle_bytes).expect("Failed to parse bundle"),
            StorageConfig::IndexedDB,
        )
        .await
        .expect("Failed to load from bundle");

        let vfs2 = tonk2.vfs();

        // Verify modified content persisted
        let handle2 = vfs2
            .find_document("/modifiable.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        let content: String = handle2.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content, "Modified content");
    }

    #[wasm_bindgen_test]
    async fn test_offline_initialization() {
        init_tracing();

        // Step 1: Load from a bundle with IndexedDB storage
        // This simulates the first load where the user fetches the manifest
        let bundle_bytes = include_bytes!("data/blank.tonk");
        let tonk1 = TonkCore::from_bundle(
            tonk_core::Bundle::from_bytes(bundle_bytes.to_vec()).expect("Failed to parse bundle"),
            StorageConfig::IndexedDB,
        )
        .await
        .expect("Failed to load from bundle with IndexedDB");

        let vfs1 = tonk1.vfs();

        // Create some test data
        vfs1.create_document("/offline-test.txt", "Offline content".to_string())
            .await
            .expect("Failed to create test document");

        // Get the root ID for verification
        let root_id1 = tonk1.vfs().root_id();

        // Step 2: Create a new TonkCore instance with IndexedDB
        // This simulates a subsequent app load where no network is needed
        let tonk2 = TonkCore::builder()
            .with_storage(StorageConfig::IndexedDB)
            .build()
            .await
            .expect("Failed to create TonkCore from stored manifest");

        let vfs2 = tonk2.vfs();

        // Verify that the VFS was restored from the stored manifest
        let root_id2 = tonk2.vfs().root_id();
        assert_eq!(
            root_id1, root_id2,
            "Root ID should be restored from manifest"
        );

        // Verify that documents created in first instance are accessible
        assert!(
            vfs2.exists("/offline-test.txt")
                .await
                .expect("Failed to check existence"),
            "Document should persist across instances"
        );

        // Verify content is correct
        let handle = vfs2
            .find_document("/offline-test.txt")
            .await
            .expect("Failed to find document")
            .expect("Document not found");
        let content: String = handle.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content, "\"Offline content\"");

        // Step 3: Verify that we can create new documents in the restored instance
        vfs2.create_document(
            "/second-session.txt",
            "Created after restoration".to_string(),
        )
        .await
        .expect("Failed to create document in restored instance");

        assert!(
            vfs2.exists("/second-session.txt")
                .await
                .expect("Failed to check existence"),
            "Should be able to create new documents in restored instance"
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_tests {
    // These tests demonstrate how IndexedDB storage would be tested if it were available in native context
    // Currently IndexedDB is only available in WASM/browser environments

    #[tokio::test]
    async fn test_indexeddb_not_available_native() {
        // This test verifies that IndexedDB storage config is only available for WASM targets
        // In native builds, we should only have InMemory and Filesystem options

        use tonk_core::{StorageConfig, TonkCore};

        // Test that we can create TonkCore with available storage options
        let _tonk_memory = TonkCore::builder()
            .with_storage(StorageConfig::InMemory)
            .build()
            .await
            .expect("Should be able to use InMemory storage in native");

        let temp_dir = tempfile::TempDir::new().expect("Failed to create temp dir");
        let _tonk_fs = TonkCore::builder()
            .with_storage(StorageConfig::Filesystem(temp_dir.path().to_path_buf()))
            .build()
            .await
            .expect("Should be able to use Filesystem storage in native");

        // Note: StorageConfig::IndexedDB is not available in native builds due to #[cfg(target_arch = "wasm32")]
    }
}
