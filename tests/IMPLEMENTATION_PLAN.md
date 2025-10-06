# Plan: Bring Testing Setup to Parity with Production

## Current State

**✅ Correct:**

- Tests point to correct relay server (`packages/relay/src/index.ts`)
- Tests use correct bundle (`examples/latergram/latergram.tonk`)
- S3 test bucket configured: `host-web-bundle-storage-test`

**❌ Problem Areas:**

- **Tests replicate worker code** - `/tests/src/test-ui/tonk-worker.ts` is 500+ lines duplicating
  production service worker logic
- **Tests don't use production service worker** - Should use
  `/packages/host-web/src/service-worker.ts`
- **No service worker build process for tests** - Production has `vite.sw.config.ts` but tests don't
  use it
- **Test storage not isolated per test** - Could cause conflicts between concurrent tests
- **Code duplication** - server-manager, connection-manager exist in both `/tests` and
  `/packages/core-js/playwright-tests`

---

## Phase 1: Replace Custom Worker with Production Service Worker 🔥 **CRITICAL**

### Current Architecture (Wrong):

```
Test UI → Custom tonk-worker.ts → @tonk/core
```

### Target Architecture (Correct):

```
Test UI → Production service-worker.ts → @tonk/core
```

### Why This Matters:

- **Production uses service worker** with fetch interception, caching, appSlug routing
- **Tests use simple worker** without these features
- **Can't test real production behavior** (file serving, offline mode, bundle loading via SW)
- **Code drift** - worker implementations will diverge over time

### Implementation Steps:

**1.1: Build Production Service Worker for Tests**

- Add build script to `/tests/package.json` to build service worker from
  `packages/host-web/src/service-worker.ts`
- Configure Vite to bundle service worker with correct env vars (`TONK_SERVER_URL`)
- Output bundled SW to `/tests/src/test-ui/public/service-worker.js`

**1.2: Update Test VFS Service**

- Modify `/tests/src/test-ui/vfs-service.ts` to:
  - Register service worker instead of creating Web Worker
  - Communicate via `navigator.serviceWorker.controller.postMessage()`
  - Handle service worker lifecycle (installing, activating, ready)
- Keep same message protocol but route through service worker

**1.3: Remove Custom Worker**

- Delete `/tests/src/test-ui/tonk-worker.ts` (15,327 bytes of replicated code)
- Update imports in test UI to use VFS service only

**1.4: Test Service Worker Features**

- Test fetch interception (requests to `/app/*` routed through VFS)
- Test appSlug routing logic
- Test bundle caching and restoration
- Test offline capabilities

---

## Phase 2: Configure Isolated Test Storage 🔥 **CRITICAL**

### Current Issue:

```typescript
// server-manager.ts currently uses:
const storageDir = 'automerge-repo-data'; // ❌ Shared across all tests!
```

### Target Configuration:

```typescript
// Each test gets isolated storage:
const storageDir = `.test-storage/${testId}/automerge-data`;
```

### Implementation Steps:

**2.1: Update Server Manager for Storage Isolation**

- Modify `/tests/src/server/server-manager.ts`:
  - Pass unique storage directory per test: `.test-storage/${testId}/`
  - Ensure relay server uses test-specific storage via env var or CLI arg

**2.2: Configure S3 Test Bucket**

- Verify relay uses `host-web-bundle-storage-test` in test mode
- Check `/packages/relay/src/index.ts` line 114:
  ```typescript
  env: {
    ...process.env,
    NODE_ENV: 'test',
    S3_BUCKET_NAME: 'host-web-bundle-storage-test',  // ✅ Already correct!
  }
  ```

**2.3: Add Storage Cleanup**

- Add `afterEach` hook in fixtures to:
  - Stop relay server
  - Delete `.test-storage/${testId}/` directory
  - Clear IndexedDB databases for test
- Prevent storage bloat during test runs

---

## Phase 3: Test Real Bundle Upload/Download/Sync Flow 🔥 **CRITICAL**

### Critical Bug to Reproduce:

> "Websocket sync breaks after accessing uploaded bundles"

### Test Scenario:

```typescript
1. Client A: Enable sync, make changes → ✅ Sync works
2. Client A: Upload bundle to S3 via relay
3. Client B: Download bundle from S3 via relay
4. Client A: Make more changes → ❌ Sync BREAKS
```

### Implementation Steps:

**3.1: Verify Bundle Upload Uses Real Endpoints**

- Check `/tests/tests/bundles/bundle-upload.spec.ts`
- Ensure it calls relay's `POST /api/bundles` endpoint
- Verify bundle is actually uploaded to `host-web-bundle-storage-test` S3 bucket

**3.2: Create Post-Upload Sync Test** (Most Important!)

```typescript
test('websocket sync works after bundle upload', async ({ browser, serverInstance }) => {
  const page1 = await setupClient(browser, serverInstance);
  const page2 = await setupClient(browser, serverInstance);

  // Verify sync works BEFORE upload
  await page1.increment(); // counter = 1
  await page2.waitForCounter(1); // ✅ Sync works

  // Upload bundle
  await page1.uploadBundle();

  // Download bundle on page2
  await page2.downloadBundle();

  // CRITICAL: Verify sync STILL works AFTER upload
  await page1.increment(); // counter = 2
  await page2.waitForCounter(2, { timeout: 10000 }); // ❌ Likely fails here

  // This test should reveal the bug!
});
```

