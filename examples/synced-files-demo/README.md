# Synced Files Demo

This is a demonstration of using the KeepSync library to create a synced filesystem between multiple clients.

## Features

- Upload files to a synced filesystem
- Download files from any connected client
- Delete files and have the changes sync to all clients
- Real-time synchronization of file metadata and content
- Automatic blob transfer between clients

## How It Works

This demo uses the KeepSync library to:

1. Create a synced document store using Automerge CRDT
2. Store file metadata in the synced document
3. Store file blobs in IndexedDB
4. Automatically sync file metadata between clients
5. Request and transfer file blobs between clients when needed

## Getting Started

### Prerequisites

- Node.js 14+ and npm/yarn

### Installation

1. Install dependencies:

```bash
npm install
# or
yarn
```

2. Start the WebSocket server:

```bash
node server.js
```

3. Start the development server:

```bash
npm start
# or
yarn start
```

4. Open multiple browser windows to `http://localhost:5173` to see the syncing in action

## Testing the Sync

1. Open two browser windows side by side
2. Upload a file in one window
3. Watch as the file metadata appears in the other window
4. Try downloading the file from the second window
5. Delete the file from either window and see it disappear from both

## How the Code Works

### Initialization

```typescript
// Configure the sync engine with WebSocket connection
configureSyncEngine({
  url: 'ws://localhost:3030/sync',
  name: `Client-${CLIENT_ID}`,
  onSync: (docId) => {
    // Handle sync events
  },
  onError: (error) => {
    // Handle errors
  }
});

// Configure the synced file system
configureSyncedFileSystem({
  docId: 'shared-files',
  dbName: 'demo-file-storage',
  storeName: 'file-blobs'
});
```

### Adding Files

```typescript
// Upload a file to the synced filesystem
await addFile(file);
```

### Getting Files

```typescript
// Get all file metadata
const allFiles = await getAllFiles();

// Download a specific file
const blob = await getFile(fileHash);
```

### Removing Files

```typescript
// Delete a file from the synced filesystem
await removeFile(fileHash);
```

## Architecture

- **React Frontend**: Provides the UI for interacting with files
- **KeepSync Library**: Handles document syncing and file management
- **WebSocket Server**: Relays messages between clients
- **IndexedDB**: Stores file blobs locally on each client

## License

MIT
