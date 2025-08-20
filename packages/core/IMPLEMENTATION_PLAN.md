# Implementation Plan: Rust Port of KeepSync (Leveraging Samod)

## Overview

This plan creates a Virtual File System (VFS) layer on top of samod's existing automerge-repo
implementation. We'll leverage samod's built-in WebSocket support and in-memory storage rather than
reimplementing them.

## Phase 1: Project Setup and Core Structure

### 1.1 Create New Rust Project

```bash
cargo new --lib core
cd core
```

### 1.2 Set Up Dependencies

Add to `Cargo.toml`:

```toml
[package]
name = "tonk-core"
version = "0.1.0"
edition = "2021"

[dependencies]
# Core dependencies - samod already provides WebSocket and storage
samod = { version = "0.2", features = ["tokio", "tungstenite"] }
automerge = "0.6"

# Async runtime and utilities
tokio = { version = "1", features = ["full"] }
tokio-stream = "0.1"
futures = "0.3"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error handling
anyhow = "1"
thiserror = "1"

# Utilities
tracing = "0.1"
tracing-subscriber = "0.3"
chrono = "0.4"

[dev-dependencies]
tempfile = "3"
tokio-tungstenite = "0.24"  # For test clients
```

### 1.3 Project Structure (Simplified)

```
core/
├── src/
│   ├── lib.rs           # Public API exports
│   ├── vfs/             # Virtual file system layer
│   │   ├── mod.rs
│   │   ├── types.rs     # Node types (RefNode, DirNode, etc.)
│   │   ├── traversal.rs # Path traversal logic
│   │   └── operations.rs # CRUD operations
│   ├── sync/            # Sync engine wrapper
│   │   ├── mod.rs
│   │   └── engine.rs    # Integration with samod
│   └── error.rs         # Error types
├── examples/
│   ├── basic_vfs.rs     # Basic VFS usage
│   ├── websocket_sync.rs # WebSocket sync example
│   └── server.rs        # WebSocket server using samod
└── tests/
    ├── vfs_tests.rs
    └── sync_tests.rs
```

## Phase 2: Core Data Structures

### 2.1 Define VFS Types (`src/vfs/types.rs`)

```rust
use samod::DocumentId;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    #[serde(rename = "doc")]
    Document,
    #[serde(rename = "dir")]
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timestamps {
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
}

impl Timestamps {
    pub fn now() -> Self {
        let now = Utc::now();
        Self {
            created: now,
            modified: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefNode {
    pub pointer: DocumentId,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub timestamps: Timestamps,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirNode {
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: String,
    pub timestamps: Timestamps,
    pub children: Vec<RefNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocNode<T> {
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: String,
    pub timestamps: Timestamps,
    pub content: T,
}
```

