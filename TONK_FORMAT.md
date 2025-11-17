# Tonk Format

## Abstract

The Tonk file format (`.tonk`) is a self-contained application bundle format that packages web-like
software with their complete application state into a single distributable file. Each Tonk contains
the necessary information to network with other Tonks like it, be they the same Tonk running on
different machines, or a different Tonk connected to the same underlying data via relay. A Tonk
represents a stark shift from traditional web applications: instead of deploying to servers and
managing infrastructure, applications become files that can be shared, copied, modified, and
executed anywhere a compatible runtime exists.

## Motivation

Modern web applications offer rich functionality but sacrifice portability and user agency. Users
cannot:

- Share a complete, working application as easily as sharing a document
- Own their application state independently from service providers
- Work offline without degraded functionality
- Collaborate on application state without centralised servers

The Tonk format addresses these limitations by reducing web applications to their essence: a file
that contains everything needed to run, persist state, and synchronise with others.

## Design Goals

### 1. Radical Portability

A `.tonk` file should run anywhere a Tonk runtime exists—browsers, Node.js, edge workers, mobile
devices—without modification or server infrastructure.

### 2. Local-First by Default

Applications should:

- Work completely offline
- Store state locally by default
- Sync peer-to-peer or through optional relay servers
- Never require cloud services for basic functionality

### 3. User Agency Through Simplicity

By packaging applications as files:

- Users can share apps via email, USB drives, messaging, or any file-sharing mechanism
- Forking an application to customise it is as simple as copying a file
- Application state belongs to the user, not a service provider
- No deployment, hosting, or infrastructure knowledge required

### 4. Self-Contained Execution

Each `.tonk` file contains:

- Complete application code and assets
- All user data and application state
- Synchronisation protocol endpoints (optional)
- No external dependencies except the runtime

### 5. Collaboration Without Centralisation

Using CRDTs (Conflict-free Replicated Data Types):

- Multiple users can modify the same application independently
- Changes merge automatically without conflicts
- Sync is optional and works peer-to-peer or through relays
- No central authority required for collaboration

## Specification

### File Format

#### Container Structure

A Tonk file is a ZIP archive with the `.tonk` extension:

```
bundle.tonk                          # Standard ZIP file
├── manifest.json                    # Bundle metadata (REQUIRED)
└── storage/                         # Document storage (REQUIRED)
    └── [XX]/                        # Directory splay (first 2 chars of doc ID)
        └── [document_id]/
            └── snapshot/
                └── bundle_export    # Automerge CRDT snapshot (binary)
```

ZIPs are well-suited for their universal support by OSes and readily available tools, as well as for
easy human inspection.

#### Manifest Schema

The `manifest.json` file contains bundle metadata:

```json
{
  "manifestVersion": 1,
  "version": {
    "major": 0,
    "minor": 1
  },
  "rootId": "AgFCjGexJYzfccJKH9YJcUmPjnr",
  "entrypoints": ["/app/index.html"],
  "networkUris": ["wss://relay.example.com"],
  "xNotes": "Optional human-readable description",
  "xVendor": {
    "xTonk": {
      "createdAt": "2025-11-14T12:00:00Z",
      "exportedFrom": "tonk-core@0.1.0"
    }
  }
}
```

Required Fields:

- `manifestVersion` (number): Manifest format version, currently `1`
- `version` (object): Tonk format version
  - `major` (number): Major version
  - `minor` (number): Minor version
- `rootId` (string): Document ID of the root PathIndex
- `entrypoints` (array of strings): Application entry points (e.g., `/app/index.html`)

Optional Fields:

- `networkUris` (array of strings): WebSocket relay URLs for synchronisation
- `xNotes` (string): Human-readable description or notes
- `xVendor` (object): Vendor-specific metadata (keys prefixed with `x`)

#### Storage Structure

Documents are stored using directory splaying for compatibility with Automerge file system storage
and for performance.

```
storage/Ag/FCjGexJYzfccJKH9YJcUmPjnr/snapshot/bundle_export
        ^^  ^^^^^^^^^^^^^^^^^^^^^^^^
        |   |
        |   +-- Remainder of document ID
        +------ First 2 characters (splay directory)
```

Each `bundle_export` file contains:

- Automerge CRDT document in binary format
- Complete document history compressed by Automerge (for merge operations)

### Network Configuration

The `networkUris` field in the manifest defines optional relay servers for synchronising application
state across multiple instances. When omitted entirely, the application will not connect to any
relays, disabling sync and collaboration. Network connectivity enhances collaboration but is never a
requirement for core functionality.

The array structure supports multiple relay URIs for redundancy, geographic distribution, and
failover. When connecting, runtimes attempt each URI in sequence until a successful connection is
established. If the first relay is unreachable—whether due to network issues, server downtime, or
firewall restrictions—the runtime automatically tries the next URI in the array. This continues
until either a connection succeeds or all options are exhausted.

Common patterns include distributing relays geographically for lower latency
(`wss://us.relay.example.com`, `wss://eu.relay.example.com`) or providing fallback infrastructure
for high availability.

Tonk provides an open source relay that anyone can operate to sync and collaborate over Tonks. This
relay acts purely as a message broker or "dumb pipe", forwarding CRDT synchronisation messages
between peers without understanding application semantics or intent.

