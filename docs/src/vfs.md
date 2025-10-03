# Virtual File System (VFS)

The Virtual File System is a core abstraction in Tonk that provides a familiar file system interface
while leveraging Automerge CRDTs for distributed, conflict-free synchronization.

## Overview

The VFS allows you to:

- Work with files and directories using familiar APIs
- Automatically sync changes across connected peers
- Watch for real-time updates
- Store both JSON data and binary content
- Maintain offline-first functionality

All VFS methods are available directly on the `TonkCore` instance.

## File Operations

### Creating Files

```typescript
// Create a file with JSON content
await tonk.createFile('/app/config.json', {
  theme: 'dark',
  fontSize: 16,
});

// Create a file with string content
await tonk.createFile('/app/notes.txt', 'Hello, World!');

// Create a file with array content
await tonk.createFile('/data/items.json', [1, 2, 3, 4, 5]);
```

### Creating Files with Binary Data

```typescript
// Create an image file with metadata and bytes
await tonk.createFileWithBytes(
  '/images/tree.png',
  { mime: 'image/png', alt: 'picture of a tree' },
  imageBytes // Uint8Array or base64 string
);
```

### Reading Files

```typescript
// Read a file
const doc = await tonk.readFile('/app/config.json');
console.log(doc.content); // JSON content
console.log(doc.name); // 'config.json'
console.log(doc.type); // 'document'
console.log(doc.timestamps.created); // timestamp
console.log(doc.timestamps.modified); // timestamp

// If file has binary data
if (doc.bytes) {
  console.log(doc.bytes); // base64-encoded binary data
}
```

### Updating Files

```typescript
// Update file content
await tonk.updateFile('/app/config.json', {
  theme: 'light',
  fontSize: 18,
});

// Update file with binary data
await tonk.updateFileWithBytes(
  '/images/tree.png',
  { mime: 'image/png', alt: 'updated picture' },
  newImageBytes
);

// Returns false if file doesn't exist
const updated = await tonk.updateFile('/nonexistent.txt', 'content');
console.log(updated); // false
```

### Deleting Files

```typescript
// Delete a file
const deleted = await tonk.deleteFile('/temp/cache.txt');
console.log(deleted); // true if deleted, false if didn't exist

// Check before deleting
if (await tonk.exists('/temp/cache.txt')) {
  await tonk.deleteFile('/temp/cache.txt');
}
```

### Checking Existence

```typescript
// Check if file or directory exists
const exists = await tonk.exists('/app/config.json');
if (!exists) {
  await tonk.createFile('/app/config.json', {});
}
```

### Renaming Files

```typescript
// Rename a file
await tonk.rename('/old-name.txt', '/new-name.txt');

// Rename a directory
await tonk.rename('/old-folder', '/new-folder');

// Returns false if source doesn't exist
const renamed = await tonk.rename('/nonexistent.txt', '/new.txt');
console.log(renamed); // false
```

## Directory Operations

### Creating Directories

```typescript
// Create a single directory
await tonk.createDirectory('/app');

// Create nested directory structure
await tonk.createDirectory('/app');
await tonk.createDirectory('/app/components');
await tonk.createDirectory('/app/components/ui');
```

### Listing Directory Contents

```typescript
// List directory contents
const entries = await tonk.listDirectory('/app');

// Process entries
for (const entry of entries) {
  console.log(`Name: ${entry.name}`);
  console.log(`Type: ${entry.type}`); // "document" or "directory"
  console.log(`Pointer: ${entry.pointer}`); // Automerge document ID
  console.log(`Created: ${entry.timestamps.created}`);
  console.log(`Modified: ${entry.timestamps.modified}`);
}

// Filter by type
const files = entries.filter(e => e.type === 'document');
const dirs = entries.filter(e => e.type === 'directory');
```

### Getting Metadata

