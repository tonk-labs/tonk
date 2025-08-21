use tonk_core::Bundle;
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Bundle Random Access Demo");
    println!("========================");

    // Create a sample ZIP file in memory for testing
    let zip_data = create_sample_zip()?;
    println!("Created sample ZIP with {} bytes", zip_data.len());

    // Load the bundle using the random access API
    let mut bundle = Bundle::from_bytes(zip_data)?;
    println!("Bundle loaded successfully!");

    // List all keys in the bundle
    let keys = bundle.list_keys();
    println!("\nFound {} keys in bundle:", keys.len());
    for key in &keys {
        println!("  - {:?}", key);
    }

    // Test reading individual keys
    println!("\nReading individual keys:");
    for key in &keys {
        if let Some(data) = bundle.get(key)? {
            let content = String::from_utf8_lossy(&data);
            println!("  {:?}: {} bytes -> \"{}\"", key, data.len(), content.trim());
        }
    }

    // Test prefix queries
    println!("\nTesting prefix queries:");
    
    // Query all files under "docs"
    let docs_prefix = vec!["docs".to_string()];
    let docs_results = bundle.get_prefix(&docs_prefix)?;
    println!("Files with prefix {:?}: {} matches", docs_prefix, docs_results.len());
    for (key, data) in docs_results {
        let content = String::from_utf8_lossy(&data);
        println!("  {:?}: \"{}\"", key, content.trim());
    }

    // Query all files under "config"
    let config_prefix = vec!["config".to_string()];
    let config_results = bundle.get_prefix(&config_prefix)?;
    println!("Files with prefix {:?}: {} matches", config_prefix, config_results.len());
    for (key, data) in config_results {
        let content = String::from_utf8_lossy(&data);
        println!("  {:?}: \"{}\"", key, content.trim());
    }

    println!("\nDemo completed successfully!");
    Ok(())
}

fn create_sample_zip() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut zip_data = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut zip_data));
        let options = SimpleFileOptions::default();

        // Add some sample files with hierarchical keys
        let files = vec![
            ("docs/readme.txt", "This is the README file\nContains important information"),
            ("docs/guide.txt", "User guide content\nStep by step instructions"),
            ("config/settings.json", r#"{"theme": "dark", "lang": "en"}"#),
            ("config/database.conf", "host=localhost\nport=5432\ndb=myapp"),
            ("data/users.csv", "id,name,email\n1,Alice,alice@example.com\n2,Bob,bob@example.com"),
            ("data/logs/app.log", "2024-01-01 10:00:00 INFO Application started\n2024-01-01 10:01:00 INFO User logged in"),
        ];

        for (path, content) in files {
            zip.start_file(path, options)?;
            zip.write_all(content.as_bytes())?;
        }

        zip.finish()?;
    }
    Ok(zip_data)
}
