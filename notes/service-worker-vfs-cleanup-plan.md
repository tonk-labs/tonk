# Service Worker & VFS Service Cleanup Plan

## Overview

Systematic refactoring of the service worker (1,748 lines → ~6 modules) and VFS service consolidation into `@tonk/host-web`, followed by comprehensive Vitest test coverage.

**Approach**: Refactor first, then add tests to the cleaner structure.

---

## Phase 1: Service Worker Modularization

### Goal
Split `packages/host-web/src/service-worker.ts` from a 1,748-line monolith into focused modules.

### Target Structure
```
packages/host-web/src/
├── service-worker.ts              (~150 lines - entry point & orchestration)
├── service-worker/
│   ├── index.ts                   (re-exports)
│   ├── types.ts                   (consolidated types - single source of truth)
│   ├── state.ts                   (~80 lines - global state management)
│   ├── constants.ts               (~40 lines - configuration constants)
│   ├── logging.ts                 (~30 lines - debug logging utility)
│   ├── lifecycle.ts               (~250 lines - TonkCore init, state machine)
│   ├── vfs-handlers.ts            (~300 lines - file/directory operations)
│   ├── fetch-handler.ts           (~250 lines - HTTP interception, path resolution)
│   ├── websocket-manager.ts       (~150 lines - connection health, reconnection)
│   ├── message-handler.ts         (~150 lines - message dispatcher)
│   └── persistence.ts             (~100 lines - Cache API helpers)
```

### Step 1.1: Extract Constants & Logging
**Files to create:**
- `service-worker/constants.ts` - `HEALTH_CHECK_INTERVAL`, `MAX_RECONNECT_ATTEMPTS`, `DEBUG_LOGGING`, build-time constants
- `service-worker/logging.ts` - `log()` function with debug flag

**Changes to original:** Replace inline constants with imports.

### Step 1.2: Extract State Management
**File to create:** `service-worker/state.ts`
```typescript
// Centralized state with typed accessors
export type TonkState =
  | { status: 'uninitialized' }
  | { status: 'loading'; promise: Promise<...> }
  | { status: 'ready'; tonk: TonkCore; manifest: Manifest }
  | { status: 'failed'; error: Error };

let tonkState: TonkState = { status: 'uninitialized' };
let appSlug: string | null = null;
let wsUrl: string | null = null;
// ... other state

export const getState = () => ({ tonkState, appSlug, wsUrl, ... });
export const setState = (updates: Partial<State>) => { ... };
export const getTonk = (): TonkCore | null => { ... };
```

### Step 1.3: Extract Persistence Layer
**File to create:** `service-worker/persistence.ts`
- `persistAppSlug()` / `restoreAppSlug()`
- `persistBundleBytes()` / `restoreBundleBytes()`
- Cache API constants (`CACHE_NAME`, URLs)

### Step 1.4: Extract WebSocket Manager
**File to create:** `service-worker/websocket-manager.ts`
- `performHealthCheck()`
- `attemptReconnect()`
- `reestablishWatchers()`
- `startHealthMonitoring()`
- Reconnection state variables

### Step 1.5: Extract Lifecycle Management
**File to create:** `service-worker/lifecycle.ts`
- `initializeTonkCore(config)` - unified initialization (replaces 4 duplicated paths)
- `autoInitializeFromCache()`
- `waitForPathIndexSync()`

**Key improvement:** Single `initializeTonkCore()` function replacing ~330 lines of duplication.

### Step 1.6: Extract VFS Handlers
**File to create:** `service-worker/vfs-handlers.ts`
- `handleReadFile()`
- `handleWriteFile()`
- `handleDeleteFile()`
- `handleRename()`
- `handleListDirectory()`
- `handleExists()`
- `handleWatchFile()` / `handleUnwatchFile()`
- `handleWatchDirectory()` / `handleUnwatchDirectory()`
- `handleToBytes()` / `handleForkToBytes()`

### Step 1.7: Extract Fetch Handler
**File to create:** `service-worker/fetch-handler.ts`
- `determinePath()` - path resolution logic
- `targetToResponse()` - VFS target to HTTP Response
- `handleFetch()` - main fetch event logic
- Dev mode proxy handling

