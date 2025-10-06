# Bundle Upload/Download Implementation Summary

## Overview

Implemented bundle upload/download functionality for Playwright tests using the real S3 production
code path with a test S3 bucket.

## Changes Made

### 1. Production Server (`src/index.ts`)

- Made S3 bucket configurable via environment variable `S3_BUCKET_NAME`
- Default: `host-web-bundle-storage` (production)
- Test override: `host-web-bundle-storage-test`

### 2. Test Server Manager (`playwright-tests/src/server/server-manager.ts`)

- Added `S3_BUCKET_NAME: 'host-web-bundle-storage-test'` to server environment variables
- All test servers now use the test S3 bucket automatically

### 3. VFS Worker Types (`playwright-tests/src/test-ui/types.ts`)

- Added `exportBundle` message type to `VFSWorkerMessage`
- Added `exportBundle` response type to `VFSWorkerResponse`

### 4. VFS Worker (`playwright-tests/src/test-ui/tonk-worker.ts`)

- Added `exportBundle` case handler
- Calls `tonk.toBytes()` to export current state as bundle
- Returns bundle bytes to main thread

### 5. VFS Service (`playwright-tests/src/test-ui/vfs-service.ts`)

- Added `exportBundle()` method
- Sends message to worker to export bundle
- Returns Promise<Uint8Array> of bundle data

### 6. Test UI App (`playwright-tests/src/test-ui/App.tsx`)

- Added bundle state: `bundleStatus` and `uploadedBundleId`
- Added `handleUploadBundle()` function:
  - Calls `vfs.exportBundle()` to get bundle bytes
  - POSTs to `/api/bundles` endpoint (production endpoint)
  - Stores returned bundle ID in `window.uploadedBundleId` for tests
- Added `handleDownloadBundle()` function:
  - Reads bundle ID from `window.__targetBundleId` or uses last uploaded
  - GETs from `/api/bundles/:id/manifest` endpoint (production endpoint)
  - Stores result in `window.downloadedBundle` for tests
- Added UI section with:
  - "Upload Bundle" button (`data-testid="upload-bundle-btn"`)
  - "Download Bundle" button (`data-testid="download-bundle-btn"`)
  - Bundle status display (`data-testid="bundle-status"`)
  - Bundle ID display (`data-testid="bundle-id"`)
- Exposed VFS service as `window.__vfsService` for tests
- Exposed counter store as `window.__counterStore` for tests

## How It Works

### Upload Flow

1. User clicks "Upload Bundle" button
2. VFS service exports current Tonk state as bundle bytes via `tonk.toBytes()`
3. Bundle POSTed to `/api/bundles` (production S3 upload endpoint)
4. Server uploads to `host-web-bundle-storage-test` S3 bucket
5. Server returns bundle ID (extracted from manifest)
6. Bundle ID stored in `window.uploadedBundleId` for test verification

### Download Flow

1. User clicks "Download Bundle" button
2. App reads bundle ID from `window.__targetBundleId` (set by test) or last uploaded ID
3. Bundle fetched from `/api/bundles/:id/manifest` (production S3 download endpoint)
4. Server downloads from `host-web-bundle-storage-test` S3 bucket
5. Success marked in `window.downloadedBundle` for test verification

## Test Usage

Tests can now:

```typescript
// Upload bundle
await page.getByTestId('upload-bundle-btn').click();
await page.waitForFunction(() => (window as any).uploadedBundleId);
const bundleId = await page.evaluate(() => (window as any).uploadedBundleId);

// Download bundle
await page.evaluate(id => {
  (window as any).__targetBundleId = id;
}, bundleId);
await page.getByTestId('download-bundle-btn').click();
await page.waitForFunction(() => (window as any).downloadedBundle);
```

## Benefits

✅ Tests exact production code path (same S3 endpoints) ✅ Isolated test data (separate S3 bucket)
✅ Real Automerge bundle creation via `tonk.toBytes()` ✅ Real bundle upload/download flow ✅ Can
verify sync continues working after bundle operations (the critical bug) ✅ Zero changes to
production server logic ✅ Clean separation via environment variable

## AWS Requirements

- Test environment needs AWS credentials with access to `host-web-bundle-storage-test` bucket
- Same credentials that access production bucket should work
- Bucket must exist in `eu-north-1` region
