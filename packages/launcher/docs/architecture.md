# Launcher Architecture

This document explains how the Tonk Launcher system works, including bundle loading, service worker
management, and the development workflow.

## Overview

The launcher is a two-part system:

1. **Launcher App** (`src/main.tsx` → `src/App.tsx`) - The main UI with sidebar for managing bundles
2. **Runtime App** (`src/runtime/main.tsx` → `src/runtime/RuntimeApp.tsx`) - Runs inside an iframe,
   manages the service worker and hosts Tonk apps

```
┌─────────────────────────────────────────────────────────────┐
│  Launcher (localhost:5173)                                  │
│  ┌─────────────┐  ┌───────────────────────────────────────┐ │
│  │   Sidebar   │  │              Main Area                │ │
│  │             │  │  ┌─────────────────────────────────┐  │ │
│  │  [Bundle 1] │  │  │  iframe: /app/index.html        │  │ │
│  │  [Bundle 2] │  │  │                                 │  │ │
│  │             │  │  │  Runtime App                    │  │ │
│  │  [+ Add]    │  │  │  └── Service Worker             │  │ │
│  │             │  │  │      └── TonkCore (VFS)         │  │ │
│  └─────────────┘  │  └─────────────────────────────────┘  │ │
│                   └───────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Key Components

### 1. Bundle Storage (IndexedDB)

**File:** `src/launcher/services/bundleStorage.ts`

Bundles are stored in IndexedDB under the database `tonk-launcher`. This storage is shared between
the launcher and runtime (same origin).

```typescript
// Save a bundle
await bundleStorage.save(id, { name, bytes, size });

// Retrieve a bundle
const bundle = await bundleStorage.get(id);

// List all bundles (metadata only)
const bundles = await bundleStorage.list();
```

### 2. Launcher App

**File:** `src/App.tsx`

The launcher displays available bundles and handles:

- Importing new bundles via file upload
- Launching bundles by setting the iframe URL

When launching a bundle:

```typescript
// Pass bundleId as query param - RuntimeApp fetches bytes from IndexedDB
setRuntimeUrl(`/app/index.html?bundleId=${encodeURIComponent(id)}`);
```

### 3. Runtime App

**File:** `src/runtime/RuntimeApp.tsx`

The runtime runs inside `/app/index.html` and:

1. Registers the service worker
2. Reads `bundleId` from URL params
3. Fetches bundle bytes from IndexedDB
4. Sends bytes to service worker via `loadBundle` message
5. Boots the first app in the bundle

```
RuntimeApp Flow:
┌──────────────────┐
│ Check URL params │
└────────┬─────────┘
         │
    ┌────▼────┐
    │bundleId?│
    └────┬────┘
         │
    Yes  │  No
    ┌────▼────────────────┐  ┌────▼─────────────────┐
    │ Fetch from IndexedDB│  │ Boot from SW cache   │
    │ Send to SW          │  │ (auto-initialized)   │
    └────┬────────────────┘  └────┬─────────────────┘
         │                        │
         └────────┬───────────────┘
                  │
         ┌────────▼────────┐
         │ Boot first app  │
         │ (redirect to    │
         │  /{appSlug}/)   │
         └─────────────────┘
```

### 4. Service Worker

**Directory:** `src/launcher/sw/`

The service worker is the core of the system, organized into modular components:

```
src/launcher/sw/
├── index.ts              # Entry point, event listeners
├── state.ts              # State management (BundleState machine)
├── cache.ts              # Cache API persistence
├── tonk-lifecycle.ts     # TonkCore init, bundle loading
├── connection.ts         # WebSocket health/reconnect
├── fetch-handler.ts      # Fetch interception, VFS serving
├── wasm-init.ts          # WASM initialization
├── types.ts              # Type definitions
├── message-handlers/
│   ├── index.ts          # Message router
│   ├── file-ops.ts       # read/write/delete/rename/exists
│   ├── directory-ops.ts  # listDirectory
│   ├── watch-ops.ts      # file/directory watchers
│   ├── bundle-ops.ts     # loadBundle, toBytes, forkToBytes
│   └── init-ops.ts       # init, initializeFrom*, getManifest
└── utils/
    ├── logging.ts        # log() helper
    ├── path.ts           # determinePath()
    └── response.ts       # postResponse(), targetToResponse()
```

It:

- Manages TonkCore (the VFS/CRDT engine)
- Intercepts fetch requests and serves files from the VFS
- Handles bundle loading and initialization
- Persists state to Cache API for survival across restarts

#### Service Worker State

```typescript
type TonkState =
  | { status: 'uninitialized' }
  | { status: 'loading'; promise: Promise<...> }
  | { status: 'ready'; tonk: TonkCore; manifest: Manifest }
  | { status: 'failed'; error: Error };
