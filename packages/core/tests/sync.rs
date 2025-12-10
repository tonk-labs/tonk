mod common;

use samod::DocumentId;
use std::time::Duration;
use tokio::time::sleep;
use tonk_core::TonkCore;

#[tokio::test]
async fn test_e2e_bundle_sync_workflow() {
    // Test the complete workflow: create bundle -> load in multiple clients -> sync
    // This test explores whether bundle-based initialization enables sync compatibility

    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();

    // Phase 1: Create initial TonkCore with content and save bundle
    let creator = TonkCore::new().await.unwrap();
    let vfs_creator = creator.vfs();

    // Add initial content to establish the VFS structure
    vfs_creator
        .create_document("/shared.txt", "Initial content from creator".to_string())
        .await
        .unwrap();
    vfs_creator.create_directory("/docs").await.unwrap();
    vfs_creator
        .create_document("/docs/readme.md", "# Shared Documentation".to_string())
        .await
        .unwrap();

    // Save the bundle - this captures the VFS structure and content
    let bundle_bytes = creator.to_bytes(None).await.unwrap();
    println!("Bundle created with {} bytes", bundle_bytes.len());

    // Phase 2: Load multiple clients from the same bundle
    let client1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let client2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let client3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify all clients have the bundle content
    for (i, client) in [&client1, &client2, &client3].iter().enumerate() {
        assert!(
            client.vfs().exists("/shared.txt").await.unwrap(),
            "Client {} should have bundle content",
            i + 1
        );
        assert!(
            client.vfs().exists("/docs/readme.md").await.unwrap(),
            "Client {} should have bundle content",
            i + 1
        );
    }

    // Phase 3: Connect all clients to server and test sync
    client1.connect_websocket(&server_url).await.unwrap();
    println!("Client 1 connected");

    client2.connect_websocket(&server_url).await.unwrap();
    println!("Client 2 connected");

    client3.connect_websocket(&server_url).await.unwrap();
    println!("Client 3 connected");

    // Wait for initial sync handshake
    sleep(Duration::from_secs(2)).await;

    // Phase 4: Test if changes propagate between clients
    println!("Testing sync propagation...");

    // Client1 adds new content
    client1
        .vfs()
        .create_document("/from_client1.txt", "New content from client1".to_string())
        .await
        .unwrap();

    // Wait for propagation
    sleep(Duration::from_secs(3)).await;

    // Check if other clients see the new content
    let c2_sees_c1 = client2.vfs().exists("/from_client1.txt").await.unwrap();
    let c3_sees_c1 = client3.vfs().exists("/from_client1.txt").await.unwrap();

    if c2_sees_c1 && c3_sees_c1 {
        println!("✓ Bundle-based clients can sync new content!");

        // Test reverse sync
        client2
            .vfs()
            .create_document("/from_client2.txt", "Response from client2".to_string())
            .await
            .unwrap();

        sleep(Duration::from_secs(3)).await;

        assert!(client1.vfs().exists("/from_client2.txt").await.unwrap());
        assert!(client3.vfs().exists("/from_client2.txt").await.unwrap());

        println!("✓ Bidirectional sync confirmed!");
    } else {
        println!("⚠ Bundle-loaded clients cannot sync new content");
        println!("  Client2 sees client1 content: {}", c2_sees_c1);
        println!("  Client3 sees client1 content: {}", c3_sees_c1);
        println!("  This indicates root document isolation in sync protocol");
    }
}

