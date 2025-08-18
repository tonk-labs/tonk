# Keepsync + Iroh Integration

This document describes how the Tauri browser now uses `@tonk/keepsync` with the Iroh network
adapter for P2P synchronization.

## Architecture Changes

### Before

- Used `@automerge/automerge-repo` directly
- Created a custom Automerge `Repo` instance with the Iroh network adapter
- Data operations were performed directly on the Automerge repo

### After

- Uses `@tonk/keepsync` as the primary synchronization engine
- Configures keepsync's `SyncEngine` with the Iroh network adapter
- Data operations use keepsync's middleware (`readDoc`, `writeDoc`, `ls`, etc.)

## Key Components

### P2PSync Class (`src/lib/p2p-sync.ts`)

- Initializes the Iroh network adapter
- Configures keepsync's `SyncEngine` with:
  - Iroh network adapter for P2P communication
  - IndexedDB storage adapter for persistence
  - Generated peer ID from Iroh node
- Provides access to the configured `SyncEngine` instance

### DataSyncDemo Component (`src/components/DataSyncDemo.tsx`)

- Uses keepsync middleware functions directly
- Waits for the `SyncEngine` to be ready before enabling data operations
- Creates and syncs documents through keepsync's API

### Iroh Network Adapter (`src/lib/adapters/iroh.ts`)

- Remains unchanged - still implements `NetworkAdapterInterface`
- Handles P2P discovery, connection, and message routing
- Integrates with Tauri's Rust backend for Iroh functionality

## Benefits

1. **Consistency**: Uses the same sync engine across all Tonk applications
2. **Maintainability**: Single source of truth for document operations
3. **Features**: Access to keepsync's advanced features like file system abstraction
4. **Testing**: Easier to test with keepsync's built-in testing utilities

## Usage Example

```typescript
import { P2PSync } from './lib/p2p-sync';
import { readDoc, writeDoc, ls } from '@tonk/keepsync';

// Initialize P2P sync with Iroh
const p2pSync = new P2PSync();
await p2pSync.initialize('my-bundle-id');

// Wait for sync engine to be ready
const syncEngine = p2pSync.getSyncEngine();
if (syncEngine) {
  await syncEngine.whenReady();
}

// Use keepsync operations
await writeDoc('/messages', {
  messages: [{ text: 'Hello from device 1', timestamp: Date.now() }],
});

const data = await readDoc('/messages');
console.log('Synced messages:', data);

// List files
const fileList = await ls('/');
console.log('Available files:', fileList);
```

## Testing P2P Sync Between Devices

1. Start the Tauri application on multiple devices
2. Use the same bundle ID on all devices
3. Ensure devices are on the same network
4. Initialize P2P on each device
5. Create messages or files on one device
6. Verify they appear on other devices through Iroh's P2P synchronization

The integration ensures that all data operations go through keepsync while leveraging Iroh's robust
P2P networking capabilities.
