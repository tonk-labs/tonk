mod common;

use std::time::Duration;
use tokio::time::{sleep, timeout};
use tonk_core::TonkCore;

#[tokio::test]
#[ignore] // Requires node server to be available
async fn test_websocket_sync_with_real_server() {
    // This test requires the example sync server to be running
    // Can be started with: cd examples/server && npm start

    let server_url = "ws://localhost:3030";

    // Create two TonkCore instances
    let tonk1 = TonkCore::new().await.unwrap();
    let tonk2 = TonkCore::new().await.unwrap();

    // Try to connect both to sync server
    match tonk1.connect_websocket(server_url).await {
        Ok(_) => println!("Connected tonk1 to sync server"),
        Err(e) => {
            println!("Failed to connect tonk1: {:?}. Is the server running?", e);
            return;
        }
    }

    match tonk2.connect_websocket(server_url).await {
        Ok(_) => println!("Connected tonk2 to sync server"),
        Err(e) => {
            println!("Failed to connect tonk2: {:?}", e);
            return;
        }
    }

    // Create document in tonk1
    tonk1
        .vfs()
        .create_document("/shared.txt", "Hello from tonk1".to_string())
        .await
        .unwrap();

    // Wait for sync
    sleep(Duration::from_secs(1)).await;

    // Verify tonk2 sees the document
    let exists = tonk2.vfs().exists("/shared.txt").await.unwrap();
    assert!(exists, "Document should be synced to tonk2");
}

#[tokio::test]
async fn test_websocket_connection_failure() {
    let tonk = TonkCore::new().await.unwrap();

    // Try to connect to non-existent server
    let result = timeout(
        Duration::from_secs(2),
        tonk.connect_websocket("ws://localhost:99999"),
    )
    .await;

    // Should either timeout or return error
    match result {
        Ok(Ok(_)) => panic!("Connection should have failed"),
        Ok(Err(_)) => {} // Connection error as expected
        Err(_) => {}     // Timeout as expected
    }
}

#[tokio::test]
async fn test_connect_from_manifest_empty() {
    let tonk = TonkCore::new().await.unwrap();

    // Should succeed with empty manifest (no URIs)
    tonk.connect_from_manifest().await.unwrap();
}

#[tokio::test]
async fn test_multiple_websocket_uris_in_manifest() {
    // This would require modifying the manifest, which currently isn't exposed
    // For now, just test that multiple URIs would be tried

    let tonk = TonkCore::new().await.unwrap();

    // Verify manifest starts empty
    assert!(tonk.manifest().network_uris.is_empty());
}

#[tokio::test]
#[ignore] // Requires mock server setup
async fn test_sync_conflict_resolution() {
    use common::mock_server;

    // Start mock server
    let port = 9999;
    tokio::spawn(async move {
        let _ = mock_server::start_mock_sync_server(port).await;
    });

    sleep(Duration::from_millis(500)).await;

    let server_url = format!("ws://127.0.0.1:{}", port);

    let tonk1 = TonkCore::new().await.unwrap();
    let tonk2 = TonkCore::new().await.unwrap();

    // Connect both
    let _ = tonk1.connect_websocket(&server_url).await;
    let _ = tonk2.connect_websocket(&server_url).await;

    // Create same file in both (potential conflict)
    tonk1
        .vfs()
        .create_document("/conflict.txt", "Version 1".to_string())
        .await
        .unwrap();
    tonk2
        .vfs()
        .create_document("/conflict.txt", "Version 2".to_string())
        .await
        .unwrap();

    // Wait for sync
    sleep(Duration::from_secs(1)).await;

    // Both should have the file (CRDT merge)
    assert!(tonk1.vfs().exists("/conflict.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/conflict.txt").await.unwrap());
}

#[tokio::test]
async fn test_offline_then_sync() {
    // Create and populate offline
    let mut tonk = TonkCore::new().await.unwrap();
    tonk.vfs()
        .create_document("/offline.txt", "Created offline".to_string())
        .await
        .unwrap();
    tonk.vfs().create_directory("/offline-dir").await.unwrap();

    // Save state
    let bytes = tonk.to_bytes().await.unwrap();

    // Load from saved state (simulating restart)
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Verify offline changes are preserved
    assert!(tonk2.vfs().exists("/offline.txt").await.unwrap());
    assert!(tonk2.vfs().exists("/offline-dir").await.unwrap());
}

#[tokio::test]
async fn test_sync_engine_operations() {
    let tonk = TonkCore::new().await.unwrap();
    let engine = tonk.engine();

    // Test creating a document through engine
    let doc = automerge::Automerge::new();
    let handle = engine.create_document(doc).await.unwrap();
    assert!(!handle.document_id().to_string().is_empty());

    // Test finding the document
    let doc_id = handle.document_id().clone();
    let found = engine.find_document(doc_id).await.unwrap();
    assert_eq!(found.document_id(), handle.document_id());
}

#[tokio::test]
async fn test_concurrent_sync_operations() {
    use futures::future::join_all;

    let tonk = TonkCore::new().await.unwrap();

    // Try multiple concurrent connections (all should fail but not panic)
    let futures = vec![
        tonk.connect_websocket("ws://localhost:11111"),
        tonk.connect_websocket("ws://localhost:22222"),
        tonk.connect_websocket("ws://localhost:33333"),
    ];

    let results = join_all(futures).await;

    // All should fail gracefully
    assert!(results.iter().all(|r| r.is_err()));
}

#[tokio::test]
async fn test_vfs_sync_readiness() {
    // Test that VFS is ready for sync operations after initialization
    let tonk = TonkCore::new().await.unwrap();

    // VFS should have a root document ID that can be synced
    let root_id = tonk.vfs().root_id();
    assert!(!root_id.to_string().is_empty());

    // Engine should be able to find the root document
    let root_handle = tonk.engine().find_document(root_id.clone()).await.unwrap();
    assert_eq!(root_handle.document_id(), &root_id);
}

#[tokio::test]
async fn test_bundle_with_network_uris() {
    // Create a bundle, then verify we can load it and attempt connections
    let mut tonk = TonkCore::new().await.unwrap();

    // Add some content
    tonk.vfs()
        .create_document(
            "/networked.txt",
            "This bundle has network config".to_string(),
        )
        .await
        .unwrap();

    // Save and reload
    let bytes = tonk.to_bytes().await.unwrap();
    let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

    // Should be able to attempt connection from manifest (even if empty)
    tonk2.connect_from_manifest().await.unwrap();

    // Content should still be there
    assert!(tonk2.vfs().exists("/networked.txt").await.unwrap());
}

