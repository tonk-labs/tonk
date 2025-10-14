# Rust Relay Port Plan

## Architecture Overview

The TypeScript relay is a WebSocket-based automerge sync server with the following components:

**Core Components:**

1. **WebSocket Server** - Handles peer connections using `NodeWSServerAdapter` (automerge-repo
   protocol)
2. **Storage Layer** - Uses `NodeFSStorageAdapter` for automerge document persistence
3. **Bundle Management** - `BundleStorageAdapter` for reading/serving `.tonk` bundles
4. **HTTP API** - Express server with CORS for manifest/bundle endpoints
5. **S3 Integration** - Optional S3 storage for bundle uploads/downloads

**Key Features:**

- Automerge sync protocol over WebSocket (CBOR-encoded messages)
- Bundle manifest serving (slim bundles with just manifest.json + root doc)
- S3 bundle storage/retrieval
- Connection tracking with keep-alive pings
- Metrics endpoint

---

## Rust Port Implementation Plan

### Phase 1: Project Setup & Dependencies

**Create new Rust crate:** `packages/relay-rust/`

**Cargo.toml dependencies:**

```toml
[package]
name = "tonk-relay"
version = "0.1.0"
edition = "2021"

[dependencies]
# Core sync engine (already compatible)
samod = { git = "https://github.com/tonk-labs/samod", branch = "wasm-runtime", features = ["tokio", "tungstenite", "threadpool"] }
automerge = "0.7.0"

# Async runtime
tokio = { version = "1.47", features = ["full"] }
tokio-stream = "0.1"
futures = "0.3"

# WebSocket server
tokio-tungstenite = "0.27"

# HTTP server
axum = { version = "0.8", features = ["ws", "multipart"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "fs", "trace"] }

# Serialization (CBOR for automerge protocol)
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
serde_cbor = "0.11"  # Wire-compatible with TypeScript cbor-x

# Bundle/ZIP handling
zip = { version = "5.0", features = ["deflate"] }

# S3 integration
aws-sdk-s3 = "1.0"
aws-config = "1.0"

# Error handling
anyhow = "1.0"
thiserror = "2.0"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Utilities
bytes = "1"
uuid = { version = "1.0", features = ["serde", "v4"] }
```

---

### Phase 2: Storage Adapters

#### 2.1 Filesystem Storage

**File:** `src/storage/filesystem.rs`

- Use existing `samod::storage::filesystem::FilesystemStorage` (already wire-compatible!)
- Implements samod `Storage` trait
- Path format: `{base_dir}/{first_2_chars}/{remaining_chars}/{...rest}`
- Already compatible with automerge-repo-storage-nodefs format

**Implementation notes:**

- The samod filesystem storage is already 100% compatible
- Just wrap it with any relay-specific configuration needs

#### 2.2 Bundle Storage Adapter

**File:** `src/storage/bundle.rs`

Port `bundleStorageAdapter.ts` functionality:

```rust
pub struct BundleStorageAdapter {
    bundle_zip: Arc<RwLock<ZipArchive<Cursor<Vec<u8>>>>>,
    memory_data: Arc<RwLock<HashMap<StorageKey, Vec<u8>>>>,
    manifest: Arc<RwLock<Manifest>>,
}

impl BundleStorageAdapter {
    pub async fn from_bundle(bundle_bytes: Vec<u8>) -> Result<Self>
    pub async fn create_slim_bundle(&self) -> Result<Vec<u8>>

    // Map bundle paths: storage/{aa}/{bb...}/snapshot/bundle_export
    // To automerge keys: {aabb...}/snapshot
    fn map_bundle_path_to_automerge_key(&self, path: &str) -> Option<StorageKey>
}

impl Storage for BundleStorageAdapter {
    async fn load(&self, key: StorageKey) -> Option<Vec<u8>>
    async fn put(&self, key: StorageKey, data: Vec<u8>)
    async fn load_range(&self, prefix: StorageKey) -> HashMap<StorageKey, Vec<u8>>
    async fn delete(&self, key: StorageKey)
}
```

**Use tonk-core's Bundle type:**

- Already have `Bundle<R: RandomAccess>` in `packages/core/src/bundle/bundle.rs`
- Already parses manifest, validates version, handles ZIP operations
- Reuse for read operations, wrap for Storage trait compatibility

---

### Phase 3: WebSocket Server Adapter

**Use samod's built-in WebSocket adapter** instead of building custom bridge:

- samod already has `connect_tungstenite()` for WebSocket handling
- The protocol is already implemented and wire-compatible
- Less code to maintain, better integration

**File:** `src/network/websocket_server.rs`

```rust
pub struct WebSocketServerAdapter {
    repo: Arc<Repo>,
    listener: TcpListener,
}

impl WebSocketServerAdapter {
    pub async fn new(repo: Arc<Repo>, addr: SocketAddr) -> Result<Self>

    pub async fn run(self) -> Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let repo = self.repo.clone();

            tokio::spawn(async move {
                if let Ok(ws_stream) = tokio_tungstenite::accept_async(stream).await {
                    // Use samod's built-in tungstenite adapter
                    repo.connect_tungstenite(ws_stream, ConnDirection::Incoming).await;
                }
            });
        }
    }
}
```

---

### Phase 4: HTTP Server (Axum)

**File:** `src/server.rs`

