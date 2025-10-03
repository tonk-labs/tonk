# Bootload Context: Host-Web + Bundle Loading Architecture

This document explains the architecture used for loading and running Tonk applications through the
host-web runtime.

## High-Level Architecture

The system consists of two main components that work together to provide a dynamic, WASM-based
application environment:

```
┌─────────────────┐    ┌─────────────────┐
│   .tonk Bundle  │───▶│    host-web     │
│  (drag & drop   │    │ (tonk runtime)  │
│   or remote)    │    │                 │
└─────────────────┘    └─────────────────┘
                              │
                              │ connects to
                              ▼
                       ┌─────────────────┐
                       │  Relay Server   │
                       │ (sync endpoint) │
                       └─────────────────┘
```

## Component Breakdown

### 1. Tonk Bundle (.tonk file)

- **Format**: ZIP archive containing application files and manifest
- **Loading Methods**:
  - Local: Drag and drop onto host-web interface
  - Remote: Fetch from URL
- **Contents**:
  - `manifest.json` - Application metadata, network endpoints, rootId
  - Application files organized in directory structure
  - Assets and compiled code

### 2. Host-Web (Runtime Environment)

- **Location**: `/packages/host-web/`
- **Purpose**: Provides the runtime environment that loads and executes `.tonk` applications
- **Port**: `localhost:3000` (development)
- **Key Components**:
  - WASM runtime (loads locally, no server dependency)
  - Service Worker for request interception
  - Virtual File System (VFS) backed by Automerge CRDTs
  - Bundle loader and parser

### 3. Relay Server

- **Location**: `/examples/latergram/server/` (example implementation)
- **Purpose**: WebSocket relay for real-time synchronization between peers
- **Port**: `localhost:8081` (configurable)
- **Responsibilities**:
  - Relay sync messages between connected peers
  - Maintain peer connections using bundle rootId as room identifier
  - No file serving or WASM hosting

## Detailed Flow

### 1. Host-Web Initialization

```bash
# In host-web directory
npm run dev  # Starts development server on port 3000
```

When you visit `localhost:3000`:

1. **Service Worker Registration**: Browser loads and registers the service worker
2. **WASM Initialization**: Service worker loads the bundled WASM runtime
3. **Bundle Loading Interface**: User sees interface to load a bundle

### 2. Bundle Loading

#### Option A: Drag and Drop (Local)

1. User drags a `.tonk` file onto the host-web interface
2. Browser reads the file as `ArrayBuffer`
3. Host-web parses the ZIP archive
4. Extracts `manifest.json` to read:
   - `rootId` - Unique identifier for this bundle/document
   - `networkUris` - WebSocket URLs for relay servers
   - `entrypoints` - Application entry points
5. Loads files into VFS using Tonk runtime

#### Option B: Remote Fetch

1. User provides URL to a `.tonk` file
2. Host-web fetches the bundle via HTTP
3. Same parsing and loading process as drag-and-drop

### 3. Bundle Structure and RootId

**manifest.json example:**

```json
{
  "manifestVersion": 1,
  "version": { "major": 0, "minor": 1 },
  "rootId": "automerge:2a3b4c5d6e7f8g9h",
  "entrypoints": ["/app/latergram/index.html"],
  "networkUris": ["ws://localhost:8081"],
  "xNotes": "Latergram social media app"
}
```

**Key Points:**

- `rootId` is the Automerge document ID for the bundle's root document
- This rootId identifies the sync "room" on the relay server
- All peers with the same rootId sync their VFS state together
- Files inside the bundle are stored in the VFS with the bundle's root as their parent document

### 4. WebSocket Connection to Relay Server

After loading the bundle:

1. Host-web reads `networkUris` from manifest
2. Establishes WebSocket connection: `ws://localhost:8081`
3. Sends initial sync message with `rootId`
4. Relay server:
   - Uses `rootId` to identify the sync room
   - Connects this peer with other peers sharing the same `rootId`
   - Relays sync messages (Automerge CRDT operations) between peers

### 5. Application Access and URL Structure

To access a loaded application, visit:

```
localhost:3000/${project-name}/
```

For latergram: `localhost:3000/latergram/`

**Why this structure?**

- Applications are bundled with their project name as a namespace: `/app/${project-name}/`
- Enables **multi-bundle support** - multiple applications can run simultaneously
- Each application is isolated within its own directory structure
- The URL path maps directly to the VFS structure

### 6. Redirect Mechanism (404.html)

Host-web includes a redirect system to handle direct navigation to application URLs:

**The Problem**: When you directly visit `localhost:3000/latergram/`, the static file server returns
a 404 because the service worker and bundle haven't loaded yet.

**The Solution**: A two-step redirect process:

1. **404.html Intercept** (`/packages/host-web/src/404.html`):

   ```javascript
   // Saves the intended URL and redirects to root
   sessionStorage.setItem('redirectPath', window.location.href);
   window.location.href = '/';
   ```

2. **Service Worker Bootstrap** (`/packages/host-web/src/index.html`):

   ```javascript
   // After service worker loads, checks for saved redirect
   const redirectPath = sessionStorage.getItem('redirectPath');
   if (redirectPath) {
     sessionStorage.removeItem('redirectPath');
     window.location.href = redirectPath; // Back to /latergram/
   }
   ```

**Flow**:

1. User visits `localhost:3000/latergram/` → 404.html
2. 404.html saves `/latergram/` to sessionStorage, redirects to `/`
3. Root loads, user loads bundle, service worker registers
4. Service worker takes control, index.html reads sessionStorage
5. Redirects back to `localhost:3000/latergram/` → now served by service worker