**3.3: Test Slim Bundle Downloads**

- Test relay's `GET /api/bundles/:id/manifest` endpoint
- Verify only manifest + root document storage is returned
- Measure bundle size difference (full vs slim)

---

## Phase 4: Consolidate Duplicated Code ⚠️ **MEDIUM PRIORITY**

### Duplicated Code Locations:

| Component               | Location 1           | Location 2                                       | Lines           |
| ----------------------- | -------------------- | ------------------------------------------------ | --------------- |
| `server-manager.ts`     | `/tests/src/server/` | `/packages/core-js/playwright-tests/src/server/` | ~246 lines each |
| `connection-manager.ts` | `/tests/src/utils/`  | `/packages/core-js/playwright-tests/src/utils/`  | ~427 lines each |
| `metrics-collector.ts`  | `/tests/src/utils/`  | `/packages/core-js/playwright-tests/src/utils/`  | ~? lines        |
| `server-profiler.ts`    | `/tests/src/utils/`  | `/packages/core-js/playwright-tests/src/utils/`  | ~? lines        |

**Total replicated code:** ~1,500+ lines

### Implementation Steps:

**4.1: Create Shared Test Utilities Package**

```
/packages/test-utils/
  ├── package.json
  ├── tsconfig.json
  ├── src/
  │   ├── server-manager.ts       (unified implementation)
  │   ├── connection-manager.ts   (unified implementation)
  │   ├── metrics-collector.ts    (unified implementation)
  │   └── server-profiler.ts      (unified implementation)
  └── index.ts
```

**4.2: Update Imports**

- `/tests/` imports from `@tonk/test-utils`
- `/packages/core-js/playwright-tests/` imports from `@tonk/test-utils`

**4.3: Remove Duplicates**

- Delete old implementations
- Run tests to verify nothing breaks

---

## Phase 5: Performance & Stress Testing ℹ️ **LOW PRIORITY**

### Test Scenarios:

**5.1: Many Concurrent Connections**

- Spawn 100+ browser contexts, each with WebSocket connection
- Verify all connect successfully
- Measure connection establishment time
- Monitor relay server memory/CPU usage

**5.2: Sync Performance**

- Measure latency for state updates across clients
- Test with varying payload sizes (1KB, 100KB, 1MB)
- Test rapid updates (100+ ops/sec)
- Generate performance report with P50, P95, P99 latencies

**5.3: Bundle Operations Under Load**

- Test concurrent bundle uploads (10+ clients uploading simultaneously)
- Test bundle downloads with high concurrency
- Verify S3 rate limits don't cause failures

---

## Implementation Order (Start to Finish)

### 🔥 Do First (Critical Path):

1. ✅ Build production service worker for tests
2. ✅ Update vfs-service.ts to use real service worker
3. ✅ Remove custom tonk-worker.ts
4. ✅ Add per-test storage isolation
5. ✅ Create post-upload sync test (reproduce bug)

### ⚠️ Do Second (Important):

6. Test bundle upload/download with real S3
7. Verify slim bundle downloads work
8. Add storage cleanup hooks

### ℹ️ Do Later (Nice to Have):

9. Consolidate duplicated utilities into `@tonk/test-utils`
10. Add performance benchmarks (100+ connections)
11. Add stress tests for bundle operations

---

## Success Criteria

✅ **Tests use production service worker, not custom worker**  
✅ **Each test has isolated storage (no conflicts)**  
✅ **Bundle upload/download tests hit real S3**  
✅ **Post-upload sync bug is reproducible in tests**  
✅ **Service worker features tested (fetch interception, caching, routing)**  
✅ **No code duplication between test suites**  
✅ **Tests measure performance (latency, throughput)**

---

## Key Files to Modify

### High Priority:

1. `/tests/package.json` - Add service worker build script
2. `/tests/vite.config.ts` - Add SW build configuration (or create separate vite.sw.config.ts)
3. `/tests/src/test-ui/vfs-service.ts` - Use service worker instead of Web Worker
4. `/tests/src/server/server-manager.ts` - Add storage isolation
5. `/tests/tests/bundles/post-upload-sync.spec.ts` - Create critical test

### Medium Priority:

6. `/tests/tests/fixtures.ts` - Add cleanup hooks
7. `/packages/test-utils/` - Create new shared utilities package

### Files to Delete:

- `/tests/src/test-ui/tonk-worker.ts` (15,327 bytes - replicated code)

---

## Detailed Architecture Comparison

### Current Test Architecture (Replicated Code):