### 2.2 Error Types (`src/error.rs`)

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VfsError {
    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("Document already exists at path: {0}")]
    DocumentExists(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Cannot create document at root path")]
    RootPathError,

    #[error("Node type mismatch: expected {expected}, got {actual}")]
    NodeTypeMismatch { expected: String, actual: String },

    #[error("Automerge error: {0}")]
    AutomergeError(#[from] automerge::AutomergeError),

    #[error("Samod error: {0}")]
    SamodError(String),

    #[error("WebSocket error: {0}")]
    WebSocketError(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, VfsError>;
```

## Phase 3: Leveraging Samod's Built-in Storage

Samod already provides `InMemoryStorage` in `samod::storage::InMemoryStorage`, so we don't need to
reimplement it. We'll use it directly:

```rust
use samod::storage::InMemoryStorage;

// Usage in sync engine
let storage = InMemoryStorage::new();
let samod = Samod::build_tokio()
    .with_storage(storage)
    .with_peer_id(peer_id)
    .load()
    .await;
```

## Phase 4: Leveraging Samod's WebSocket Support

Samod already provides WebSocket support through `connect_tungstenite()` and `connect_websocket()`
methods. We'll use these directly instead of implementing our own WebSocket layer:

```rust
use samod::{Samod, ConnDirection};
use tokio_tungstenite::connect_async;

// Client connection example
async fn connect_to_peer(samod: &Samod, url: &str) {
    let (ws_stream, _) = connect_async(url).await.unwrap();

    // Use samod's built-in WebSocket support
    let conn_finished = samod.connect_tungstenite(
        ws_stream,
        ConnDirection::Outgoing
    ).await;

    println!("Connection finished: {:?}", conn_finished);
}

// Server example using samod with axum (if using axum feature)
#[cfg(feature = "axum")]
async fn handle_websocket(
    ws: axum::extract::ws::WebSocketUpgrade,
    samod: Samod,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| async move {
        samod.accept_axum(socket).await;
    })
}
```

The automerge-repo protocol handles all the document synchronization automatically through samod.

## Phase 5: Sync Engine Integration

### 5.1 Sync Engine (`src/sync/engine.rs`)

The sync engine wraps samod and adds the VFS layer on top:

```rust
use crate::vfs::VirtualFileSystem;
use crate::error::Result;
use samod::{Samod, DocumentId, PeerId, ConnDirection};
use samod::storage::InMemoryStorage;
use std::sync::Arc;
use tokio_tungstenite::connect_async;

pub struct SyncEngine {
    samod: Arc<Samod>,
    vfs: Arc<VirtualFileSystem>,
}

impl SyncEngine {
    pub async fn new() -> Result<Self> {
        Self::with_peer_id(PeerId::random()).await
    }

    pub async fn with_peer_id(peer_id: PeerId) -> Result<Self> {
        // Use samod's built-in InMemoryStorage
        let storage = InMemoryStorage::new();

        // Build samod instance
        let samod = Samod::build_tokio()
            .with_storage(storage)
            .with_peer_id(peer_id)
            .load()
            .await;

        let samod = Arc::new(samod);

        // Create VFS layer on top of samod
        let vfs = Arc::new(VirtualFileSystem::new(samod.clone()).await?);

        Ok(Self { samod, vfs })
    }

    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        self.vfs.clone()
    }

    pub fn samod(&self) -> Arc<Samod> {
        self.samod.clone()
    }

    /// Connect to a WebSocket peer
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        let (ws_stream, _) = connect_async(url).await?;

        // Use samod's built-in WebSocket support
        let conn_finished = self.samod.connect_tungstenite(
            ws_stream,
            ConnDirection::Outgoing
        ).await;

        tracing::info!("WebSocket connection finished: {:?}", conn_finished);
        Ok(())
    }

    /// Find a document by its ID
    pub async fn find_document(&self, doc_id: DocumentId) -> Result<DocHandle> {
        Ok(self.samod.find(doc_id).await?)
    }
}
```

## Phase 6: VFS Implementation (Core Contribution)

This is the main new functionality - a virtual file system layer on top of samod.

### 6.1 Path Traversal (`src/vfs/traversal.rs`)

Port the path traversal logic from keepsync's `addressing.ts`:

```rust
use crate::vfs::types::*;
use crate::error::{Result, VfsError};
use samod::{Samod, DocumentId, DocHandle};
use std::sync::Arc;

pub struct TraverseResult {
    pub node_handle: DocHandle,
    pub node: DirNode,
    pub target_ref: Option<RefNode>,
    pub parent_path: String,
}

pub struct PathTraverser {
    samod: Arc<Samod>,
}

impl PathTraverser {
    pub fn new(samod: Arc<Samod>) -> Self {
        Self { samod }
    }

    pub async fn traverse(
        &self,
        root_id: DocumentId,
        path: &str,
        create_missing: bool,
    ) -> Result<TraverseResult> {
        // This is a direct port of keepsync's traverseDocTree function
        // Implementation details from addressing.ts lines 55-292
        todo!("Port traverseDocTree logic")
    }
}
```

### 6.2 VFS Operations (`src/vfs/operations.rs`)

```rust
use crate::vfs::types::*;
use crate::vfs::traversal::{PathTraverser, TraverseResult};
use crate::error::Result;
use samod::{Samod, DocumentId, DocHandle};
use std::sync::Arc;
use tokio::sync::broadcast;
use automerge::Automerge;