Generally, the relay is infrastructure for real-time synchronisation, not a gatekeeper or data
custodian, though file system storage is also provided by the relay for data duplication on the host
machine if desired. Note that the open source relay currently has no security guarantees or
encryption. The relay can reproduce all data sent through it.

### Data Model

#### Virtual File System (VFS)

Tonk provides a hierarchical file system abstraction over Automerge CRDT documents. Applications
interact with familiar path-based operations:

```javascript
// Write application files
await tonk.writeFile('/src/App.tsx', sourceCode);
await tonk.writeFile('/data/users.json', userData);

// Read files
const content = await tonk.readFile('/data/users.json');

// List directories
const files = await tonk.listDir('/src');

// Watch for changes
const watcher = await tonk.watchFile('/data/users.json', content => {
  console.log('File updated:', content);
});
```

#### PathIndex

Unlike traditional hierarchical file systems, Tonk uses a flat `PathIndex` stored as a single
Automerge document:

```rust
pub struct PathIndex {
    paths: HashMap<String, PathEntry>,
    last_updated: DateTime<Utc>,
}

pub struct PathEntry {
    doc_id: String,       // Points to actual document
    node_type: NodeType,  // Document | Directory
    created: DateTime<Utc>,
    modified: DateTime<Utc>,
}
```

The flat structure enables:

- O(1) path lookups without tree traversal
- Atomic updates across the entire file system structure
- Efficient CRDT synchronisation
- Fast directory listings

Example PathIndex:

```json
{
  "/": { "doc_id": "doc_root", "node_type": "Directory", ... },
  "/app": { "doc_id": "doc_app", "node_type": "Directory", ... },
  "/app/index.html": { "doc_id": "doc_index", "node_type": "Document", ... },
  "/src/App.tsx": { "doc_id": "doc_app_tsx", "node_type": "Document", ... }
}
```

#### Document Types

Tonk uses a three-tier document architecture where each type serves a distinct purpose in the
virtual file system.

##### Directory Nodes

Directories are first-class Automerge documents that store references to their children rather than
embedding child content directly. This indirection keeps directory documents small, enables atomic
file operations without updating parent directories, and allows independent synchronisation of
directory structures across peers.

Timestamps provide familiar file system semantics and enable deterministic last-write-wins conflict
resolution when multiple peers modify the same directory simultaneously.

```rust
pub struct DirNode {
    node_type: "directory",
    name: String,
    timestamps: Timestamps,
    children: Vec<RefNode>,  // References to child documents
}
```

##### Document Nodes

Document nodes contain actual file content through a hybrid model supporting both structured and
binary data. The `content` field holds JSON-serialisable data that receives full CRDT merge
semantics—when two peers modify different fields of an object, changes merge cleanly. The optional
`bytes` field stores binary assets (images, videos, etc.) using last-write-wins semantics, enabling
fully self-contained applications without external blob storage.

```rust
pub struct DocNode<T> {
    node_type: "document",
    name: String,
    timestamps: Timestamps,
    content: T,              // JSON-serialisable application data
    bytes: Option<Vec<u8>>,  // Optional binary data (images, etc.)
}
```

##### Reference Nodes

Reference nodes are lightweight structures stored in parent directories that enable efficient
directory listings without loading full child documents. Each contains the minimum metadata needed
for display: a stable document ID pointer, node type, timestamps, and name.

```rust
pub struct RefNode {
    pointer: DocumentId,     // Points to actual document
    node_type: NodeType,
    timestamps: Timestamps,
    name: String,
}
```

### Runtime Architecture

#### Execution Model

```
┌────────────────────────────────────────────────┐
│       Host Environment (Browser/Node.js)       │
│       ┌────────────────────────────────┐       │
│       │ User Application (HTML/JS/CSS) │       │
│       │ - Uses VFS API for persistence │       │
│       │ - Oblivious to CRDT internals  │       │
│       └───────────────┬────────────────┘       │
│                       │ VFS API                │
├───────────────────────┼────────────────────────┤
│  ┌────────────────────▼─────────────────────┐  │
│  │          TonkCore (WASM Module)          │  │
│  │     ┌───────────┐   ┌─────────────────┐  │  │
│  │     │    VFS    │   │      Samod      │  │  │
│  │     │(PathIndex)│   │   (CRDT repo)   │  │  │
│  │     └───────────┘   └─────────────────┘  │  │
│  └────────────────────┬─────────────────────┘  │
│                       │ Storage API            │
├───────────────────────┼────────────────────────┤
│      ┌────────────────▼──────────────────┐     │
│      │Storage Backend                    │     │
│      │- IndexedDB (browser, persistent)  │     │
│      │- Filesystem (Node.js, persistent) │     │
│      │- InMemory (ephemeral)             │     │
│      └───────────────────────────────────┘     │
└────────────────────────────────────────────────┘
```

Minimal Runtime Requirements:

- WebAssembly execution environment
- Storage backend (IndexedDB, filesystem, or in-memory)
- Optional: WebSocket support for synchronisation

At the moment only Chrome, Safari, Edge and other Chromium/WebKit browsers are supported. The
browser runtime requires service worker features not supported in Firefox at the time of writing.