```
┌─────────────────────────────────────────────────────────────┐
│ Playwright Browser                                          │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ Test UI (React App)                                  │  │
│  │                                                      │  │
│  │  ┌────────────────┐        ┌──────────────────┐    │  │
│  │  │  VFS Service   │───────>│ Custom Worker    │    │  │
│  │  │  (Wrapper)     │        │ tonk-worker.ts   │    │  │
│  │  └────────────────┘        │ (500+ lines)     │    │  │
│  │                            │ ❌ REPLICATED    │    │  │
│  │                            └──────────────────┘    │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                                   │
                                   │ WebSocket
                                   ▼
                    ┌──────────────────────────────┐
                    │ Relay Server (Test Instance) │
                    │ - Port: 8100-8999            │
                    │ - Storage: shared ❌         │
                    │ - S3: test bucket ✅         │
                    └──────────────────────────────┘
```

### Target Production-Parity Architecture:

```
┌─────────────────────────────────────────────────────────────┐
│ Playwright Browser                                          │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ Test UI (React App)                                  │  │
│  │                                                      │  │
│  │  ┌────────────────┐                                 │  │
│  │  │  VFS Service   │                                 │  │
│  │  │  (SW Manager)  │                                 │  │
│  │  └────────┬───────┘                                 │  │
│  │           │ postMessage                             │  │
│  │           ▼                                         │  │
│  │  ┌──────────────────────────────────────────────┐  │  │
│  │  │ Service Worker (Production Build)           │  │  │
│  │  │ - From: packages/host-web/service-worker.ts │  │  │
│  │  │ - Fetch interception ✅                     │  │  │
│  │  │ - Bundle caching ✅                         │  │  │
│  │  │ - appSlug routing ✅                        │  │  │
│  │  │ - Same code as production! ✅               │  │  │
│  │  └──────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                                   │
                                   │ WebSocket
                                   ▼
                    ┌──────────────────────────────────────┐
                    │ Relay Server (Test Instance)        │
                    │ - Port: 8100-8999                   │
                    │ - Storage: .test-storage/{id}/ ✅   │
                    │ - S3: test bucket ✅                │
                    │ - Isolated per test ✅              │
                    └──────────────────────────────────────┘
```

---

## Benefits of This Approach

### 1. **No Code Duplication**

- Delete 15,327 bytes of replicated worker code
- Consolidate 1,500+ lines of duplicated test utilities
- Single source of truth for service worker logic

### 2. **Test Real Production Behavior**

- Service worker fetch interception
- Bundle caching and restoration
- appSlug routing
- Offline mode
- File serving through VFS

### 3. **Catch Bugs Earlier**

- Tests run against actual production code
- Service worker bugs caught in tests, not production
- Bundle upload/download flow identical to production

### 4. **Easier Maintenance**

- Fix service worker bug once, tests update automatically
- Add service worker feature once, tests can verify it
- No need to keep worker implementations in sync

### 5. **Better Performance Testing**

- Isolated storage prevents test interference
- Can run tests in parallel safely
- Accurate performance measurements

---

## Risk Mitigation

### Potential Issues:

**1. Service Worker Scope Limitations**

- Service workers require HTTPS or localhost
- ✅ Tests already run on localhost (vite dev server)

**2. Service Worker Registration Delays**

- SW registration is async, might slow tests
- Mitigation: Wait for SW activation before running tests

**3. Service Worker Persistence Between Tests**

- SW might persist state across test runs
- Mitigation: Unregister SW after each test, clear caches

**4. Build Complexity**

- Need to build SW separately from test UI
- Mitigation: Add npm script, document in README

---

## Timeline Estimate

### Phase 1 (Critical): 4-6 hours

- Build SW setup: 2 hours
- Update vfs-service: 2 hours
- Testing & debugging: 2 hours

### Phase 2 (Critical): 2-3 hours

- Storage isolation: 1 hour
- Cleanup hooks: 1 hour
- Testing: 1 hour

### Phase 3 (Critical): 3-4 hours

- Create post-upload sync test: 2 hours
- Debug if bug is found: 2 hours

### Phase 4 (Medium): 6-8 hours

- Create test-utils package: 2 hours
- Migrate code: 2 hours
- Update imports: 2 hours
- Testing: 2 hours

### Phase 5 (Low): 4-6 hours

- Performance tests: 4 hours
- Documentation: 2 hours

**Total Estimated Time: 19-27 hours**

For critical path only (Phases 1-3): **9-13 hours**

---

## Next Steps

1. **Review this plan** - Get team approval
2. **Start with Phase 1.1** - Build service worker for tests
3. **Test incrementally** - Verify each step works before moving on
4. **Document as you go** - Update README with new setup instructions
5. **Measure improvements** - Track code deletion, test coverage, performance

---

## Questions to Answer

- [ ] Should we use same Vite config as host-web or separate one for tests?
- [ ] Should service worker be built on-demand or pre-built?
- [ ] Should we use LocalStack for S3 testing or actual AWS S3 test bucket?
- [ ] Should test-utils be a separate npm package or monorepo internal package?
- [ ] What's the IndexedDB cleanup strategy between tests?

---

## Resources

- Production service worker: `/packages/host-web/src/service-worker.ts`
- Production SW build config: `/packages/host-web/vite.sw.config.ts`
- Test relay server: `/packages/relay/src/index.ts`
- Test fixtures: `/tests/tests/fixtures.ts`
- Current test worker: `/tests/src/test-ui/tonk-worker.ts` (to be deleted)