```rust
pub struct RelayServer {
    repo: Arc<Repo>,
    bundle_storage: Arc<BundleStorageAdapter>,
    s3_storage: Option<Arc<S3Storage>>,
    connection_count: Arc<AtomicUsize>,
}

impl RelayServer {
    pub async fn create(
        port: u16,
        bundle_path: PathBuf,
        storage_dir: PathBuf,
    ) -> Result<Self>

    pub async fn run(self) -> Result<()>
}
```

**HTTP Routes (match TypeScript endpoints):**

```rust
async fn routes(state: Arc<RelayServer>) -> Router {
    Router::new()
        .route("/", get(health_check))
        .route("/tonk_core_bg.wasm", get(serve_wasm))
        .route("/.manifest.tonk", get(serve_manifest))
        .route("/api/bundles", post(upload_bundle))
        .route("/api/bundles/:id", get(download_bundle))
        .route("/api/bundles/:id/manifest", get(download_bundle_manifest))
        .route("/api/blank-tonk", get(serve_blank_tonk))
        .route("/metrics", get(metrics))
        .layer(CorsLayer::very_permissive())
}

// WebSocket upgrade handled by samod directly on separate port
```

**Endpoint implementations:**

1. **GET /** → `"👍 Tonk relay server is running"`

2. **GET /tonk_core_bg.wasm** → Serve WASM file from `../../core-js/dist/`

3. **GET /.manifest.tonk** → `bundle_storage.create_slim_bundle()`
   - Returns ZIP with manifest.json + storage/{rootId[0:2]}/\* files

4. **POST /api/bundles** → Upload bundle to S3
   - Parse bundle, extract rootId from manifest
   - `s3_storage.upload_bundle(bundle_id, data)`

5. **GET /api/bundles/:id** → Download full bundle from S3

6. **GET /api/bundles/:id/manifest** → Download slim bundle from S3
   - Fetch full bundle, create slim version with just manifest + root doc

7. **GET /api/blank-tonk** → Serve `latergram.tonk` template file

8. **GET /metrics** → JSON with memory, connections, uptime
   ```rust
   {
       "timestamp": SystemTime::now(),
       "memory": {
           "rss": get_memory_usage(),
       },
       "connections": connection_count.load(Ordering::Relaxed),
       "uptime": start_time.elapsed().as_secs(),
   }
   ```

---

### Phase 5: S3 Storage

**File:** `src/storage/s3.rs`

```rust
pub struct S3Storage {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl S3Storage {
    pub async fn new(bucket: String, region: String) -> Result<Self>

    pub async fn upload_bundle(&self, bundle_id: &str, data: Vec<u8>) -> Result<()>

    pub async fn download_bundle(&self, bundle_id: &str) -> Result<Vec<u8>>

    pub async fn bundle_exists(&self, bundle_id: &str) -> Result<bool>

    pub async fn health_check(&self) -> bool
}
```

**S3 key format:** `bundles/{bundle_id}.tonk`

**Metadata:**

- `Content-Type: application/octet-stream`
- `uploadedAt: timestamp`

---

### Phase 6: Wire Protocol Compatibility

#### Storage Key Format

Both TS and Rust use same format:

- Array of strings: `["docId", "snapshot"]`
- File path: `{baseDir}/{docId[0:2]}/{docId[2:]}/snapshot`
- Example: `storage/4C/SBjwxmWCro5MfDrDUBtjJmZRYU/snapshot`

#### Samod Protocol

- Samod already handles wire protocol internally
- No custom CBOR message types needed on our side
- Just pass WebSocket connections to samod's `connect_tungstenite()`

---

### Phase 7: Project Structure

```
packages/relay-rust/
├── Cargo.toml
├── src/
│   ├── main.rs                    # Entry point, CLI args
│   ├── server.rs                  # RelayServer struct, HTTP routes
│   ├── network/
│   │   ├── mod.rs
│   │   └── websocket_server.rs    # WebSocket handler using samod
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── filesystem.rs          # Re-export samod filesystem storage
│   │   ├── bundle.rs              # BundleStorageAdapter
│   │   └── s3.rs                  # S3Storage
│   └── error.rs                   # Error types
├── .env.example
└── README.md
```

---

## Benefits of Rust Port

1. **Performance** → Lower memory usage, faster processing, native async
2. **Type Safety** → Compile-time guarantees
3. **Single Codebase** → Unify with tonk-core (already Rust)
4. **Better Concurrency** → Tokio's work-stealing scheduler
5. **Wire Compatible** → Existing TS clients work without changes
6. **Smaller Binary** → No Node.js runtime needed
7. **Less Code** → Leveraging samod's existing WebSocket adapter

---

## Development Checklist

- [ ] Phase 1: Set up Cargo project with dependencies
- [ ] Phase 2: Implement storage adapters (filesystem, bundle, S3)
- [ ] Phase 3: Implement WebSocket server using samod adapter
- [ ] Phase 4: Implement HTTP server with all endpoints
- [ ] Phase 5: Add S3 integration
- [ ] Phase 6: Wire compatibility verification
- [ ] Phase 7: Integration test with TypeScript client
- [ ] Phase 8: Cross-language storage compatibility tests
- [ ] Phase 9: Performance benchmarks
- [ ] Phase 10: Documentation and deployment guide

---

## Implementation Timeline Estimate

- **Phase 1-2 (Storage):** 2-3 days
- **Phase 3 (WebSocket with samod):** 1-2 days
- **Phase 4 (HTTP Server):** 2-3 days
- **Phase 5 (S3):** 1-2 days
- **Phase 6-8 (Testing):** 3-4 days
- **Phase 9-10 (Deploy/Docs):** 1-2 days

**Total:** ~2-3 weeks for full port with comprehensive testing