```

#### Message Types

| Message         | Direction | Purpose                                      |
| --------------- | --------- | -------------------------------------------- |
| `loadBundle`    | Page → SW | Load bundle bytes into TonkCore              |
| `setAppSlug`    | Page → SW | Set the current app slug for path resolution |
| `getManifest`   | Page → SW | Get manifest with entrypoints                |
| `listDirectory` | Page → SW | List files in VFS directory                  |
| `readFile`      | Page → SW | Read file from VFS                           |
| `writeFile`     | Page → SW | Write file to VFS                            |
| `ready`         | SW → Page | Service worker is ready                      |

#### Auto-Initialization from Cache

On service worker startup, `autoInitializeFromCache()` attempts to restore state:

```
SW Startup:
┌─────────────────────────┐
│ Check Cache API for:    │
│ - appSlug               │
│ - bundleBytes           │
│ - wsUrl                 │
└───────────┬─────────────┘
            │
      Found │ Not Found
      ┌─────▼─────┐  ┌─────▼─────┐
      │Initialize │  │Wait for   │
      │TonkCore   │  │loadBundle │
      │from cache │  │message    │
      └───────────┘  └───────────┘
```

#### Fetch Interception

When `appSlug` is set, the SW intercepts same-origin requests:

```
Request: /myapp/index.html
         ↓
determinePath() → "myapp/index.html"
         ↓
tonk.readFile("/myapp/index.html")
         ↓