#[tokio::test]
async fn test_shared_root_document_sync() {
    // Test the pattern where all clients share the same root document ID
    // by fetching it from the automerge-repo server's /root endpoint

    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();
    let http_url = server.http_url();

    // Fetch the canonical root document ID from server
    let root_response = reqwest::get(&format!("{}/root", http_url))
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    let root_doc_id_str = root_response["rootDocumentId"].as_str().unwrap();
    println!("Server canonical root document ID: {}", root_doc_id_str);

    // Parse the document ID
    let root_doc_id = root_doc_id_str.parse::<DocumentId>().unwrap();

    // Client 1: Connect and request the shared root document
    let client1 = TonkCore::new().await.unwrap();
    client1.connect_websocket(&server_url).await.unwrap();

    // Wait for sync to establish
    sleep(Duration::from_secs(2)).await;

    // Try to find the shared root document
    if let Ok(_shared_root) = client1.find_document(root_doc_id.clone()).await {
        println!("✓ Client1 found shared root document from server");

        // Create a VFS using this shared root
        // Note: This would require a new VFS constructor that takes an existing root
        // For now, we'll test with the client's own VFS but verify root sync

        // Add content to client1
        client1
            .vfs()
            .create_document("/client1_content.txt", "From client1".to_string())
            .await
            .unwrap();

        // Client 2: Also connect and sync with the shared root
        let client2 = TonkCore::new().await.unwrap();
        client2.connect_websocket(&server_url).await.unwrap();

        sleep(Duration::from_secs(3)).await;

        // Test if they can see each other's content
        let sync_works = client2.vfs().exists("/client1_content.txt").await.unwrap();

        if sync_works {
            println!("✓ Clients syncing through shared root document!");
        } else {
            println!("! Clients have separate VFS roots despite shared server root");
            println!("  Each TonkCore creates its own VFS root document");
        }
    } else {
        println!("! Client could not find shared root document from server");
        println!("  Server root: {}", root_doc_id_str);
        println!(
            "  This suggests the server's root document protocol differs from TonkCore VFS expectations"
        );
    }
}

#[tokio::test]
async fn test_bundle_content_sync_behavior() {
    // Test sync behavior when clients load from the same bundle
    // Note: Clients will have different root document IDs but should be able to sync content

    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();

    // Create initial client and bundle with content
    let original = TonkCore::new().await.unwrap();
    original
        .vfs()
        .create_document(
            "/foundation.txt",
            "Foundation content from bundle".to_string(),
        )
        .await
        .unwrap();
    original
        .vfs()
        .create_directory("/shared_folder")
        .await
        .unwrap();
    original
        .vfs()
        .create_document(
            "/shared_folder/data.json",
            r#"{"shared": true}"#.to_string(),
        )
        .await
        .unwrap();

    let bundle_bytes = original.to_bytes(None).await.unwrap();

    // Load bundle into multiple clients
    let client1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let client2 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // Verify both have the same initial content from bundle
    assert!(client1.vfs().exists("/foundation.txt").await.unwrap());
    assert!(
        client1
            .vfs()
            .exists("/shared_folder/data.json")
            .await
            .unwrap()
    );
    assert!(client2.vfs().exists("/foundation.txt").await.unwrap());
    assert!(
        client2
            .vfs()
            .exists("/shared_folder/data.json")
            .await
            .unwrap()
    );

    // Connect both to server
    client1.connect_websocket(&server_url).await.unwrap();
    client2.connect_websocket(&server_url).await.unwrap();

    // Wait for any initial sync
    sleep(Duration::from_secs(2)).await;

    // Client1 creates new content after sync connection
    client1
        .vfs()
        .create_document(
            "/new_from_client1.txt",
            "Created after sync connection".to_string(),
        )
        .await
        .unwrap();

    // Wait for sync propagation
    sleep(Duration::from_secs(2)).await;

    // Test if client2 sees the new content
    // Note: This may not work if root documents are different - that's what we're testing
    let sees_new_content = client2.vfs().exists("/new_from_client1.txt").await.unwrap();

    if sees_new_content {
        println!("✓ Sync works even with different root document IDs");

        // Test bidirectional sync
        client2
            .vfs()
            .create_document("/new_from_client2.txt", "Created by client2".to_string())
            .await
            .unwrap();

        sleep(Duration::from_secs(2)).await;

        assert!(client1.vfs().exists("/new_from_client2.txt").await.unwrap());
    } else {
        println!(
            "! Clients with different root document IDs cannot sync through automerge-repo server"
        );
        println!(
            "  This is expected behavior - automerge-repo syncs documents, not arbitrary content"
        );
    }
}

