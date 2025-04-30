# Keepsync

## Creating a Store

Right now there's no way to create a store outside of an app. Stores are uniquely identified by a docId which is used to reconcile state between all clients connected over the server.

If you create a store in the app with a docId that does not exist, then it is created. If it already exists then it will synchronize with the other clients and the server. Simple as that!

You connect your app to the store using the sync middleware. See [Create a Synced Store with the Middleware](###create-a-synced-store-with-the-middleware)

## Basic Usage

### 1. Set Up the Sync Provider

If you create a Tonk app through the Hub or throhugh the CLI, this should already be in the code.

Initialize the sync engine in your application entry point (or before using any synced stores):

```typescript
// index.tsx
import { configureSyncEngine } from "@tonk/keepsync";
import { BrowserWebSocketClientAdapter } from "@automerge/automerge-repo-network-websocket";
import { IndexedDBStorageAdapter } from "@automerge/automerge-repo-storage-indexeddb";

const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const httpProtocol =
  window.location.protocol === "https:" ? "https://" : "http://";
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

Use the `sync` middleware to create stores that automatically synchronize with other clients:

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
      // Optional: configure initialization timeout
      initTimeout: 30000,
      // Optional: handle initialization errors
      onInitError: (error) =>
        console.error("Sync initialization error:", error),
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

You can also directly read and write documents and address them using paths similar to a filesystem. This is useful for when you need more fine-grained control over document access and
a zustand store is too cumbersome (e.g. when you want each document to have its own space and be directly addressable);

```
import { readDoc, writeDoc } from "@tonk/keepsync";

 * Reads a document from keepsync
 *
 * This function retrieves a document at the specified path in your sync engine.
 * It returns the document content if found, or undefined if the document doesn't exist.
 *
 * @param path - The path identifying the document to read
 * @returns Promise resolving to the document content or undefined if not found
 * @throws Error if the SyncEngine is not properly initialized
 */
readDoc = async <T>(path: string): Promise<T | undefined>;

/**
 * Writes content to a document to keepsync
 *
 * This function creates or updates a document at the specified path.
 * If the document doesn't exist, it creates a new one.
 * If the document already exists, it updates it with the provided content.
 *
 * @param path - The path identifying the document to write
 * @param content - The content to write to the document
 * @throws Error if the SyncEngine is not properly initialized
 */
writeDoc = async <T>(path: string, content: T);
```
