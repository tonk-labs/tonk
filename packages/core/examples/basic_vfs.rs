use tonk_core::Vfs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create VFS instance
    let vfs = Vfs::new().await?;

    println!("VFS created with peer ID: {}", vfs.peer_id());
    println!("Root ID: {}", vfs.vfs().root_id());

    // TODO: Test basic operations (commented out until VFS is fully implemented)
    /*
    // Create a document using VFS paths
    vfs.create_file("/documents/example.txt", "Hello, VFS!").await?;
    println!("Created document at /documents/example.txt");

    // Read the document
    if let Some(handle) = vfs.read_file("/documents/example.txt").await? {
        handle.with_document(|doc: &String| {
            println!("Document content: {}", doc);
        });
    }

    // List directory
    let files = vfs.list_dir("/documents").await?;
    println!("\nFiles in /documents:");
    for file in files {
        println!("  - {} ({:?})", file.name, file.node_type);
    }
    */

    Ok(())
}
