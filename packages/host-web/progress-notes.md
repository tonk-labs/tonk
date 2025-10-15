# WASM Init Fix Attempts

## Changes Made
1. Updated `packages/core-js/src/init.ts:39` - Changed `init(config.wasmPath)` to `init({ module_or_path: config.wasmPath })`
2. Updated `packages/core-js/src/init.ts:73` - Same change for `initializeTonkWithEmbeddedWasm`
3. Rebuilt WASM from Rust: `cd packages/core-js && ./build.sh`
4. Rebuilt service worker: `cd packages/host-web && pnpm run build:sw`

## Current Issue
Still getting error with old hash `_3c9c0d586e575a16`

The app is fetching WASM from production server: `https://relay.tonk.xyz/tonk_core_bg.wasm`
This server has the OLD WASM file, not the new one we just built.

## Root Cause Analysis
WASM URL is determined by `TONK_SERVER_URL` in `vite.sw.config.ts:20-23`:
```js
TONK_SERVER_URL: JSON.stringify(
  process.env.NODE_ENV === 'production'
    ? 'https://relay.tonk.xyz'
    : 'http://localhost:8081'
),
```

Used in service-worker.ts at:
- Line 343: `const wasmUrl = ${TONK_SERVER_URL}/tonk_core_bg.wasm;`
- Line 932: Similar for loadBundle
- Line 1065: Similar for initializeFromUrl

## Solution
✅ Set `NODE_ENV=development` when building service worker
✅ Started relay server on port 8081 (`cd packages/relay && pnpm install && pnpm run dev`)

## Current Status
- Service worker correctly pointing to `http://localhost:8081`
- Relay server running and listening on port 8081
- ✅ **FIXED Timeout Issue**: Increased `loadBundle` timeouts from 30s to 120s
  - Updated 3 locations in createNewTonk (lines 951, 1038)
  - Updated processTonkFile (line 1233)
  - Updated initializeFromUrl (line 1443)
  - Rebuilt service worker with `NODE_ENV=development pnpm run build:sw`

## New Features Added

### Sprinkles - Unified Dev Environment
Created `packages/sprinkles/` - a unified dev environment that runs all three servers:
- **Location**: `packages/sprinkles/`
- **Purpose**: Run relay, host-web, and app dev server together with mprocs
- **Usage**: `cd packages/sprinkles && pnpm run dev`

#### Architecture:
```
Port 8081: relay server (WebSocket sync, WASM, bundles)
Port 4000: host-web (BIOS/host system, service worker)
Port 4001: app dev server (your app with HMR)
```

#### Features:
1. **Auto-build on start** (`predev` lifecycle hook):
   - Builds WASM from Rust (`packages/core && pnpm run build:browser`)
   - Builds service worker with `NODE_ENV=development`
   - Copies service worker to src for dev

2. **mprocs.yaml configuration**:
   - Runs all three processes with nice TUI
   - Tab to switch between processes
   - Easy start/stop per process

### Local Development Mode with HMR

#### TONK_SERVE_LOCAL Flag
Added `TONK_SERVE_LOCAL` flag to service worker:
- **vite.sw.config.ts:25**: `TONK_SERVE_LOCAL: JSON.stringify(process.env.NODE_ENV !== 'production')`
- **service-worker.ts:14**: Declared flag
- **service-worker.ts:159**: Log flag value on startup

#### Service Worker Proxy
When `TONK_SERVE_LOCAL` is true, service worker proxies requests to local dev server:
- **Location**: `service-worker.ts:585-623`
- **Behavior**:
  - Intercepts fetch requests when appSlug is set
  - Forwards to `http://localhost:4001` instead of VFS
  - Removes appSlug prefix from path
  - Returns local dev server response with full HMR support
  - Falls back to 502 if dev server unreachable

#### App Dev Server Configuration
- **Location**: `packages/sprinkles/app/`
- **Port**: 4001 (configured in `app/vite.config.ts`)
- **HMR**: Configured with explicit WebSocket connection to localhost:4001
- **Integration**: Runs as third process in mprocs alongside relay and host-web

### Workflow
```bash
cd packages/sprinkles
pnpm install  # first time only
pnpm run dev  # builds everything, starts all 3 servers

# Navigate to:
# http://localhost:4000 -> host-web BIOS
# Service worker proxies app requests to port 4001
# Full HMR support for app development
```