Return Response with file content
```

## Build Artifacts

The runtime app and service worker are pre-built and placed in `public/app/`:

```
public/app/
├── index.html              # Runtime HTML entry
├── main.js                 # Runtime app bundle
├── main.css                # Runtime styles
├── service-worker-bundled.js  # Service worker bundle
└── *.otf                   # Fonts
```

## Build Commands

### Full Build

```bash
bun run build:host
```

This runs `scripts/build-runtime.ts` which:

1. Builds service worker → `dist-sw/`
2. Builds runtime app → `dist-runtime/`
3. Copies both to `public/app/`
4. Cleans up temp directories

### Development Builds

**Service Worker only:**

```bash
TONK_SERVE_LOCAL=false npx vite build -c vite.sw.config.ts
cp dist-sw/service-worker-bundled.js public/app/
```

**Runtime App only:**

```bash
TONK_SERVE_LOCAL=true npx vite build -c vite.runtime.config.ts
cp -r dist-runtime/* public/app/
```

### Watch Mode (Dev)

```bash
bun run dev
```

This starts:

- Vite dev server for launcher on port 5173
- SW watch/rebuild in background (`watch:sw`)

**Note:** The runtime app (`public/app/main.js`) is NOT watched. You must manually rebuild it after
changes to `src/runtime/`.

## Configuration Flags

### TONK_SERVE_LOCAL

Controls whether the service worker proxies requests to a local dev server.

| Value   | Behavior                                               |
| ------- | ------------------------------------------------------ |
| `true`  | SW proxies requests to `http://localhost:4001` for HMR |
| `false` | SW serves files from VFS bundle                        |

**When to use each:**

- `TONK_SERVE_LOCAL=true`: Developing an app with hot reload (need separate app dev server)
- `TONK_SERVE_LOCAL=false`: Loading pre-built bundles from IndexedDB (launcher scenario)

### TONK_SERVER_URL

The relay server URL for WebSocket sync.

| Environment | Default                  |
| ----------- | ------------------------ |
| Development | `http://localhost:8081`  |
| Production  | `https://relay.tonk.xyz` |

## Data Flow: Loading a Bundle

```
1. User clicks bundle in sidebar
   │
2. App.tsx: setRuntimeUrl('/app/index.html?bundleId=abc123')
   │
3. iframe loads, RuntimeApp mounts
   │
4. RuntimeApp: Register/wait for service worker
   │
5. RuntimeApp: bundleStorage.get('abc123') → bytes from IndexedDB
   │
6. RuntimeApp: sendMessage({ type: 'loadBundle', bundleBytes })
   │
7. SW: Initialize WASM if needed
   │
8. SW: TonkCore.fromBytes(bundleBytes)
   │
9. SW: Connect WebSocket to relay
   │
10. SW: Wait for PathIndex sync
    │
11. SW: tonkState = { status: 'ready', tonk, manifest }
    │
12. SW: Persist bundleBytes to Cache API
    │
13. SW: postResponse({ type: 'loadBundle', success: true })
    │
14. RuntimeApp: queryAvailableApps() → get entrypoints from manifest
    │
15. RuntimeApp: confirmBoot(appSlug) → redirect to /{appSlug}/
    │
16. SW: Intercept requests, serve from VFS
```

## Troubleshooting

### "Tonk not initialized" errors

The service worker hasn't finished loading the bundle. Causes:

- Bundle not loaded yet (race condition)
- Auto-initialization from cache failed
- `loadBundle` message failed

**Fix:** The SW now waits for initialization before handling fetch requests (with 15s timeout).

### CORS errors with localhost:4001

The SW is built with `TONK_SERVE_LOCAL=true` but no app dev server is running.

**Fix:** Rebuild SW with `TONK_SERVE_LOCAL=false`:

```bash
TONK_SERVE_LOCAL=false npx vite build -c vite.sw.config.ts
cp dist-sw/service-worker-bundled.js public/app/
```

### Changes to RuntimeApp not taking effect

The `public/app/main.js` is pre-built and not watched in dev mode.

**Fix:** Rebuild the runtime:

```bash
npx vite build -c vite.runtime.config.ts
cp -r dist-runtime/* public/app/
```

### Service worker not updating

Browsers cache service workers aggressively.

**Fix:**

1. DevTools → Application → Service Workers → "Unregister"
2. Hard refresh (Cmd+Shift+R / Ctrl+Shift+R)

### Bundle loads but app doesn't render

Check that:

1. `appSlug` is set correctly in SW
2. Files exist in VFS at `/{appSlug}/index.html`
3. No JavaScript errors in the hosted app

## File Reference

| File                                     | Purpose                    |
| ---------------------------------------- | -------------------------- |
| `src/App.tsx`                            | Launcher UI                |
| `src/main.tsx`                           | Launcher entry             |
| `src/runtime/RuntimeApp.tsx`             | Runtime/iframe app         |
| `src/runtime/main.tsx`                   | Runtime entry              |
| `src/launcher/sw/index.ts`               | Service worker entry       |
| `src/launcher/sw/state.ts`               | Bundle state machine       |
| `src/launcher/sw/tonk-lifecycle.ts`      | TonkCore lifecycle mgmt    |
| `src/launcher/sw/fetch-handler.ts`       | Request interception       |
| `src/launcher/services/bundleStorage.ts` | IndexedDB bundle storage   |
| `src/launcher/services/bundleManager.ts` | Bundle import/management   |
| `src/runtime/hooks/useServiceWorker.ts`  | SW communication hook      |
| `vite.config.ts`                         | Launcher vite config       |
| `vite.sw.config.ts`                      | Service worker vite config |
| `vite.runtime.config.ts`                 | Runtime app vite config    |
| `scripts/build-runtime.ts`               | Full build script          |
| `scripts/watch-sw-copy.ts`               | SW watch/copy script       |

## Known Limitations

### Single TonkCore Instance

The service worker maintains a **single TonkCore instance** that serves all bundle iframes. When switching
between bundles, the SW replaces the active TonkCore.

**Current behavior:**

- Each iframe sends `loadBundle` with its `launcherBundleId` when activated
- SW compares `launcherBundleId` to decide if reload is needed
- If different, SW tears down old TonkCore and creates new one
- If same, SW skips reload (optimization for re-selecting same bundle)

**Implication:** All file operations go through the currently active TonkCore. When you switch bundles:

1. Old TonkCore is disconnected (watchers stop, WebSocket closes)
2. New TonkCore loads and connects to its relay
3. App receives `tonk:bundleReloaded` message to reset state

### Why launcherBundleId vs rootId?

- `rootId` (from manifest) = Genesis document ID needed by TonkCore to rebuild Automerge files
- `launcherBundleId` (from IndexedDB) = Unique identifier for the imported bundle in the launcher

Two bundles can share the same `rootId` if they have the same code but different data (e.g., two desktonk
instances connected to different relays). Using `launcherBundleId` for skip logic ensures proper switching.

---

## Future: Multi-Bundle Isolation (Option 3)

For true isolation where each bundle maintains its own TonkCore simultaneously, the architecture would
need significant changes.

### Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Service Worker                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Bundle State Map                                       ││
│  │  Map<launcherBundleId, {                                ││
│  │    tonk: TonkCore,                                      ││
│  │    manifest: Manifest,                                  ││
│  │    watchers: Map<...>,                                  ││
│  │    wsUrl: string,                                       ││
│  │  }>                                                     ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### Required Changes

1. **`sw/state.ts`**: Change from single `BundleState` to `Map<string, BundleState>`
2. **`sw/tonk-lifecycle.ts`**:
   - `loadBundle()` creates/returns existing TonkCore instead of replacing
   - Add `unloadBundle(launcherBundleId)` for cleanup when iframe evicted
3. **`sw/fetch-handler.ts`**: Route requests to correct TonkCore based on referrer/client ID
4. **`sw/message-handlers/*`**: All handlers need to identify which TonkCore to use
5. **Message protocol**: All messages need `launcherBundleId` to route correctly
6. **`RuntimeApp.tsx`**: Include `launcherBundleId` in ALL SW messages, not just `loadBundle`
7. **`App.tsx`**: Notify SW when iframe is evicted from pool so it can cleanup

### Challenges

- **Memory**: Multiple TonkCore instances consume more memory
- **WebSocket connections**: Each bundle maintains its own connection
- **Cache management**: Need per-bundle cache keys
- **Message routing**: SW must route messages to correct TonkCore based on client

### Migration Path

1. ✅ Implement Option 1 - uses `launcherBundleId` for skip logic (current state)
2. Change `state.ts` to use Map with `launcherBundleId` as key
3. Update all message handlers to accept `launcherBundleId` and route to correct instance
4. Add cleanup on iframe eviction from pool