pub struct VirtualFileSystem {
    samod: Arc<Samod>,
    root_id: DocumentId,
    traverser: PathTraverser,
    event_tx: broadcast::Sender<VfsEvent>,
}

#[derive(Debug, Clone)]
pub enum VfsEvent {
    DocumentCreated { path: String, doc_id: DocumentId },
    DocumentUpdated { path: String, doc_id: DocumentId },
    DocumentDeleted { path: String },
}

impl VirtualFileSystem {
    pub async fn new(samod: Arc<Samod>) -> Result<Self> {
        // Create root document
        let root_doc = Automerge::new();
        let root_handle = samod.create(root_doc).await?;
        let root_id = root_handle.document_id().clone();

        // Initialize root as directory
        root_handle.with_document(|doc| {
            // Set up root directory structure
            // This needs to match keepsync's DirNode structure
        })?;

        let (event_tx, _) = broadcast::channel(100);
        let traverser = PathTraverser::new(samod.clone());

        Ok(Self {
            samod,
            root_id,
            traverser,
            event_tx,
        })
    }

    // Port createDocument from addressing.ts
    pub async fn create_document<T>(&self, path: &str, content: T) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        // Implementation from addressing.ts lines 328-392
        todo!("Port createDocument")
    }

    // Port findDocument from addressing.ts
    pub async fn find_document(&self, path: &str) -> Result<Option<DocHandle>> {
        // Implementation from addressing.ts lines 301-318
        todo!("Port findDocument")
    }

    // Port removeDocument from addressing.ts
    pub async fn remove_document(&self, path: &str) -> Result<bool> {
        // Implementation from addressing.ts lines 401-458
        todo!("Port removeDocument")
    }

    pub async fn list_directory(&self, path: &str) -> Result<Vec<RefNode>> {
        let result = self.traverser.traverse(
            self.root_id,
            path,
            false
        ).await?;

        Ok(result.node.children.clone())
    }
}
```

## Phase 7: High-Level API

### 7.1 Simple API (`src/lib.rs`)

```rust
pub use vfs::{VirtualFileSystem, VfsEvent, DirNode, DocNode, RefNode};
pub use sync::SyncEngine;
pub use error::{VfsError, Result};

/// Simplified API for common use cases
pub struct VFS {
    engine: SyncEngine,
}

impl VFS {
    /// Create a new VFS instance with in-memory storage
    pub async fn new() -> Result<Self> {
        let engine = SyncEngine::new().await?;
        Ok(Self { engine })
    }

    /// Connect to a WebSocket peer
    pub async fn connect(&self, url: &str) -> Result<()> {
        self.engine.connect_websocket(url).await
    }

