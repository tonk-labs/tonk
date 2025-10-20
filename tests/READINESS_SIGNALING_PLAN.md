# PLAN: Service Worker Readiness Signaling Implementation

## Overview

Add a robust readiness signaling mechanism to prevent VFS operations from being sent before the
service worker completes initialization, fixing the race condition causing 57% error rates in
distributed stress tests.

---

## Phase 1: Service Worker Changes

### File: /packages/host-web/src/service-worker.ts

#### 1.1 Add Readiness State Management (lines 36-46)

Location: After let tonkState: TonkState = { status: 'uninitialized' };

Changes:

• Add readiness tracking variables: let isReady = false; let readyPromise: Promise<void> | null =
null; let readyResolve: (() => void) | null = null;

#### 1.2 Improve Auto-Initialization (lines 318-427)

Changes:

• Make autoInitializeFromCache() set readiness state • Add structured error handling • Send
initialization status messages to clients

New behavior:

async function autoInitializeFromCache() { try { // ... existing initialization logic ...

    // On success:
    tonkState = { status: 'ready', tonk, manifest };
    isReady = true;
    if (readyResolve) readyResolve();
    await broadcastToClients({ type: 'swReady', autoInitialized: true });

} catch (error) { // On failure: tonkState = { status: 'uninitialized' }; await broadcastToClients({
type: 'swReady', autoInitialized: false, needsBundle: true }); } }

async function broadcastToClients(message: any): Promise<void> { const allClients = await (self as
any).clients.matchAll(); allClients.forEach(client => client.postMessage(message)); log('info',
'Broadcast message to clients', { type: message.type, clientCount: allClients. length }); }

#### 1.3 Update loadBundle Handler (lines 1257-1383)

Changes:

• Set isReady = true after successful bundle load • Broadcast ready message to all clients • Resolve
readyPromise

#### 1.4 Add Readiness Check Helper

Location: After helper functions (around line 500)

New function:

