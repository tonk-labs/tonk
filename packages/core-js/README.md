# @tonk/core

Browser/Node.js WASM bindings for Tonk Core - a local-first sync engine with virtual file system and
bundle support.

## Installation

```bash
npm install @tonk/core
```

## Features

- **TonkCore**: Main synchronization engine
- **Virtual File System**: CRDT-based file system with hierarchical document management
- **Bundle Operations**: ZIP-based data export/import
- **WebSocket Sync**: Real-time peer-to-peer synchronization

## Quick Start

```typescript
import { createTonk, TonkCore, VirtualFileSystem } from '@tonk/core';

// Create a Tonk Core
const tonk = await createTonk();

// Get the peer ID
const peerId = await tonk.getPeerId();
console.log('Peer ID:', peerId);

// Get the virtual file system
const vfs = await tonk.getVfs();

// Create a file
await vfs.createFile('/hello.txt', 'Hello, World!');

// Read the file
const content = await vfs.readFile('/hello.txt');
console.log('File content:', content);
```

## API Reference

### Initialization

The WASM module is automatically initialized when you import from `@tonk/core`. For custom
configuration:

#### `initializeTonk(config?: TonkConfig): Promise<void>`

```typescript
import { initializeTonk } from '@tonk/core';

await initializeTonk({
  wasmPath: '/path/to/tonk_core_bg.wasm', // Optional: custom WASM path
});
```

#### `isInitialized(): boolean`

Check if the WASM module has been initialized.

### TonkCore

The main synchronization engine that manages VFS interactions.

#### Factory Functions

```typescript
// Create with auto-generated peer ID
const tonk = await createTonk();

// Create with specific peer ID
const tonk = await createTonkWithPeerId('my-peer-id');

// Create from existing bundle
const tonk = await createTonkFromBundle(bundle);

// Create from bundle bytes
const tonk = await createTonkFromBytes(bundleData);
```

#### TonkCore Class

```typescript
class TonkCore {
  // Get the peer ID
  getPeerId(): Promise<string>;

  // Connect to a WebSocket server
  connectWebsocket(url: string): Promise<void>;

  // Get the virtual file system
  getVfs(): Promise<VirtualFileSystem>;

  // Get the repository (low-level CRDT access)
  getRepo(): Promise<Repository>;

  // Export to bundle bytes
  toBytes(): Promise<Uint8Array>;

  // Free WASM memory (call when done)
  free(): void;
}
```

### Virtual File System

CRDT-based file system with automatic synchronization between peers.

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
  getMetadata(path: string): Promise<NodeMetadata | null>;
}
```

### Bundle Operations

ZIP-based bundles for data portability with manifest-based metadata.

#### Creating Bundles

```typescript
// Create bundle from existing data
const bundle = await createBundleFromBytes(data);
```

#### Bundle Class

```typescript
class Bundle {
  // Get the root document ID
  getRootId(): Promise<string>;

  // Retrieve a value by key
  get(key: string): Promise<Uint8Array | null>;

  // Get all entries with matching prefix
  getPrefix(prefix: string): Promise<BundleEntry[]>;

  // List all keys in the bundle
  listKeys(): Promise<string[]>;

  // Get the bundle manifest
  getManifest(): Promise<Manifest>;

  // Serialize bundle to bytes
  toBytes(): Promise<Uint8Array>;

  // Free WASM memory (call when done)
  free(): void;
}
```

### Types

```typescript
interface DirectoryEntry {
  name: string;
  type: 'directory' | 'document';
}

interface NodeMetadata {
  nodeType: 'directory' | 'document';
  createdAt: Date;
  modifiedAt: Date;
}

interface BundleEntry {
  key: string;
  value: Uint8Array;
}

interface Manifest {
  manifestVersion: number;
  version: Version;
  rootId: string;
  entrypoints: string[];
  networkUris: string[];
  xNotes?: string;
  xVendor?: object;
}
```

## Examples

### Real-time Collaboration

```typescript
import { createTonk } from '@tonk/core';

async function setupCollaboration() {
  const tonk = await createTonk();
  const vfs = await tonk.getVfs();

  // Connect to sync server
  await tonk.connectWebsocket('ws://localhost:8080');

  // Create and sync files
  await vfs.createFile('/shared/document.md', '# Collaborative Document');
  await vfs.createDirectory('/team');
  await vfs.createFile('/team/notes.txt', 'Team meeting notes');

  // Files are automatically synchronized with other connected peers
}
```

### Working with Bundles

```typescript
import { createTonk, createBundleFromBytes } from '@tonk/core';

async function bundleExample() {
  // Create and populate a Tonk instance
  const tonk1 = await createTonk();
  const vfs1 = await tonk1.getVfs();

  await vfs1.createFile('/config.json', '{"version": 1}');
  await vfs1.createFile('/data.txt', 'Some important data');

  // Export to bundle
  const bundleData = await tonk1.toBytes();

  // Import into new instance
  const tonk2 = await createTonkFromBytes(bundleData);
  const vfs2 = await tonk2.getVfs();

  // Verify data was preserved
  const exists = await vfs2.exists('/config.json');
  console.log('Data preserved:', exists);

  // Working with bundle directly
  const bundle = await createBundleFromBytes(bundleData);
  const manifest = await bundle.getManifest();
  const keys = await bundle.listKeys();

  console.log('Bundle manifest:', manifest);
  console.log('Available keys:', keys);
}
```

### Directory Operations

```typescript
import { createTonk } from '@tonk/core';

async function directoryExample() {
  const tonk = await createTonk();
  const vfs = await tonk.getVfs();

  // Create directory structure
  await vfs.createDirectory('/projects');
  await vfs.createDirectory('/projects/webapp');
  await vfs.createFile('/projects/webapp/index.html', '<html>...</html>');
  await vfs.createFile('/projects/webapp/style.css', 'body { margin: 0; }');

  // List contents
  const entries = await vfs.listDirectory('/projects/webapp');
  for (const entry of entries) {
    console.log(`${entry.type}: ${entry.name}`);
  }

  // Get metadata
  const metadata = await vfs.getMetadata('/projects/webapp/index.html');
  if (metadata) {
    console.log('Created:', metadata.createdAt);
    console.log('Modified:', metadata.modifiedAt);
  }
}
```

### Error Handling

```typescript
import { createTonk, FileSystemError, BundleError } from '@tonk/core';

async function errorHandling() {
  try {
    const tonk = await createTonk();
    const vfs = await tonk.getVfs();

    await vfs.createFile('/test.txt', 'content');
  } catch (error) {
    if (error instanceof FileSystemError) {
      console.error('File system error:', error.message);
    } else if (error instanceof BundleError) {
      console.error('Bundle error:', error.message);
    } else {
      console.error('Unexpected error:', error);
    }
  }
}
```

## Memory Management

Call `free()` on TonkCore and Bundle instances when done to prevent memory leaks:

```typescript
const tonk = await createTonk();
const bundle = await createBundleFromBytes(data);

// Use tonk and bundle...

// Clean up
tonk.free();
bundle.free();
```

## Browser Compatibility

This package requires a modern browser with WebAssembly support:

- Chrome/Edge 57+
- Firefox 52+
- Safari 11+

## Node.js Compatibility

Requires Node.js 14+ with WebAssembly support.

## License

MIT
