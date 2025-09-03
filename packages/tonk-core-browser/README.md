# @tonk/core-browser-wasm

Browser WASM bindings for Tonk Core - a local-first sync engine with virtual file system and bundle
support.

## Installation

```bash
npm install @tonk/core-browser-wasm
```

## Features

- **Sync Engine**: Real-time synchronization with WebSocket support
- **Virtual File System**: In-memory file system with Automerge-based CRDT synchronization
- **Bundle Operations**: Create and manage bundled content
- **TypeScript Support**: Full type definitions included

## Quick Start

```typescript
import { createSyncEngine, SyncEngine, VirtualFileSystem } from '@tonk/core-browser-wasm';

// Create a sync engine
const engine = await createSyncEngine();

// Get the peer ID
const peerId = await engine.getPeerId();
console.log('Peer ID:', peerId);

// Get the virtual file system
const vfs = await engine.getVfs();

// Create a file
await vfs.createFile('/hello.txt', 'Hello, World!');

// Read the file
const content = await vfs.readFile('/hello.txt');
console.log('File content:', content);
```

## API Reference

### Initialization

#### `initializeTonk(config?: TonkConfig): Promise<void>`

Initialize the WASM module with optional configuration.

```typescript
await initializeTonk({
  wasmPath: '/path/to/tonk_core_bg.wasm', // Optional: custom WASM path
});
```

### Sync Engine

#### `createSyncEngine(): Promise<SyncEngine>`

Create a new sync engine instance with a random peer ID.

#### `createSyncEngineWithPeerId(peerId: string): Promise<SyncEngine>`

Create a new sync engine instance with a specific peer ID.

#### `SyncEngine`

```typescript
class SyncEngine {
  // Get the peer ID
  getPeerId(): Promise<string>;

  // Connect to a WebSocket server
  connectWebsocket(url: string): Promise<void>;

  // Connect and adopt server's root document
  connectWebsocketWithServerRoot(url: string): Promise<void>;

  // Get the virtual file system
  getVfs(): Promise<VirtualFileSystem>;

  // Get the repository
  getRepo(): Promise<Repository>;
}
```

### Virtual File System

#### `VirtualFileSystem`

```typescript
class VirtualFileSystem {
  // Create a file with content
  createFile(path: string, content: string | Uint8Array): Promise<void>;

  // Read a file
  readFile(path: string): Promise<string>;

  // Delete a file
  deleteFile(path: string): Promise<boolean>;

  // Check if a path exists
  exists(path: string): Promise<boolean>;

  // Create a directory
  createDirectory(path: string): Promise<void>;

  // List directory contents
  listDirectory(path: string): Promise<DirectoryEntry[]>;

  // Get file/directory metadata
  getMetadata(path: string): Promise<FileMetadata>;
}
```

### Bundle Operations

#### `createBundle(): Bundle`

Create a new empty bundle.

#### `createBundleFromBytes(data: Uint8Array): Bundle`

Create a bundle from existing data.

#### `Bundle`

```typescript
class Bundle {
  // Store a value in the bundle
  put(key: string, value: Uint8Array): Promise<void>;

  // Retrieve a value from the bundle
  get(key: string): Promise<Uint8Array | null>;

  // Delete a value from the bundle
  delete(key: string): Promise<void>;

  // Get all values with a specific prefix
  getPrefix(prefix: string): Promise<Record<string, Uint8Array>>;

  // List all keys in the bundle
  listKeys(): Promise<string[]>;

  // Serialize bundle to bytes
  toBytes(): Promise<Uint8Array>;
}
```

## Examples

### Real-time Collaboration

```typescript
import { initializeTonk, createSyncEngine } from '@tonk/core-browser-wasm';

async function setupCollaboration() {
  await initializeTonk();

  const engine = await createSyncEngine();
  const vfs = await engine.getVfs();

  // Connect to sync server
  await engine.connectWebsocketWithServerRoot('ws://localhost:8080');

  // Create and sync files
  await vfs.createFile('/shared/document.md', '# Collaborative Document');

  // Files are automatically synchronized with other connected peers
}
```

### Working with Bundles

```typescript
import { initializeTonk, createBundle } from '@tonk/core-browser-wasm';

async function bundleExample() {
  await initializeTonk();

  const bundle = createBundle();

  // Store JSON data
  const data = { name: 'Example', version: 1 };
  const encoder = new TextEncoder();
  await bundle.put('config.json', encoder.encode(JSON.stringify(data)));

  // Retrieve data
  const stored = await bundle.get('config.json');
  if (stored) {
    const decoder = new TextDecoder();
    const json = JSON.parse(decoder.decode(stored));
    console.log('Stored config:', json);
  }

  // Export bundle
  const bytes = await bundle.toBytes();
  // Save or transmit bytes...
}
```

## Browser Compatibility

This package requires a modern browser with WebAssembly support:

- Chrome/Edge 57+
- Firefox 52+
- Safari 11+

## License

MIT