    /// Get VFS handle for file operations
    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        self.engine.vfs()
    }

    /// Get the underlying samod instance for advanced operations
    pub fn samod(&self) -> Arc<Samod> {
        self.engine.samod()
    }

    /// Create a document at a path
    pub async fn create_file<T>(&self, path: &str, content: T) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.vfs().create_document(path, content).await
    }

    /// Read a document at a path
    pub async fn read_file(&self, path: &str) -> Result<Option<DocHandle>> {
        self.vfs().find_document(path).await
    }

    /// Delete a document at a path
    pub async fn delete_file(&self, path: &str) -> Result<bool> {
        self.vfs().remove_document(path).await
    }

    /// List files in a directory
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RefNode>> {
        self.vfs().list_directory(path).await
    }
}
```

## Phase 8: Examples

### 8.1 Basic VFS Example (`examples/basic_vfs.rs`)

```rust
use tonk_core::VFS;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Create VFS instance
    let sync = VFS::new().await?;

    // Create a document using VFS paths
    sync.create_file("/documents/example.txt", "Hello, VFS!").await?;
    println!("Created document at /documents/example.txt");

    // Read the document
    if let Some(handle) = sync.read_file("/documents/example.txt").await? {
        handle.with_document(|doc: &String| {
            println!("Document content: {}", doc);
        });
    }

    // List directory
    let files = sync.list_dir("/documents").await?;
    println!("\nFiles in /documents:");
    for file in files {
        println!("  - {} ({:?})", file.name, file.node_type);
    }

    Ok(())
}
```

### 8.2 WebSocket Sync Example (`examples/websocket_sync.rs`)

```rust
use tonk_core::VFS;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use samod::ConnDirection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Start a server in the background
    tokio::spawn(async {
        let server = VFS::new().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

        while let Ok((stream, _)) = listener.accept().await {
            let ws_stream = accept_async(stream).await.unwrap();

            // Use samod's WebSocket support
            server.samod().connect_tungstenite(
                ws_stream,
                ConnDirection::Incoming
            ).await;
        }
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create a client
    let client = VFS::new().await?;

    // Connect to server
    client.connect("ws://127.0.0.1:8080").await?;

    // Create documents on client - they'll sync automatically
    client.create_file("/shared/doc1.txt", "Document 1").await?;
    client.create_file("/shared/doc2.txt", "Document 2").await?;

    println!("Documents created and syncing...");

    // Keep running
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

### 8.3 Server with Axum (`examples/server.rs`)

```rust
#[cfg(feature = "axum")]
use axum::{
    extract::ws::WebSocketUpgrade,
    response::Response,
    routing::get,
    Router,
    Extension,
};
use tonk_core::VFS;
use std::sync::Arc;

async fn websocket_handler(
    ws: WebSocketUpgrade,
    Extension(vfs): Extension<Arc<VFS>>,
) -> Response {
    ws.on_upgrade(move |socket| async move {
        // Use samod's axum support
        vfs.samod().accept_axum(socket).await;
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let vfs = Arc::new(VFS::new().await?);

    // Create some initial documents
    vfs.create_file("/README.md", "# VFS Server").await?;
    vfs.create_file("/config.json", r#"{"version": "1.0"}"#).await?;

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .layer(Extension(vfs));

    println!("Server running on http://localhost:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
```

## Phase 9: Implementation Timeline

### Week 1: Core VFS Implementation

- Set up project structure
- Define VFS types matching keepsync's structure
- Port path traversal logic from keepsync's `addressing.ts`
- Implement basic CRUD operations

### Week 2: Samod Integration

- Integrate VFS with samod's document management
- Use samod's built-in WebSocket support
- Leverage samod's InMemoryStorage
- Test document synchronization

### Week 3: Complete Port

- Port remaining functions from keepsync
- Add directory operations
- Implement event system
- Handle edge cases and error conditions

### Week 4: Polish and Examples

- Create high-level API
- Write comprehensive examples
- Add tests
- Documentation

## Key Architecture Decisions

1. **Leverage Samod** - Use samod's existing WebSocket and storage instead of reimplementing
2. **Focus on VFS** - The virtual file system is the main new contribution
3. **Direct Port** - Port keepsync's addressing logic directly to maintain compatibility
4. **Minimal Dependencies** - Only add what's needed for the VFS layer

## Implementation Priority

1. **Critical Path**:
   - Port `traverseDocTree` from keepsync's `addressing.ts`
   - Port `createDocument`, `findDocument`, `removeDocument`
   - Integrate with samod's document handles

2. **Nice to Have**:
   - Event streaming for VFS changes
   - Batch operations
   - Performance optimizations

## Testing Strategy

### Unit Tests

- Test path traversal with various edge cases
- Test CRUD operations
- Test concurrent modifications
- Test directory operations

### Integration Tests

- Test with samod's WebSocket connections
- Test multi-client synchronization
- Test persistence with different storage backends
- Test compatibility with JavaScript keepsync

## Compatibility Notes

- Maintain wire compatibility with JavaScript automerge-repo
- Use same document structure as keepsync for interoperability
- Support same path semantics (Unix-style paths)
- Preserve timestamp and metadata formats

This implementation plan focuses on creating a VFS layer that works seamlessly with samod's existing
infrastructure, avoiding duplication of already-solved problems.