```typescript
// Get metadata for a file or directory
const metadata = await tonk.getMetadata('/app/config.json');
console.log(`Type: ${metadata.type}`);
console.log(`Created: ${metadata.timestamps.created}`);
console.log(`Modified: ${metadata.timestamps.modified}`);
console.log(`Pointer: ${metadata.pointer}`);
```

## File Watching

### Watching Files

```typescript
// Watch a file for changes
const watcher = await tonk.watchFile('/app/config.json', doc => {
  console.log('File changed:', doc.content);
  console.log('Modified at:', doc.timestamps.modified);
});

// Get the document ID being watched
console.log(watcher.documentId());

// Stop watching when done
await watcher.stop();
```

### Watching Directories

```typescript
// Watch a directory for changes (direct descendants only)
const dirWatcher = await tonk.watchDirectory('/app/data', dirNode => {
  console.log('Directory changed:', dirNode.name);
  console.log('Children:', dirNode.children);

  // Check timestamps to see what changed
  if (dirNode.children) {
    for (const child of dirNode.children) {
      console.log(`${child.name} modified: ${child.timestamps.modified}`);
    }
  }
});

// Stop watching
await dirWatcher.stop();
```

### Watch Patterns

```typescript
// React component example
import { useEffect, useState } from 'react';

function ConfigViewer({ tonk }) {
  const [config, setConfig] = useState(null);

  useEffect(() => {
    let watcher: DocumentWatcher | null = null;

    tonk.watchFile('/config.json', (doc) => {
      setConfig(doc.content);
    }).then(w => watcher = w);

    return () => {
      if (watcher) {
        watcher.stop();
      }
    };
  }, [tonk]);

  return <div>{JSON.stringify(config)}</div>;
}

// Multi-file watcher class
class MultiFileWatcher {
  private watchers: Map<string, DocumentWatcher> = new Map();

  async watchFiles(tonk: TonkCore, paths: string[]) {
    for (const path of paths) {
      const watcher = await tonk.watchFile(path, (doc) => {
        this.handleFileChange(path, doc);
      });
      this.watchers.set(path, watcher);
    }
  }

  async cleanup() {
    for (const [path, watcher] of this.watchers) {
      await watcher.stop();
    }
    this.watchers.clear();
  }

  private handleFileChange(path: string, doc: DocumentData) {
    console.log(`File ${path} changed:`, doc.content);
  }
}
```

## Path Resolution

### Path Rules

1. **Absolute Paths Required**: All paths must start with `/`
2. **No Trailing Slashes**: Paths should not end with `/` (except root)
3. **Case Sensitive**: `/App` and `/app` are different
4. **Forward Slashes Only**: Use `/` even on Windows

### Valid Path Examples

```typescript
// ✅ Valid paths
'/app/config.json';
'/data/users/profile.json';
'/assets/images/logo.png';
'/'; // Root directory

// ❌ Invalid paths
'app/config.json'; // No leading slash
'/app/config.json/'; // Trailing slash
'\\app\\config.json'; // Wrong slash type
'./app/config.json'; // Relative path
```

## Binary File Support

### Writing Binary Files

```typescript
// Browser: Convert file to bytes
async function uploadImage(file: File, tonk: TonkCore) {
  const arrayBuffer = await file.arrayBuffer();
  const bytes = new Uint8Array(arrayBuffer);

  await tonk.createFileWithBytes(
    `/images/${file.name}`,
    {
      mime: file.type,
      size: file.size,
      uploadedAt: Date.now(),
    },
    bytes
  );
}

// Node.js: Read file as bytes
import { readFile } from 'fs/promises';

const imageBytes = await readFile('./tree.png');
await tonk.createFileWithBytes('/images/tree.png', { mime: 'image/png' }, imageBytes);
```

### Reading Binary Files

```typescript
// Read and display image (browser)
async function displayImage(path: string, tonk: TonkCore) {
  const doc = await tonk.readFile(path);

  if (doc.bytes) {
    const img = document.createElement('img');
    img.src = `data:${doc.content.mime};base64,${doc.bytes}`;
    document.body.appendChild(img);
  }
}
```