### 7. Request Interception and Path Mapping

Once initialized, the service worker intercepts all requests to `localhost:3000`:

```javascript
// Example: Request to localhost:3000/latergram/app.js
// 1. Service worker intercepts the request
// 2. Extracts path: "latergram/app.js"
// 3. Maps to VFS path: /app/latergram/app.js
// 4. Reads file from Tonk VFS: tonk.readFile('/app/latergram/app.js')
// 5. Returns file content as HTTP response
```

**Bundle VFS Structure:**

```
/app/
  └── latergram/           ← Project namespace
      ├── index.html       ← Main application entry
      ├── assets/
      │   ├── index.js     ← Bundled JavaScript
      │   └── styles.css   ← Bundled CSS
      └── manifest.json    ← Application metadata
```

## Key Technical Details

### Bundle Format (.tonk files)

- ZIP archive containing:
  - `manifest.json` - Application metadata and configuration
  - Application files organized in directory structure
  - Assets and compiled code
- Loaded directly in the browser (no server needed)
- `rootId` from manifest identifies the document for sync purposes

### Virtual File System (VFS)

- Powered by Tonk WASM runtime (loaded locally)
- Files stored in CRDT (Conflict-free Replicated Data Type) format
- Real-time synchronization via WebSocket relay
- All file operations go through `tonkCore.readFile()`, `tonkCore.createFile()`, etc.
- Bundle files are loaded into VFS on initialization

### Relay Server Architecture

The relay server is a simple WebSocket relay that:

- Accepts connections from peers
- Uses bundle `rootId` to group peers into sync rooms
- Forwards Automerge sync messages between peers in the same room
- Does not store or serve files
- Does not host WASM or application code

**Example Implementation** (`/examples/latergram/server/src/index.ts`):

```typescript
// Simplified concept
const rooms = new Map<string, Set<WebSocket>>();

wss.on('connection', ws => {
  ws.on('message', data => {
    const message = JSON.parse(data);
    const { rootId, syncMessage } = message;

    // Add peer to room based on rootId
    if (!rooms.has(rootId)) {
      rooms.set(rootId, new Set());
    }
    rooms.get(rootId).add(ws);

    // Relay sync message to other peers in same room
    for (const peer of rooms.get(rootId)) {
      if (peer !== ws) {
        peer.send(syncMessage);
      }
    }
  });
});
```

## Development Workflow

### Making Changes to an Application

1. Edit files in `/examples/latergram/src/`
2. Run `pnpm bundle create` to create new `.tonk` bundle
3. Drag and drop the new bundle onto host-web
4. Application reloads with updated code

### Starting the Relay Server

```bash
# In latergram/server directory
npm run dev  # Starts relay server on port 8081
```

### Debugging Issues

1. **Service Worker Problems**: Check browser DevTools → Application → Service Workers
2. **Bundle Loading Issues**: Check console for ZIP parsing or manifest errors
3. **VFS Issues**: Verify files exist in bundle and are loaded into VFS
4. **Sync Issues**: Check relay server connection and rootId matching

### Common Gotchas

- **Wrong URL**: Must visit `localhost:3000/${project-name}/` not just `localhost:3000/`
- **Service Worker Persistence**: May need to unregister and re-register after major changes
- **Bundle Caching**: Hard refresh may be needed after loading a new version of the same bundle
- **RootId Mismatch**: Peers must have the same rootId to sync with each other
- **Relay Server Down**: App still works offline, but won't sync without relay connection

## File System Paths

### Build Artifacts

- Application bundle: `/examples/latergram/latergram.tonk`
- WASM runtime: Bundled with host-web (no separate serving)
- Service worker: `/packages/host-web/dist/service-worker-bundled.js`

### Key Configuration Files

- Service worker source: `/packages/host-web/src/service-worker.ts`
- Relay server implementation: `/examples/latergram/server/src/index.ts`
- Bundle manifest: Inside each `.tonk` file as `manifest.json`

## Architecture Benefits

1. **True Portability**: `.tonk` files can be shared via any medium (USB, email, IPFS, HTTP)
2. **Decentralized**: No dependency on specific servers for application hosting
3. **Offline-First**: Applications work without network connectivity
4. **Simple Relay**: Relay servers only forward messages, don't host content
5. **Multi-Bundle**: Load and run multiple applications simultaneously
6. **Peer-to-Peer Ready**: Direct peer connections possible (relay is optional)

## Comparison to Previous Architecture

**Old Architecture (Server-Hosted)**:

- Server served WASM runtime
- Server served application files
- Server provided both sync and file hosting
- Required server to load application

**New Architecture (Local-First)**:

- WASM runtime bundled with host-web
- Application files in portable `.tonk` bundle
- Relay server only handles sync messages
- Can load and run applications without any server
- Relay server only needed for multi-peer sync

## Future Considerations

- **Bundle Signing**: Cryptographic verification of bundle authenticity
- **Encrypted Bundles**: Group-based encryption for private applications
- **DHT/IPFS Integration**: Distributed bundle storage and discovery
- **Direct P2P**: WebRTC connections bypassing relay server
- **Bundle Versioning**: Semantic versioning and update notifications
- **Partial Loading**: Stream large bundles instead of loading all at once

---

_This architecture enables true application portability and offline-first collaborative software
with minimal infrastructure requirements._