### Step 1.8: Create Message Dispatcher
**File to create:** `service-worker/message-handler.ts`
```typescript
import { handleReadFile, handleWriteFile, ... } from './vfs-handlers';
import { initializeTonkCore } from './lifecycle';

const handlers: Record<string, Handler> = {
  readFile: handleReadFile,
  writeFile: handleWriteFile,
  // ... map all 18 message types
};

export async function handleMessage(event: ExtendableMessageEvent) {
  const { type, id, ...payload } = event.data;
  const handler = handlers[type];
  if (!handler) {
    return respond(id, { success: false, error: `Unknown message type: ${type}` });
  }
  try {
    const result = await handler(payload);
    respond(id, { success: true, ...result });
  } catch (error) {
    respond(id, { success: false, error: error.message });
  }
}
```

### Step 1.9: Update Entry Point
**File to modify:** `service-worker.ts` (~150 lines)
```typescript
import { handleMessage } from './service-worker/message-handler';
import { handleFetch } from './service-worker/fetch-handler';
import { autoInitializeFromCache } from './service-worker/lifecycle';

self.addEventListener('install', (event) => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener('fetch', handleFetch);
self.addEventListener('message', handleMessage);

// Auto-init on startup
autoInitializeFromCache();
```

---

## Phase 2: VFS Service Consolidation

### Goal
Move VFSService from `packages/desktonk/src/lib/` to `@tonk/host-web` as the canonical implementation.

### Step 2.1: Consolidate Types
**File to create:** `packages/host-web/src/vfs-client/types.ts`
- Merge types from both `desktonk/src/lib/types.ts` and `host-web/src/types.ts`
- Add missing types: `swReady`, `swInitializing`, `needsReinit`, `ready`, `handshake`
- Export from `@tonk/host-web`

### Step 2.2: Move VFS Service
**Files to create:**
```
packages/host-web/src/vfs-client/
├── index.ts                 (exports)
├── types.ts                 (consolidated types)
├── vfs-service.ts           (main class)
└── vfs-utils.ts             (byte conversion utilities)
```

**Changes:**
- Copy `vfs-service.ts` from desktonk
- Add proper TypeScript types throughout
- Add error handling improvements (retry logic, exponential backoff)
- Add callback exception handling in watcher notifications
- Export from `@tonk/host-web/client` or `@tonk/host-web/vfs`

### Step 2.3: Update Package Exports
**File to modify:** `packages/host-web/package.json`
```json
{
  "exports": {
    ".": "./src/index.ts",
    "./client": "./src/vfs-client/index.ts"
  }
}
```

### Step 2.4: Update Desktonk to Use Package
**Files to modify:**
- `packages/desktonk/package.json` - add `@tonk/host-web` dependency
- `packages/desktonk/src/lib/vfs-service.ts` - replace with re-export:
  ```typescript
  export { VFSService, getVFSService, resetVFSService } from '@tonk/host-web/client';
  export type { VFSWorkerMessage, VFSWorkerResponse, ... } from '@tonk/host-web/client';
  ```

---

## Phase 3: Testing Infrastructure

### Goal
Add comprehensive Vitest tests for the refactored service worker and VFS service.

### Step 3.1: Set Up Vitest for host-web
**Files to create/modify:**
- `packages/host-web/vitest.config.ts`
- `packages/host-web/package.json` - add test script and dependencies

**Dependencies to add:**
```json
{
  "devDependencies": {
    "vitest": "^4.0.12",
    "happy-dom": "^17.0.0",
    "fake-indexeddb": "^6.0.0"
  }
}
```

### Step 3.2: Create Service Worker Test Utilities
**File to create:** `packages/host-web/src/__tests__/test-utils.ts`
```typescript
// Mock ServiceWorkerGlobalScope
export function createMockServiceWorkerScope() { ... }

// Mock TonkCore
export function createMockTonkCore() { ... }

// Mock Cache API
export function createMockCacheStorage() { ... }

// Helper to simulate message events
export function createMessageEvent(data: VFSWorkerMessage) { ... }
```

### Step 3.3: Service Worker Module Tests

**Tests to create:**
```
packages/host-web/src/__tests__/
├── persistence.test.ts      - Cache API operations
├── state.test.ts            - State management
├── lifecycle.test.ts        - Initialization paths
├── vfs-handlers.test.ts     - File operations
├── fetch-handler.test.ts    - HTTP interception
├── websocket-manager.test.ts - Connection health
└── message-handler.test.ts  - Message dispatch
```

