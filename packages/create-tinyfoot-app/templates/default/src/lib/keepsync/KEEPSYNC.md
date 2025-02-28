# KeepSync

A reactive sync engine framework for use with Tinyfoot.

## Installation

```bash
npm install keepsync
# or
yarn add keepsync
# or
pnpm add keepsync
```

## Features

- Real-time synchronization using CRDTs (Conflict-free Replicated Data Types)
- Built on Automerge for robust conflict resolution
- Seamless integration with React and Zustand
- Automatic reconnection and offline support
- Simple WebSocket-based synchronization
- Persistent storage for offline-first applications

## Basic Usage

### 1. Configure the Sync Engine

First, configure the sync engine at your application's entry point:

```typescript
import { configureSyncEngine } from 'keepsync';

// Configure the sync engine at application startup
configureSyncEngine({
  port: 3030,
  name: 'MySyncEngine',
  onSync: (docId) => console.log(`Document ${docId} synced`),
  onError: (error) => console.error('Sync error:', error),
});
```

### 2. Create a Synced Store

Create a store that automatically syncs with other clients:

```typescript
import { createSyncedStore } from 'keepsync';

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
const PORT = process.env.PORT || 3030;
server.listen(PORT, () => {
  console.log(`Sync server running on port ${PORT}`);
});
```

## License

MIT
