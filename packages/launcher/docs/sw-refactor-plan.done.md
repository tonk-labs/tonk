# Service Worker Refactor Plan

## Context

Refactoring `sw-host.ts` (~1900 lines) into logical modules with a state machine for bundle management.

---

## Current State Analysis

### Scattered Global State

The SW has multiple related variables at module level that are implicitly coupled:

```typescript
let tonkState: TonkState = { status: 'uninitialized' };
let appSlug: string | null = null;
let wsUrl: string | null = null;
let healthCheckInterval: number | null = null;
let connectionHealthy = true;
let reconnectAttempts = 0;
let initializationPromise: Promise<void> | null = null;
const watchers = new Map();
```

### Bundle Caching (Current)

Two separate caching mechanisms exist:

1. **Launcher's `bundleStorage`** (`src/launcher/services/bundleStorage.ts`)
   - IndexedDB storage for multiple bundles by ID
   - Used by the launcher UI to list/manage bundles

2. **SW's Cache API** (`persistBundleBytes`, `persistAppSlug`, `persistWsUrl`)
   - Only stores the *currently active* bundle
   - Used for SW restart recovery
   - Overwrites on each bundle load

### Bundle Switching (Current Behavior)

When clicking a different bundle:
1. `App.tsx` sets new `runtimeUrl` with `?bundleId=newId`
2. Iframe remounts (due to `key={runtimeUrl}`)
3. RuntimeApp sends `loadBundle` with new bytes
4. SW overwrites `tonkState` with new TonkCore

**Problems:**
- No cleanup of old TonkCore (WebSocket, watchers, intervals may leak)
- Cache gets overwritten (loses previous bundle state)
- No explicit "unload" step

---

## Target File Structure

```
src/launcher/sw/
├── index.ts              # Entry point, event listeners (~100 lines)
├── state.ts              # State management (~100 lines)
├── cache.ts              # Cache API persistence (~150 lines)
├── tonk-lifecycle.ts     # TonkCore init, bundle loading (~250 lines)
├── connection.ts         # WebSocket health/reconnect (~150 lines)
├── fetch-handler.ts      # Fetch interception, VFS serving (~300 lines)
├── types.ts              # Type definitions
├── message-handlers/
│   ├── index.ts          # Message router
│   ├── file-ops.ts       # read/write/delete/rename/exists (~150 lines)
│   ├── directory-ops.ts  # listDirectory (~50 lines)
│   ├── watch-ops.ts      # file/directory watchers (~200 lines)
│   ├── bundle-ops.ts     # loadBundle, toBytes, forkToBytes (~150 lines)
│   └── init-ops.ts       # init, initializeFrom*, getManifest (~200 lines)
└── utils/
    ├── logging.ts        # log() helper (~30 lines)
    ├── path.ts           # determinePath() (~80 lines)
    └── response.ts       # postResponse(), targetToResponse() (~50 lines)
```

---

## State Machine Design

```typescript
type BundleState =
  | { status: 'idle' }
  | { status: 'loading'; bundleId: string; promise: Promise<void> }
  | {
      status: 'active';
      bundleId: string;
      tonk: TonkCore;
      manifest: Manifest;
      appSlug: string;
      wsUrl: string;
      healthCheckInterval: number;
      watchers: Map<string, any>;
      connectionHealthy: boolean;
      reconnectAttempts: number;
    }
  | { status: 'error'; error: Error; previousBundleId?: string };

let state: BundleState = { status: 'idle' };

function transitionTo(newState: BundleState) {
  const oldState = state;

  // Cleanup old state automatically
  if (oldState.status === 'active') {
    clearInterval(oldState.healthCheckInterval);
    oldState.watchers.forEach(w => w.stop());
    oldState.tonk.disconnectWebsocket?.();
  }

  state = newState;
  persistState(state);
}
```

---

## Implementation Phases

### Phase 1: Utils (no behavioral change)
- `sw/utils/logging.ts` - DEBUG_LOGGING, log()
- `sw/utils/response.ts` - postResponse(), targetToResponse()
- `sw/utils/path.ts` - determinePath()

### Phase 2: Cache layer
- `sw/cache.ts` - persist*/restore* functions

### Phase 3: State machine
- `sw/types.ts` - BundleState, FetchEvent
- `sw/state.ts` - state variable, transitionTo(), getActiveBundle()

### Phase 4: Connection management
- `sw/connection.ts` - health check, reconnect, watchers

### Phase 5: TonkCore lifecycle
- `sw/tonk-lifecycle.ts` - PathIndex sync, autoInitialize, bundle loading

### Phase 6: Message handlers
- `sw/message-handlers/file-ops.ts`
- `sw/message-handlers/directory-ops.ts`
- `sw/message-handlers/watch-ops.ts`
- `sw/message-handlers/bundle-ops.ts`
- `sw/message-handlers/init-ops.ts`
- `sw/message-handlers/index.ts`

### Phase 7: Fetch handler
- `sw/fetch-handler.ts`

### Phase 8: Entry point
- `sw/index.ts`
- Update `vite.sw.config.ts`
- Delete `sw-host.ts`

---

## Verification After Each Phase

1. `bunx tsc --noEmit` - TypeScript check
2. `TONK_SERVE_LOCAL=false npx vite build -c vite.sw.config.ts` - Build SW
3. Copy to `public/app/`, test in browser
4. Verify bundle loading, file serving, hotswap still work