**Example test cases:**

`lifecycle.test.ts`:
- `initializeTonkCore` creates TonkCore instance correctly
- `initializeTonkCore` connects WebSocket
- `initializeTonkCore` waits for PathIndex sync
- `autoInitializeFromCache` restores from cached bytes
- `autoInitializeFromCache` handles missing cache gracefully
- State transitions: uninitialized → loading → ready
- State transitions: loading → failed on error

`vfs-handlers.test.ts`:
- `handleReadFile` returns document data
- `handleReadFile` returns error for non-existent file
- `handleWriteFile` creates new file
- `handleWriteFile` updates existing file
- `handleWatchFile` registers callback
- `handleWatchFile` notifies on changes
- `handleUnwatchFile` removes callback
- Directory operations work correctly

`fetch-handler.test.ts`:
- `determinePath` resolves appSlug correctly
- `determinePath` handles root requests
- `determinePath` defaults to index.html
- Fetch handler serves VFS files
- Fetch handler returns 404 for missing files
- Dev mode proxies to Vite server

### Step 3.4: VFS Client Tests

**File to create:** `packages/host-web/src/vfs-client/__tests__/vfs-service.test.ts`

**Test cases:**
- `connect()` verifies service worker is ready
- `readFile()` sends correct message and returns data
- `writeFile()` sends correct message
- `watchFile()` registers watcher and receives updates
- `unwatchFile()` removes watcher
- Request timeout after 30 seconds
- Connection state transitions
- Watcher re-establishment after reconnection
- Error handling for failed operations

### Step 3.5: Integration Tests

**File to create:** `packages/host-web/src/__tests__/integration.test.ts`

**Test cases:**
- Full flow: initialize → write file → read file → verify
- Watch flow: watch → write → callback triggered
- Reconnection flow: disconnect → reconnect → watchers work
- Multiple concurrent operations

---

## Phase 4: Documentation & Cleanup

### Step 4.1: Update Architecture Doc
Update `notes/vfs-service-worker-auth-architecture.md` to reflect new structure.

### Step 4.2: Remove Duplicated Code
- Delete `packages/desktonk/src/lib/vfs-service.ts` (replaced by re-export)
- Delete `packages/desktonk/src/lib/types.ts` (using @tonk/host-web types)
- Delete `packages/desktonk/src/lib/vfs-utils.ts` (using @tonk/host-web utils)

### Step 4.3: Add Package Scripts
**File to modify:** `packages/host-web/package.json`
```json
{
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest",
    "test:coverage": "vitest run --coverage"
  }
}
```

---

## Critical Files to Modify

| File | Action |
|------|--------|
| `packages/host-web/src/service-worker.ts` | Refactor into modules |
| `packages/host-web/src/types.ts` | Move to `service-worker/types.ts`, consolidate |
| `packages/host-web/package.json` | Add exports, test scripts, dependencies |
| `packages/desktonk/src/lib/vfs-service.ts` | Replace with re-export |
| `packages/desktonk/src/lib/types.ts` | Delete (use host-web) |
| `packages/desktonk/package.json` | Add @tonk/host-web dependency |

---

## Implementation Order

1. **Phase 1.1-1.3**: Extract constants, logging, state, persistence (low risk)
2. **Phase 1.4**: Extract WebSocket manager (medium risk)
3. **Phase 1.5**: Extract lifecycle with unified init (high impact)
4. **Phase 1.6-1.7**: Extract VFS handlers and fetch handler (medium risk)
5. **Phase 1.8-1.9**: Create message dispatcher and update entry point
6. **Phase 2**: VFS service consolidation
7. **Phase 3**: Add Vitest tests
8. **Phase 4**: Documentation and cleanup

Each step should be a separate commit to allow easy rollback if issues arise.

---

## Success Criteria

- [ ] Service worker split into ~6 modules, each <300 lines
- [ ] No duplicated initialization code (single `initializeTonkCore`)
- [ ] VFSService exported from `@tonk/host-web/client`
- [ ] Single source of truth for types
- [ ] Vitest tests with >80% coverage on new modules
- [ ] All existing functionality preserved (manual testing)
- [ ] desktonk works with imported VFSService