## Performance Considerations

### Efficient File Operations

```typescript
// ❌ Inefficient: Sequential operations
for (const file of files) {
  await tonk.createFile(`/data/${file.name}`, file.content);
}

// ✅ Efficient: Parallel operations
await Promise.all(files.map(file => tonk.createFile(`/data/${file.name}`, file.content)));
```

## Error Handling

```typescript
import { FileSystemError } from '@tonk/core';

async function safeReadFile(tonk: TonkCore, path: string) {
  try {
    return await tonk.readFile(path);
  } catch (error) {
    if (error instanceof FileSystemError) {
      console.error(`File error: ${error.message}`);
      return null;
    }
    throw error;
  }
}

// Check existence before operations
async function updateFileIfExists(tonk: TonkCore, path: string, content: any) {
  if (await tonk.exists(path)) {
    await tonk.updateFile(path, content);
  } else {
    await tonk.createFile(path, content);
  }
}
```

## Best Practices

### 1. Organize with Directories

```typescript
// Good: Clear hierarchy
await tonk.createDirectory('/app');
await tonk.createDirectory('/app/components');
await tonk.createDirectory('/app/stores');
await tonk.createDirectory('/app/utils');
```

### 2. Use Descriptive Paths

```typescript
// Good: Self-documenting
'/config/app-settings.json';
'/data/user-profiles/john-doe.json';
'/cache/api-responses/weather-2024-01-15.json';

// Avoid: Cryptic names
'/c.json';
'/data/u1.json';
'/tmp/x.json';
```

### 3. Clean Up Watchers

Always call `stop()` on watchers when done to prevent memory leaks.

```typescript
const watcher = await tonk.watchFile('/config.json', handleUpdate);

// Later...
await watcher.stop();
```

### 4. Handle Non-Existent Files

```typescript
// Use exists() for cleaner code
if (await tonk.exists(path)) {
  const doc = await tonk.readFile(path);
  // ... process doc
} else {
  // ... handle missing file
}

// Or use updateFile's return value
const updated = await tonk.updateFile(path, content);
if (!updated) {
  await tonk.createFile(path, content);
}
```

## Type Definitions

```typescript
interface DocumentData {
  content: JsonValue;
  name: string;
  timestamps: DocumentTimestamps;
  type: 'document' | 'directory';
  bytes?: string; // Base64-encoded when file has binary data
}

interface RefNode {
  name: string;
  type: 'directory' | 'document';
  pointer: string; // Automerge document ID
  timestamps: DocumentTimestamps;
}

interface DirectoryNode {
  name: string;
  type: 'directory';
  pointer: string;
  timestamps: DocumentTimestamps;
  children?: RefNode[];
}

interface DocumentTimestamps {
  created: number;
  modified: number;
}

interface DocumentWatcher {
  documentId(): string;
  stop(): Promise<void>;
}

type JsonValue =
  | { [key: string]: JsonValue | null | boolean | number | string }
  | (JsonValue | null | boolean | number | string)[];
```

## Synchronization Behavior

### Automatic Sync

When connected to peers, VFS operations automatically synchronize:

```typescript
// Peer A writes
await tonkA.createFile('/shared/doc.txt', 'Hello from A');

// Peer B receives update automatically (if watching)
await tonkB.watchFile('/shared/doc.txt', doc => {
  console.log(doc.content); // "Hello from A"
});
```

### Conflict Resolution

The VFS uses Automerge's CRDT algorithms for automatic conflict resolution. Different types of
changes merge in different ways:

- Object merges: Properties from both sides are preserved
- Array operations: Insertions and deletions are merged
- Primitive overwrites: Last-write-wins based on Lamport timestamps

## Current Limitations

- Maximum recommended file size: 10MB
- No native symlink support
- No file permissions/ownership model
- Directory watches only track direct descendants
- Binary data is base64-encoded (size overhead)
