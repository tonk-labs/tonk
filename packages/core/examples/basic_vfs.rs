use automerge::{transaction::Transactable, AutomergeError, ReadDoc, ROOT};
use std::sync::{Arc, Mutex};
use tonk_core::Vfs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create VFS instance
    let vfs = Vfs::new().await?;

    println!("VFS created with peer ID: {}", vfs.peer_id());
    println!("Root ID: {}", vfs.vfs().root_id());

    // Create a document using VFS paths
    vfs.create_file("/documents/example.txt", "Hello, VFS!".to_string())
        .await?;
    println!("Created document at /documents/example.txt");

    // Read the document
    if let Some(handle) = vfs.read_file("/documents/example.txt").await? {
        handle.with_document(|doc| {
            if let Ok(Some((value, _))) = doc.get(ROOT, "content") {
                println!("Document content: {value}");
            }
        });
    }

    // Set up a change listener
    if let Some(watcher) = vfs.watch_file("/documents/example.txt").await? {
        let changes = Arc::new(Mutex::new(Vec::new()));
        let changes_clone = changes.clone();

        // Spawn a task to listen for changes
        let listener_task = tokio::spawn(async move {
            watcher
                .on_change(move |doc| {
                    if let Ok(Some((value, _))) = doc.get(ROOT, "content") {
                        println!("Document changed! New content: {value}");
                        changes_clone.lock().unwrap().push(value.to_string());
                    }
                })
                .await;
        });

        // Give the listener time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Make some changes to the document
        if let Some(handle) = vfs.read_file("/documents/example.txt").await? {
            println!("\nMaking changes to the document...");

            // Change 1
            handle.with_document(|doc| {
                doc.transact::<_, _, AutomergeError>(|tx| {
                    tx.put(ROOT, "content", "Updated content!")?;
                    Ok(())
                })
                .unwrap();
            });

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            // Change 2
            handle.with_document(|doc| {
                doc.transact::<_, _, AutomergeError>(|tx| {
                    tx.put(ROOT, "content", "Final content!")?;
                    Ok(())
                })
                .unwrap();
            });

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Check changes received
        println!("\nChanges received: {:?}", changes.lock().unwrap());

        // Clean up
        listener_task.abort();
    }

    // List directory
    let files = vfs.list_dir("/documents").await?;
    println!("\nFiles in /documents:");
    for file in files {
        println!("  - {} ({:?})", file.name, file.node_type);
    }

    Ok(())
}
