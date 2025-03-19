# Synced Files Architecture

```
┌─────────────────────────────────────────┐     ┌─────────────────────────────────────────┐
│                Client A                 │     │                Client B                 │
│                                         │     │                                         │
│  ┌─────────────┐       ┌─────────────┐  │     │  ┌─────────────┐       ┌─────────────┐  │
│  │             │       │             │  │     │  │             │       │             │  │
│  │   React     │◄─────►│  KeepSync   │  │     │  │   React     │◄─────►│  KeepSync   │  │
│  │   App       │       │  Library    │  │     │  │   App       │       │  Library    │  │
│  │             │       │             │  │     │  │             │       │             │  │
│  └─────────────┘       └──────┬──────┘  │     │  └─────────────┘       └──────┬──────┘  │
│                               │         │     │                               │         │
│                        ┌──────▼──────┐  │     │                        ┌──────▼──────┐  │
│                        │             │  │     │                        │             │  │
│                        │  IndexedDB  │  │     │                        │  IndexedDB  │  │
│                        │  (Blobs)    │  │     │                        │  (Blobs)    │  │
│                        │             │  │     │                        │             │  │
│                        └──────┬──────┘  │     │                        └──────┬──────┘  │
│                               │         │     │                               │         │
└───────────────────────┬───────┼─────────┘     └───────────────────────┬───────┼─────────┘
                        │       │                                       │       │
                        │       │                                       │       │
                        │       │                                       │       │
                        │       │                                       │       │
                        │       │                                       │       │
                        │       │                                       │       │
┌───────────────────────▼───────▼───────────────────────────────────────▼───────▼─────────┐
│                                                                                         │
│                                     WebSocket Server                                    │
│                                                                                         │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

## Data Flow

1. **File Upload**:
   - Client A uploads a file
   - File blob is stored in Client A's IndexedDB
   - File metadata is added to the Automerge document
   - Metadata changes are synced to Client B via WebSocket

2. **File Download on Client B**:
   - Client B requests a file that exists in metadata but not locally
   - Client B sends a blob request message via WebSocket
   - Client A receives the request and sends the blob data
   - Client B stores the received blob in its IndexedDB
   - Client B can now access the file locally

3. **File Deletion**:
   - Client A deletes a file
   - File blob is removed from Client A's IndexedDB
   - File metadata is removed from the Automerge document
   - Metadata changes are synced to Client B via WebSocket
   - Client B detects the file is no longer in metadata and removes the blob

## Components

### Client Side

- **React App**: User interface for file operations
- **KeepSync Library**:
  - **SyncEngine**: Handles document syncing using Automerge
  - **SyncedFileManager**: Manages file metadata and blobs
  - **FileManager**: Handles IndexedDB operations for blob storage
- **IndexedDB**: Local storage for file blobs

### Server Side

- **WebSocket Server**: Simple relay server that forwards messages between clients
  - Does not store any data
  - Only responsible for message routing

## Message Types

1. **Document Sync**: Automerge document changes
2. **Blob Request**: Request for a file blob from another client
3. **Blob Response**: Response containing the requested file blob

## Syncing Process

1. **Document Syncing**:
   - Uses Automerge CRDT for conflict-free document syncing
   - Changes are automatically merged without conflicts

2. **Blob Syncing**:
   - On-demand transfer of blobs between clients
   - Only transfers blobs when they are needed
   - Uses custom message protocol over WebSocket