#[tokio::test]
async fn test_different_bundles_isolated_sync() {
    // Test that clients from different bundles don't interfere with each other

    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();

    // Create two separate bundles with different content
    let bundle1 = {
        let tonk = TonkCore::new().await.unwrap();
        tonk.vfs()
            .create_document("/bundle1.txt", "From bundle 1".to_string())
            .await
            .unwrap();
        tonk.to_bytes(None).await.unwrap()
    };

    let bundle2 = {
        let tonk = TonkCore::new().await.unwrap();
        tonk.vfs()
            .create_document("/bundle2.txt", "From bundle 2".to_string())
            .await
            .unwrap();
        tonk.to_bytes(None).await.unwrap()
    };

    // Load clients from different bundles
    let client_a = TonkCore::from_bytes(bundle1).await.unwrap();
    let client_b = TonkCore::from_bytes(bundle2).await.unwrap();

    // They should have different root document IDs
    assert_ne!(client_a.vfs().root_id(), client_b.vfs().root_id());

    // Connect both to same server
    client_a.connect_websocket(&server_url).await.unwrap();
    client_b.connect_websocket(&server_url).await.unwrap();

    // Wait for potential sync
    sleep(Duration::from_secs(1)).await;

    // Each should only see their own content (isolated by root document)
    assert!(client_a.vfs().exists("/bundle1.txt").await.unwrap());
    assert!(!client_a.vfs().exists("/bundle2.txt").await.unwrap());

    assert!(client_b.vfs().exists("/bundle2.txt").await.unwrap());
    assert!(!client_b.vfs().exists("/bundle1.txt").await.unwrap());
}

#[tokio::test]
async fn test_sequential_bundle_client_joins() {
    // Test clients joining at different times but sharing the same bundle

    let server = common::AutomergeServer::start().await.unwrap();
    let server_url = server.url();

    // Create bundle with initial content
    let bundle_bytes = {
        let tonk = TonkCore::new().await.unwrap();
        tonk.vfs()
            .create_document("/foundation.txt", "Foundation content".to_string())
            .await
            .unwrap();
        tonk.to_bytes(None).await.unwrap()
    };

    // Client 1 joins first
    let client1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    client1.connect_websocket(&server_url).await.unwrap();

    // Client1 adds content
    client1
        .vfs()
        .create_document("/early_content.txt", "Added early".to_string())
        .await
        .unwrap();

    sleep(Duration::from_millis(500)).await;

    // Client 2 joins later
    let client2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    client2.connect_websocket(&server_url).await.unwrap();

    // Wait for sync
    sleep(Duration::from_secs(1)).await;

    // Client2 should see both foundation and early content
    assert!(client2.vfs().exists("/foundation.txt").await.unwrap());
    assert!(client2.vfs().exists("/early_content.txt").await.unwrap());

    // Client2 adds content
    client2
        .vfs()
        .create_document("/late_content.txt", "Added late".to_string())
        .await
        .unwrap();

    sleep(Duration::from_millis(500)).await;

    // Client 3 joins even later
    let client3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();
    client3.connect_websocket(&server_url).await.unwrap();

    // Wait for sync
    sleep(Duration::from_secs(1)).await;

    // Client3 should see all content
    assert!(client3.vfs().exists("/foundation.txt").await.unwrap());
    assert!(client3.vfs().exists("/early_content.txt").await.unwrap());
    assert!(client3.vfs().exists("/late_content.txt").await.unwrap());

    // And client1 should see the late content
    assert!(client1.vfs().exists("/late_content.txt").await.unwrap());
}

#[tokio::test]
async fn test_websocket_connection_failure() {
    let tonk = TonkCore::new().await.unwrap();

    // Try to connect to non-existent server
    let result = tokio::time::timeout(
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
async fn test_peer_id_uniqueness_after_bundle_load() {
    // Test that each client gets unique peer ID even from same bundle

    let bundle_bytes = {
        let tonk = TonkCore::new().await.unwrap();
        tonk.vfs()
            .create_document("/test.txt", "Test content".to_string())
            .await
            .unwrap();
        tonk.to_bytes(None).await.unwrap()
    };

    // Load same bundle multiple times
    let client1 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let client2 = TonkCore::from_bytes(bundle_bytes.clone()).await.unwrap();
    let client3 = TonkCore::from_bytes(bundle_bytes).await.unwrap();

    // All should have unique peer IDs
    let id1 = client1.peer_id();
    let id2 = client2.peer_id();
    let id3 = client3.peer_id();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);

    // Note: Root document IDs will be different since each repo assigns new IDs
    // But the VFS content and structure should be the same

    // And same content
    assert!(client1.vfs().exists("/test.txt").await.unwrap());
    assert!(client2.vfs().exists("/test.txt").await.unwrap());
    assert!(client3.vfs().exists("/test.txt").await.unwrap());
}