## VFS Integration in App Template

### Overview
Integrated Tonk VFS service into the Sprinkles app template, enabling the app to communicate with the VFS through the service worker while maintaining full HMR support.

### Architecture
```
┌─────────────────┐
│   App (4001)    │  <- React app with HMR
│  VFS Service    │     Served from Vite dev server
└────────┬────────┘
         │ postMessage
         ↓
┌─────────────────┐
│ Service Worker  │  <- Handles VFS operations
│    (4000)       │     Already has VFS support built-in
└────────┬────────┘
         │ WebSocket
         ↓
┌─────────────────┐
│  Relay (8081)   │  <- Tonk backend
│  VFS Backend    │
└─────────────────┘
```

### Files Created

#### VFS Library (`packages/sprinkles/app/src/lib/`)
1. **types.ts** - VFS message and response type definitions
2. **vfs-utils.ts** - Byte conversion utilities (base64 ↔ UTF-8)
3. **tonk-worker.ts** - Web worker implementation (fallback, not used in app)
4. **vfs-service.ts** - Main VFS service class
   - Creates service worker proxy that mimics Worker interface
   - Communicates with SW via `navigator.serviceWorker.controller.postMessage()`
   - Listens for responses via `navigator.serviceWorker.addEventListener('message')`
   - Handles connection state management and auto-reconnection
   - Provides methods: readFile, writeFile, deleteFile, listDirectory, exists, watchFile, etc.

#### React Integration (`packages/sprinkles/app/src/`)
1. **contexts/VFSContext.tsx** - VFS React context provider
   - Wraps VFSService in React context
   - Provides useVFS() hook for components
   - Tracks connection state, initialization, errors
   - Auto-initializes VFS on mount

2. **main.tsx** - Updated to wrap App in VFSProvider

3. **App.tsx** - Comprehensive VFS demo component
   - Connection status display with color-coded indicators
   - File operations: read, write, delete, list
   - Byte operations: writeStringAsBytes, readBytesAsString
   - File watchers: watch/unwatch with real-time updates
   - Error handling and output display

4. **App.css** - Clean, modern styling for demo UI

### Dependencies Added
- `@tonk/core: workspace:*` - Core Tonk library
- `mime: ^4.0.0` - MIME type detection for file operations

### Key Features

#### 1. Service Worker Communication
- App uses service worker proxy pattern
- All VFS operations go through service worker
- Service worker handles actual Tonk Core communication
- No direct worker instantiation in app (uses SW instead)

#### 2. Dual Mode Support
**Development (TONK_SERVE_LOCAL = true):**
- Static assets (HTML/JS/CSS) → proxied to localhost:4001 (HMR)
- VFS operations → handled by service worker

**Production (TONK_SERVE_LOCAL = false):**
- All requests → served from VFS
- No external dev server needed

#### 3. Auto-Initialization
- VFS auto-initializes on app mount
- Fetches manifest from `http://localhost:8081/.manifest.tonk`
- Connects to WebSocket at `ws://localhost:8081`
- Connection state tracked and displayed to user

#### 4. Comprehensive Demo
The demo app showcases all VFS capabilities:
- ✅ File read/write/delete operations
- ✅ Directory listing
- ✅ File existence checking
- ✅ Byte-level operations with UTF-8 encoding
- ✅ Real-time file watching
- ✅ Connection state management
- ✅ Error handling and user feedback

### Testing the Integration

1. **Start Sprinkles:**
   ```bash
   cd packages/sprinkles
   pnpm run dev
   ```

2. **Navigate to app:**
   - Open `http://localhost:4000`
   - Load a bundle in host-web BIOS
   - App will auto-initialize VFS
   - Try the demo operations

3. **Verify HMR:**
   - Edit `app/src/App.tsx`
   - Changes should hot-reload instantly
   - VFS operations should continue working

### Documentation
See `packages/sprinkles/VFS_INTEGRATION_SPEC.md` for complete technical specification.

### Next Steps
- Test file watchers with multi-client sync
- Add more comprehensive error handling
- Create reusable VFS hooks for common patterns
- Consider adding VFS devtools for debugging
