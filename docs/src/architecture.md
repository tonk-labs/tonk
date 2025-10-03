# Tonk Architecture

Tonk is built on a local-first, peer-to-peer architecture that enables real-time collaboration
without centralized servers. This document provides a detailed overview of Tonk's core components
and how they work together.

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     JavaScript/TypeScript Layer              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   Core-JS    │  │   Keepsync   │  │   Application    │  │
│  │   Wrapper    │  │  Middleware  │  │     Layer        │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    WASM Core (Rust)                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   TonkCore   │  │     VFS      │  │     Bundle       │  │
│  │              │  │              │  │     System       │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   Automerge  │  │   WebSocket  │  │    Storage       │  │
│  │     CRDT     │  │     Sync     │  │    Backend       │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. TonkCore (Rust/WASM)

The heart of Tonk - a Rust library compiled to WebAssembly that provides the foundation for all Tonk
operations.

**Key Responsibilities:**

- Managing the Automerge document store
- Coordinating VFS operations
- Handling peer connections
- Bundle import/export
- Storage persistence

**Implementation Details:**

```rust
pub struct TonkCore {
    repo: Arc<Mutex<Repo>>,
    vfs: Arc<VirtualFileSystem>,
    connections: HashMap<String, Connection>,
    storage: Box<dyn StorageBackend>,
}
```

### 2. Virtual File System (VFS)

A document-based file system abstraction that provides familiar file operations while leveraging
Automerge for synchronization.

**Features:**

- Hierarchical directory structure
- File read/write operations
- Directory listing and creation
- File watching for reactive updates
- Support for text and binary data

**Document Structure:** Each file or directory is represented as an Automerge document with:

- Metadata (name, type, timestamps)
- Content (text or binary data)
- References to child documents (for directories)

### 3. Bundle System

A packaging format for distributing Tonk applications as self-contained units.

**Bundle Structure:**

```
tonk-app.tonk (ZIP archive)
├── manifest.json           # Metadata and configuration
├── storage/               # Serialized Automerge documents
│   ├── root.automerge
│   └── [document-id].automerge
```

**Manifest Format:**

This format hasn't been fully integrated yet.

```json
{
  "manifest_version": 1,
  "version": { "major": 1, "minor": 0 },
  "root_id": "automerge-document-id",
  "entrypoints": ["app_folder"],
  "network_uris": ["wss://sync.example.com"]
}
```

### 5. Automerge

Tonk very intentionally does not want to veer too far away from Automerge. This is in order to
encourage maximum interoperability and leave open optionality for a broader standard. If you are
interested in certain low-level features about the protocol and about how network and storage
adapters work, please see the [Automerge](https://automerge.org/) site.
