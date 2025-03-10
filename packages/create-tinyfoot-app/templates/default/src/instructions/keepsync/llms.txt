# KeepSync

A reactive sync engine framework for use with Tinyfoot.

## Installation

```bash
npm install @tonk/keepsync
# or
yarn add @tonk/keepsync
# or
pnpm add @tonk/keepsync
```

## Features

- Real-time synchronization using CRDTs (Conflict-free Replicated Data Types)
- Built on Automerge for robust conflict resolution
- Seamless integration with React and Zustand
- Automatic reconnection and offline support
- Simple WebSocket-based synchronization
- Persistent storage for offline-first applications
- File system synchronization for collaborative file management

## Basic Usage

### 1. Configure the Sync Engine

First, configure the sync engine at your application's entry point:

```typescript
import { configureSyncEngine } from '@tonk/keepsync';

// Configure the sync engine at application startup
configureSyncEngine({
  url: 'ws://localhost:4080',
  name: 'MySyncEngine',
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error('Sync error:', error),
});
```

### 2. Create a Synced Store

Create a store that automatically syncs with other clients:

```typescript
import { createSyncedStore } from '@tonk/keepsync';

// Define your state and actions
interface CounterState {
  count: number;
  increment: () => void;
  decrement: () => void;
  reset: () => void;
}

// Create a synced store
export const useCounterStore = await createSyncedStore<CounterState>(
  {
    // Unique document ID for this store
    docId: 'counter',
    // Initial state if the document doesn't exist yet
    initialState: {
      count: 0,
      // Add stub methods that will be replaced by the actual implementations
      increment: () => {},
      decrement: () => {},
      reset: () => {},
    },
  },
  (set) => ({
    count: 0,

    // Increment the counter
    increment: () => {
      set((state) => ({ count: state.count + 1 }));
    },

    // Decrement the counter
    decrement: () => {
      set((state) => ({ count: state.count - 1 }));
    },

    // Reset the counter
    reset: () => {
      set({ count: 0 });
    },
  })
);
```

### 3. Use the Store in React Components

```typescript
import { useCounterStore } from './store/counterStore';

function Counter() {
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
          Open this app in multiple windows to see real-time collaboration in action.
        </small>
      </p>
    </div>
  );
}
```

### 4. Synced File System

KeepSync also provides a file system API for collaborative file management:

```typescript
import { 
  configureSyncedFileSystem, 
  addFile, 
  getAllFiles, 
  getFile, 
  removeFile 
} from '@tonk/keepsync';

// Configure the synced file system
configureSyncedFileSystem({
  docId: 'my-files',
  dbName: 'my-app-files',
  storeName: 'file-blobs'
});

// Add a file
const fileInput = document.getElementById('fileInput') as HTMLInputElement;
fileInput.addEventListener('change', async () => {
  if (fileInput.files && fileInput.files.length > 0) {
    const file = fileInput.files[0];
    const metadata = await addFile(file);
    console.log('Added file:', metadata);
  }
});

// List all files
async function displayAllFiles() {
  const files = await getAllFiles();
  console.log('All files:', files);
  
  // Display files in UI
  const fileList = document.getElementById('fileList');
  if (fileList) {
    fileList.innerHTML = '';
    files.forEach(file => {
      const item = document.createElement('div');
      item.textContent = `${file.name} (${file.size} bytes)`;
      
      // Add download button
      const downloadBtn = document.createElement('button');
      downloadBtn.textContent = 'Download';
      downloadBtn.onclick = async () => {
        const blob = await getFile(file.hash);
        if (blob) {
          const url = URL.createObjectURL(blob);
          const a = document.createElement('a');
          a.href = url;
          a.download = file.name;
          a.click();
          URL.revokeObjectURL(url);
        }
      };
      
      // Add delete button
      const deleteBtn = document.createElement('button');
      deleteBtn.textContent = 'Delete';
      deleteBtn.onclick = async () => {
        await removeFile(file.hash);
        displayAllFiles(); // Refresh the list
      };
      
      item.appendChild(downloadBtn);
      item.appendChild(deleteBtn);
      fileList.appendChild(item);
    });
  }
}
```

## Setting Up the Sync Server

KeepSync uses a simple WebSocket server for synchronization:

```javascript
import { WebSocketServer } from 'ws';
import { createServer } from 'http';

// Create a simple HTTP server
const server = createServer();
const wss = new WebSocketServer({
  server,
  path: '/sync'
});

// Store connected clients
const connections = new Set();

// Handle WebSocket connections
wss.on('connection', (ws) => {
  console.log('Client connected');
  connections.add(ws);

  // Handle messages from clients
  ws.on('message', data => {
    connections.forEach(client => {
      if (client !== ws && client.readyState === WebSocket.OPEN) {
        client.send(data.toString());
      }
    });
  });

  // Handle client disconnection
  ws.on('close', () => {
    console.log('Client disconnected');
    connections.delete(ws);
  });
});

// Start the server
const PORT = process.env.PORT || 4080;
server.listen(PORT, () => {
  console.log(`Sync server running on port ${PORT}`);
});
```

## Cleanup

When your application is shutting down or when components are unmounting, make sure to clean up resources:

```typescript
import { closeSyncEngine, closeSyncedFileSystem, cleanupSyncedStore } from '@tonk/keepsync';

// Clean up a specific store when a component unmounts
useEffect(() => {
  return () => {
    cleanupSyncedStore(useCounterStore);
  };
}, []);

// Clean up everything when the app is shutting down
function shutdownApp() {
  closeSyncedFileSystem();
  closeSyncEngine();
}
```

## License

MIT
