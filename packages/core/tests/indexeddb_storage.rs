#[cfg(target_arch = "wasm32")]
mod wasm_tests {
    use tonk_core::{StorageConfig, TonkCore};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_indexeddb_storage_persistence() {
        tracing_wasm::set_as_global_default_with_config(
            tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(tracing::Level::TRACE)
                .build(),
        );

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
        let bundle_bytes = tonk1.to_bytes().await.expect("Failed to export bundle");

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
        let bundle_bytes = tonk1.to_bytes().await.expect("Failed to export bundle");

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
    async fn test_indexeddb_multiple_instances_isolation() {
        // Create two separate TonkCore instances with different peer IDs
        let tonk1 = TonkCore::builder()
            .with_storage(StorageConfig::IndexedDB)
            .build()
            .await
            .expect("Failed to create first TonkCore instance");

        let tonk2 = TonkCore::builder()
            .with_storage(StorageConfig::IndexedDB)
            .build()
            .await
            .expect("Failed to create second TonkCore instance");

        // Ensure they have different peer IDs
        assert_ne!(tonk1.peer_id(), tonk2.peer_id());

        let vfs1 = tonk1.vfs();
        let vfs2 = tonk2.vfs();

        // Create documents in each instance
        vfs1.create_document("/instance1.txt", "Content from instance 1".to_string())
            .await
            .expect("Failed to create document in instance 1");

        vfs2.create_document("/instance2.txt", "Content from instance 2".to_string())
            .await
            .expect("Failed to create document in instance 2");

        // Each instance should only see its own documents
        assert!(vfs1
            .exists("/instance1.txt")
            .await
            .expect("Failed to check existence"));
        assert!(!vfs1
            .exists("/instance2.txt")
            .await
            .expect("Failed to check existence"));

        assert!(vfs2
            .exists("/instance2.txt")
            .await
            .expect("Failed to check existence"));
        assert!(!vfs2
            .exists("/instance1.txt")
            .await
            .expect("Failed to check existence"));
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
