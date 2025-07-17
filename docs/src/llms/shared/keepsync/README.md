# Keepsync Instructions

Keepsync is Tonk's local-first sync engine that provides real-time collaboration and data synchronization. This section contains environment-specific instructions for working with keepsync.

## Environment-Specific Instructions

### React/Browser Environment
- **[React/Browser Instructions](./react-browser.md)** - Complete guide for using keepsync in React applications
  - IndexedDB storage for browser persistence
  - WebSocket connections for real-time sync
  - React hooks and Zustand store integration

### Worker/Node.js Environment
- **[Worker/Node.js Instructions](./worker-nodejs.md)** - Complete guide for using keepsync in Node.js workers
  - Node.js filesystem storage
  - Server-side document operations
  - Background processing patterns

## Code Examples

### React Examples
- **[React Examples](./examples/react-examples.md)** - Complete todo application with collaborative features
  - Zustand store with sync middleware
  - React components using synced state
  - Real-time collaboration patterns

### Worker Examples
- **[Worker Examples](./examples/worker-examples.md)** - API data fetching and processing
  - External API integration
  - Document storage and retrieval
  - Background processing workflows

## Key Concepts

### Core Features
- **Real-time Synchronization**: Changes are instantly propagated across all clients
- **Offline-first**: Applications work without internet, sync when reconnected
- **Conflict Resolution**: Automatic conflict resolution using Automerge CRDTs
- **Path-based Storage**: Filesystem-like document organization

### Common Patterns
- **Synced Stores**: Zustand stores with automatic synchronization
- **Document Operations**: Direct document reading and writing
- **File System Operations**: Directory and document management
- **Error Handling**: Graceful handling of network and initialization failures

## Usage Guidelines

1. **Choose the Right Environment**: Use React/Browser instructions for frontend apps, Worker/Node.js for backend services
2. **Follow Path Conventions**: Use clear, hierarchical paths like `users/profiles/john`
3. **Handle Initialization**: Always provide error handling for sync engine initialization
4. **Use Meaningful Document IDs**: Make document IDs descriptive and unique
5. **Clean Up Listeners**: Remove listeners when components unmount or are no longer needed

## API Reference

Both environments provide the same core API with environment-specific adapters:
- `configureSyncEngine()` - Initialize the sync engine
- `readDoc()` / `writeDoc()` - Document operations
- `sync()` middleware - Zustand store synchronization
- `listenToDoc()` - Real-time document listening
- `ls()` / `mkDir()` / `rm()` - Filesystem operations 