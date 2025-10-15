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
- Ready to test - refresh browser and try creating new tonk