async function waitForReady(timeoutMs: number = 30000): Promise<void> { if (isReady) return;

if (!readyPromise) { readyPromise = new Promise(resolve => { readyResolve = resolve; }); }

return Promise.race([ readyPromise, new Promise((_, reject) => setTimeout(() => reject(new Error('SW
ready timeout')), timeoutMs) ) ]); }

#### 1.5 Update Message Handler (lines 714-730)

Changes:

• Add waitForReady call before processing operations • Handle timeout gracefully

New logic:

const allowedWhenUninitialized = ['init', 'loadBundle', 'initializeFromUrl', 'getServerUrl'];

if (!allowedWhenUninitialized.includes(message.type)) { try { await waitForReady(10000); // 10
second timeout } catch (error) { postResponse({ type: message.type, id: message.id, success: false,
error: 'VFS not initialized. Please load a bundle first.', }); return; } }

#### 1.6 Update Activate Handler (lines 436-460)

Changes:

• Send proper ready state based on auto-init status

New logic:

self.addEventListener('activate', event => { event.waitUntil( (async () => { await (self as
any).clients.claim();

      // Only send swReady if auto-init is already complete
      if (isReady) {
        const allClients = await (self as any).clients.matchAll();
        allClients.forEach(client => {
          client.postMessage({
            type: 'swReady',
            autoInitialized: true,
            needsBundle: false
          });
        });
      }
      // Otherwise, autoInitializeFromCache() will send it when done
    })()

); });

---

## Phase 2: Type Definitions

### File: /packages/host-web/src/types.ts

#### 2.1 Add New Message Types (after line 114)

Changes:

| { type: 'swReady'; autoInitialized: boolean; needsBundle?: boolean } | { type: 'swInitializing' }
| { type: 'needsReinit'; appSlug: string | null; reason: string };

---

## Phase 3: VFS Service Changes

### File: /tests/src/test-ui/vfs-service.ts

#### 3.1 Add Readiness Tracking (lines 16-18)

Changes:

export class VFSService { private serviceWorker: ServiceWorker | null = null; private initialized =
false; private serviceWorkerReady = false; private swReadyPromise: Promise<void> | null = null; //
... rest of properties

#### 3.2 Update Message Listener (lines 134-139)

Changes:

• Add handler for swReady message

New handler:

if (response.type === 'swReady') { console.log('[VFSService] Service worker ready:', response);
this.serviceWorkerReady = response.autoInitialized;

if (this.swReadyResolve) { this.swReadyResolve(); this.swReadyResolve = null; }

// If auto-initialization failed, we need to load bundle manually if (!response.autoInitialized &&
response.needsBundle) { console.log('[VFSService] Service worker needs bundle - will load on
initialize()'); }

return; }

#### 3.3 Add Wait for Ready Method

Location: After registerServiceWorker() (around line 207)

New method:

private async waitForServiceWorkerReady(): Promise<void> { if (this.serviceWorkerReady) { return; }

if (!this.swReadyPromise) { this.swReadyPromise = new Promise((resolve, reject) => {
this.swReadyResolve = resolve;

      setTimeout(() => {
        reject(new Error('Service worker ready timeout after 15s - likely initialization

failed')); }, 15000); }); }

return this.swReadyPromise; }

#### 3.4 Update Initialize Method (lines 209-298)

Changes:

• Wait for service worker ready before sending messages • Add retry logic with exponential backoff

New flow:

async initialize(manifestUrl: string, wsUrl: string): Promise<void> { console.log('[VFSService]
Starting initialization...');

await this.registerServiceWorker();

// CRITICAL: Wait for service worker to be ready console.log('[VFSService] Waiting for service
worker to be ready...'); try { await this.waitForServiceWorkerReady(15000);
console.log('[VFSService] Service worker is ready'); } catch (error) { console.warn('[VFSService]
Service worker ready timeout - will retry with bundle load'); }

// If service worker auto-initialized, we're done if (this.serviceWorkerReady) {
console.log('[VFSService] Service worker auto-initialized from cache'); this.initialized = true;
return; }

// Otherwise, load bundle with retry logic await this.loadBundleWithRetry(manifestUrl, wsUrl, 3); }

private async loadBundleWithRetry( manifestUrl: string, wsUrl: string, maxRetries: number ):
Promise<void> { for (let attempt = 1; attempt <= maxRetries; attempt++) { try { const response =
await fetch(manifestUrl); if (!response.ok) throw new Error(`HTTP ${response.status}`);

      const manifest = await response.arrayBuffer();
      const id = this.generateId();
      const manifestUrlObj = new URL(manifestUrl);
      const serverUrl = `${manifestUrlObj.protocol}//${manifestUrlObj.host}`;

      console.log(`[VFSService] Loading bundle (attempt ${attempt}/${maxRetries})...`);

      await this.sendMessage<void>({
        type: 'loadBundle',
        id,
        bundleBytes: manifest,
        serverUrl,
      } as any);

      console.log('[VFSService] Bundle loaded successfully');
      this.initialized = true;
      this.serviceWorkerReady = true;
      return;

    } catch (error) {
      console.warn(`[VFSService] Bundle load attempt ${attempt} failed:`, error);

      if (attempt < maxRetries) {
        const delay = Math.min(1000 * Math.pow(2, attempt - 1), 5000);
        console.log(`[VFSService] Retrying in ${delay}ms...`);
        await new Promise(resolve => setTimeout(resolve, delay));
      } else {
        throw error;
      }
    }

} }

#### 3.5 Update sendMessage (lines 304-347)

Changes:

• Check service worker readiness before sending • Add helpful error messages

New check:

private sendMessage<T>(message: VFSWorkerMessage & { id: string }): Promise<T> { if
(!this.serviceWorker) { return Promise.reject(new Error('Service worker not initialized')); }

if (!this.serviceWorkerReady && !['loadBundle', 'initializeFromUrl'].includes(message.type)) {
return Promise.reject(new Error('Service worker not ready yet - initialization in progress')); }

// ... rest of existing logic ... }

---

## Phase 4: Fixtures Helper

### File: /tests/tests/fixtures.ts

#### 4.1 Update setupTestWithServer (lines 241-266)

Changes:

• Add explicit wait for service worker ready

Enhanced function:

export async function setupTestWithServer( page: Page, serverInstance: ServerInstance,
additionalConfig?: Record<string, any> ): Promise<void> { const config = { port:
serverInstance.port, wsUrl: serverInstance.wsUrl, manifestUrl: serverInstance.manifestUrl,
...additionalConfig, };

await page.addInitScript({ content:
`       window.serverConfig = ${JSON.stringify(config)};       console.log('[TEST] Server config injected:', window.serverConfig);     `,
});

await page.goto('http://localhost:5173'); await page.waitForLoadState('load');

// CRITICAL: Wait for service worker ready message // Wait for service worker ready message
console.log('[TEST] Waiting for service worker ready...');

const swReady = await page.evaluate(() => { return new Promise<boolean>((resolve, reject) => { //
Check if already ready if ((window as any).\_\_swReady) { resolve(true); return; }

      const handler = (event: MessageEvent) => {
        if (event.data?.type === 'swReady') {
          navigator.serviceWorker.removeEventListener('message', handler);
          (window as any).__swReady = true;
          resolve(true);
        }
      };

      navigator.serviceWorker.addEventListener('message', handler);

      // Only timeout as fallback
      setTimeout(() => {
        navigator.serviceWorker.removeEventListener('message', handler);
        resolve(false);
      }, 15000);
    });

});

if (!swReady) { throw new Error('Service worker did not become ready within 15 seconds'); }

console.log('[TEST] Service worker is ready'); }

---

## Phase 5: Connection Manager Enhancement (Optional but Recommended)

### File: /tests/src/utils/connection-manager.ts

#### 5.1 Add Readiness Verification (lines 88-136)

Changes:

• Add verification step after connection creation • Implement timeout with helpful error

Enhanced createSingleConnection:

private async createSingleConnection(id: string): Promise<void> { const startTime = Date.now();

try { const context = await this.browser.newContext(); const page = await context.newPage();

    // Setup test with server
    await setupTestWithServer(page, this.serverInstance);

    // CRITICAL: Verify VFS service is actually ready before continuing
    const vfsReady = await page.evaluate(() => {
      return (window as any).vfsService?.isInitialized() === true;
    });

    if (!vfsReady) {
      console.warn(`[${id}] VFS not ready after setupTestWithServer, waiting...`);

      // Wait up to 10 seconds for VFS to be ready
      await page.waitForFunction(
        () => (window as any).vfsService?.isInitialized() === true,
        { timeout: 10000 }
      ).catch(() => {
        throw new Error(`VFS service not ready after 10 seconds for connection ${id}`);
      });
    }

    // Wait for VFS connection
    await waitForVFSConnection(page);

    const connectionTime = Date.now() - startTime;

    const connInfo: ConnectionInfo = {
      id,
      context,
      page,
      connectedAt: Date.now(),
      connectionTime,
      latency: [],
      operationCount: 0,
      errorCount: 0,
      isHealthy: true,
    };

    this.connections.set(id, connInfo);

    // Setup error monitoring
    page.on('pageerror', error => {
      console.error(`[${id}] Page error:`, error.message);
      connInfo.errorCount++;
      connInfo.isHealthy = false;
    });

} catch (error) { console.error(`Failed to create connection ${id}:`, error); throw error; } }

---

## Phase 6: Testing & Validation

### 6.1 Unit Tests

Create tests for:

• Service worker readiness signaling • Race condition scenarios • Timeout handling • Retry logic

### 6.2 Integration Tests

Update distributed stress test:

• Add readiness verification • Monitor error rates (should be <5%) • Verify connection scaling (10 →
50+ connections)

### 6.3 Validation Metrics

Success criteria:

• ✅ Error rate < 5% across all phases • ✅ All 50+ connections establish successfully • ✅ No "VFS
not initialized" errors after first 30 seconds • ✅ Gradual load tests continue to work • ✅
Auto-initialization from cache works reliably

---

## Implementation Order

1. Phase 1 (Critical): Service worker changes - fixes core race condition
2. Phase 2 (Critical): Type definitions - enables communication
3. Phase 3 (Critical): VFS service changes - implements waiting logic
4. Phase 4 (Important): Fixtures helper - ensures tests wait properly
5. Phase 5 (Recommended): Connection manager - adds verification
6. Phase 6 (Required): Testing - validates fixes

---

## Expected Impact

Metric │ Before │ After
─────────────────────────────────┼─────────────────────────────────┼────────────────────────────────
Error Rate │ 12.5% → │ < (WARMUP) │ 1.0% │ 2% │ │ Error Rate │ 12.5% → │ < (SUSTAINED) │ 1.8% │ 2% │
│ Error Rate │ 250% → │ < (COOLDOWN) │ 14.3% │ 5% │ │ Connection │ 10/50 │ 50/50 Success │ (20%) │
(100%) │ │ Time to │ Variable │ < 5 Ready │ │ seconds │ │ Auto-init │ ~ │ > Success │ 40% │ 95% │ │

---

## Risk Mitigation

1. Backward Compatibility: All changes are additive - existing tests continue to work
2. Graceful Degradation: Timeout logic ensures system doesn't hang
3. Clear Error Messages: Users get actionable feedback
4. Incremental Rollout: Can deploy phase-by-phase
5. Rollback Plan: Changes are isolated and can be reverted easily
