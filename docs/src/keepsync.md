# Keepsync

Keepsync is our local-first sync engine that provides real-time collaboration and local-first data management for Tonk applications. It uses Automerge CRDTs under the hood to enable automatic conflict resolution when multiple users edit the same data, and when working offline.

## Documents and Stores

Keepsync supports two main ways to work with data:

1. **Synced Stores**: Zustand stores enhanced with real-time synchronisation using the `sync` middleware
2. **Direct Document Access**: File-system-like access to individual documents using path-based addressing

Documents are uniquely identified by a `docId` and automatically reconcile state between all clients connected to the same server.

## Basic Usage

### 1. Set Up the Sync Engine

Initialise the sync engine in your application entry point (this is automatically included when you create a Tonk app):

```typescript
// index.tsx
import { configureSyncEngine } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const httpProtocol = window.location.protocol === "https:" ? "https://" : "http://";
const wsUrl = `${wsProtocol}//${window.location.host}/sync`;
const wsAdapter = new BrowserWebSocketClientAdapter(wsUrl);
const storage = new IndexedDBStorageAdapter();

configureSyncEngine({
  url: `${httpProtocol}//${window.location.host}`,
  network: [wsAdapter as any],
  storage,
});
```

### 2. Create a Synced Store with the Middleware

Use the `sync` middleware to create stores that automatically synchronise with other clients:

```typescript
// stores/counterStore.ts
import { create } from "zustand";
import { sync } from "@tonk/keepsync";

interface CounterState {
  count: number;
  increment: () => void;
  decrement: () => void;
  reset: () => void;
}

export const useCounterStore = create<CounterState>(
  sync(
    // The store implementation
    (set) => ({
      count: 0,

      // Increment the counter
      increment: () => {
        set((state) => ({ count: state.count + 1 }));
      },

      // Decrement the counter
      decrement: () => {
        set((state) => ({ count: Math.max(0, state.count - 1) }));
      },

      // Reset the counter
      reset: () => {
        set({ count: 0 });
      },
    }),
    // Sync configuration
    {
      docId: "counter",
      // Optional: configure initialisation timeout (default: 30000ms)
      initTimeout: 30000,
      // Optional: handle initialisation errors
      onInitError: (error) =>
        console.error("Sync initialisation error:", error),
    }
  )
);
```

### 3. Use the Store in React Components

```typescript
// components/Counter.tsx
import React from "react";
import { useCounterStore } from "../stores/counterStore";

export function Counter() {
  // Use the store hook directly - sync is handled by the middleware
  const { count, increment, decrement, reset } = useCounterStore();

  return (
    <div>
      <h2>Collaborative Counter: {count}</h2>
      <div>
        <button onClick={decrement}>-</button>
        <button onClick={increment}>+</button>
        <button onClick={reset}>Reset</button>
      </div>
      <p>
        <small>
          Open this app in multiple windows to see real-time collaboration in
          action.
        </small>
      </p>
    </div>
  );
}
```

## Direct Document Access

For scenarios where you need more fine-grained control over document access, when working outside of React, or when a Zustand store is too heavyweight, you can work directly with documents using filesystem-like paths.

### Reading and Writing Documents

```typescript
import { readDoc, writeDoc } from "@tonk/keepsync";

// Read a document
const userData = await readDoc<{ name: string; email: string }>('users/john');
console.log(userData); // { name: "John Doe", email: "john@example.com" } or undefined

// Write a document
await writeDoc('users/john', {
  name: "John Doe",
  email: "john@example.com",
  lastLogin: new Date().toISOString()
});

// Update an existing document
const currentData = await readDoc('users/john');
if (currentData) {
  await writeDoc('users/john', {
    ...currentData,
    lastLogin: new Date().toISOString()
  });
}
```

### Listening to Document Changes

You can listen for changes to specific documents without using the full sync middleware:

```typescript
import { listenToDoc } from "@tonk/keepsync";

// Attach a listener to a document
const removeListener = await listenToDoc('users/john', (doc) => {
  console.log('User document changed:', doc);
  // Update UI or trigger other side effects
});

// Later, when you want to stop listening
removeListener();
```

### File System Operations

Keepsync provides filesystem-like operations for organising your documents:

```typescript
import { ls, mkDir, rm } from "@tonk/keepsync";

// List contents of a directory
const contents = await ls('users');
console.log(contents); // Returns DocNode with children array

// Create a directory structure
await mkDir('projects/tonk-app/data');

// Remove a document or directory (recursively)
const success = await rm('users/inactive-user');
console.log(success); // true if removed successfully
```

## Advanced Features

### Document Types and Structure

Keepsync organises documents in a hierarchical structure similar to a filesystem:

```typescript
import type { DocNode, DirNode, RefNode } from "@tonk/keepsync";

// DocNode: Represents a document or directory
interface DocNode {
  type: 'doc' | 'dir';
  pointer?: DocumentId;
  name: string;
  timestamps: {
    create: number;
    modified: number;
  };
  children?: RefNode[];
}

// DirNode: Represents a directory
interface DirNode {
  type: 'dir';
  name: string;
  timestamps: {
    create: number;
    modified: number;
  };
  children?: RefNode[];
}

// RefNode: Reference to a document or directory
interface RefNode {
  pointer: DocumentId;
  type: 'doc' | 'dir';
  timestamps: {
    create: number;
    modified: number;
  };
  name: string;
}
```

### Error Handling

```typescript
import { readDoc, writeDoc } from "@tonk/keepsync";

try {
  const data = await readDoc('some/path');
  if (!data) {
    console.log('Document not found');
  }
} catch (error) {
  console.error('Sync engine not initialised:', error);
}

// Handle sync initialisation errors in stores
const useMyStore = create(
  sync(
    (set) => ({ /* store definition */ }),
    {
      docId: "my-store",
      onInitError: (error) => {
        // Handle initialisation failures
        console.error('Failed to initialise sync:', error);
        // Could show user notification, retry logic, etc.
      }
    }
  )
);
```

## Best Practices

1. **Use meaningful document paths**: Organise your data logically using clear, hierarchical paths like `users/profiles/john` or `projects/my-app/settings`.

2. **Handle initialisation gracefully**: Always provide `onInitError` callbacks for sync middleware to handle network or initialisation issues.

3. **Choose the right tool**: Use synced stores for application state that needs real-time collaboration, and direct document access for more structured data or when you need filesystem-like operations.

4. **Clean up listeners**: Always call the cleanup function returned by `listenToDoc` when components unmount or when listeners are no longer needed.

5. **Path conventions**: Use forward slashes (`/`) for path separators and avoid starting paths with `/` (they will be normalised automatically).

## API Reference

### Sync Middleware

- `sync<T>(config: StateCreator<T>, options: SyncOptions): StateCreator<T>` - Creates a synced Zustand store

### Document Operations

- `readDoc<T>(path: string): Promise<T | undefined>` - Read a document
- `writeDoc<T>(path: string, content: T): Promise<void>` - Write/update a document
- `listenToDoc<T>(path: string, listener: (doc: T) => void): Promise<() => void>` - Listen for document changes

### Filesystem Operations

- `ls(path: string): Promise<DocNode | undefined>` - List directory contents
- `mkDir(path: string): Promise<DirNode | undefined>` - Create directory structure
- `rm(path: string): Promise<boolean>` - Remove document or directory

### Configuration

- `configureSyncEngine(options: SyncEngineOptions): SyncEngine` - Initialise the sync engine
- `getSyncEngine(): SyncEngine | null` - Get the current sync engine instance
