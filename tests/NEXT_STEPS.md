# Next Steps for Relay Playwright Tests

## ✅ Completed

1. ✅ Created full directory structure
2. ✅ Created package.json with all dependencies
3. ✅ Created Relay Server Manager for test isolation
4. ✅ Created Test UI with VFS service and Zustand sync middleware
5. ✅ Created Playwright fixtures and configuration
6. ✅ Created TypeScript and Vite configurations
7. ✅ Created example multi-client sync test

## 🚀 To Complete the Setup

### 1. Install Dependencies

```bash
cd /Users/jackdouglas/tonk/tonk/packages/relay/playwright-tests
pnpm install
```

This will install:

- Playwright and test runner
- React and React DOM
- Vite and build tools
- Zustand for state management
- @tonk/core package
- All TypeScript types

### 2. Install Playwright Browsers

```bash
pnpm exec playwright install
```

### 3. Create Additional Test Files

You should create test files in the following directories:

#### `tests/sync/` - WebSocket Synchronization Tests

- **multi-client-sync.spec.ts** (✅ Already created as an example)
- **race-conditions.spec.ts** - Test concurrent writes from multiple clients
- **connection-resilience.spec.ts** - Test reconnection and state recovery

#### `tests/bundles/` - Bundle Operation Tests

- **bundle-upload.spec.ts** - Test bundle upload functionality
- **bundle-download.spec.ts** - Test bundle download and manifest retrieval
- **post-upload-sync.spec.ts** - **CRITICAL**: Test the reported issue where websocket sync breaks
  after accessing uploaded bundles

#### `tests/stress/` - Stress and Performance Tests

- **many-clients.spec.ts** - Test with 10+ concurrent clients
- **large-state.spec.ts** - Test syncing large state objects
- **rapid-updates.spec.ts** - Test rapid state changes

### 4. Run the Tests

```bash
# Run all tests
pnpm test

# Run specific test suites
pnpm test:sync
pnpm test:bundles

# Run with UI for debugging
pnpm test:ui

# Run in headed mode to see browsers
pnpm test:headed
```

### 5. Test the Dev Server

Before running tests, verify the test UI works:

```bash
pnpm run dev
# Visit http://localhost:5174
```

## 📝 Writing Tests - Key Patterns

### Basic Test Structure

```typescript
import { test, expect, setupTestWithServer, waitForVFSConnection } from '../fixtures';

test('test description', async ({ browser, relayServer }) => {
  // Each test gets its own relay server instance on a random port
  const context = await browser.newContext();
  const page = await context.newPage();

  // Setup page with server config
  await setupTestWithServer(page, relayServer);

  // Wait for VFS to connect
  await waitForVFSConnection(page);

  // Your test logic here

  await context.close();
});
```

### Testing Multi-Client Sync

```typescript
test('sync between clients', async ({ browser, relayServer }) => {
  const context1 = await browser.newContext();
  const context2 = await browser.newContext();

  const page1 = await context1.newPage();
  const page2 = await context2.newPage();

  await setupTestWithServer(page1, relayServer);
  await setupTestWithServer(page2, relayServer);

  // Enable sync on both clients
  await page1.getByTestId('enable-sync-btn').click();
  await page2.getByTestId('enable-sync-btn').click();

  // Make changes on page1
  await page1.getByTestId('increment-btn').click();

  // Wait for sync to page2
  await page2.waitForFunction(() => {
    const counter = document.querySelector('[data-testid="counter-value"]');
    return counter?.textContent?.includes('1');
  });

  // Verify sync
  const counter2 = await page2.getByTestId('counter-value').textContent();
  expect(counter2).toContain('1');
});
```

### Testing Bundle Operations

```typescript
test('bundle upload and download', async ({ browser, relayServer }) => {
  const page = await browser.newPage();
  await setupTestWithServer(page, relayServer);
  await waitForVFSConnection(page);

  // Upload bundle
  await page.getByTestId('upload-bundle-btn').click();

  // Wait for upload to complete
  await page.waitForFunction(() => (window as any).uploadedBundleId);

  // Download bundle
  await page.getByTestId('download-bundle-btn').click();

  // Verify download
  const downloaded = await page.evaluate(() => (window as any).downloadedBundle);
  expect(downloaded).toBeTruthy();
});
```

## 🐛 Testing the Critical Bug

The main issue you mentioned is **websocket sync breaking after accessing uploaded bundles**. Create
a test like:

```typescript
test('websocket sync works after bundle upload', async ({ browser, relayServer }) => {
  const page1 = await browser.newContext().newPage();
  const page2 = await browser.newContext().newPage();

  await setupTestWithServer(page1, relayServer);
  await setupTestWithServer(page2, relayServer);

  // Enable sync
  await page1.getByTestId('enable-sync-btn').click();
  await page2.getByTestId('enable-sync-btn').click();

  // Verify sync works BEFORE upload
  await page1.getByTestId('increment-btn').click();
  await page2.waitForFunction(() => {
    const counter = document.querySelector('[data-testid="counter-value"]');
    return counter?.textContent?.includes('1');
  });

  // Upload bundle
  await page1.getByTestId('upload-bundle-btn').click();
  await page1.waitForFunction(() => (window as any).uploadedBundleId);

  // Download bundle on page2
  await page2.getByTestId('download-bundle-btn').click();

  // **CRITICAL TEST**: Verify sync STILL works after bundle operations
  await page1.getByTestId('increment-btn').click();
  await page2.waitForFunction(
    () => {
      const counter = document.querySelector('[data-testid="counter-value"]');
      return counter?.textContent?.includes('2');
    },
    { timeout: 10000 }
  );

  const counter2 = await page2.getByTestId('counter-value').textContent();
  expect(counter2).toContain('2');
});
```

## 📊 What This Infrastructure Provides

- **Isolated Tests**: Each test gets its own relay server on a random port
- **Real VFS**: Uses actual IndexedDB storage via @tonk/core
- **Real Sync**: Uses actual Zustand sync middleware from the latergram example
- **Real Bundle Operations**: Tests actual bundle upload/download flows
- **Parallel Execution**: Tests can run concurrently without interference
- **Debugging UI**: Test UI at localhost:5174 for manual testing

## 🎯 Priority Order

1. **Install dependencies** (pnpm install)
2. **Run the example test** to verify setup
3. **Create post-upload-sync test** to reproduce the critical bug
4. **Add more sync tests** for comprehensive coverage
5. **Add bundle operation tests**
6. **Add stress tests** for performance validation

Good luck! The infrastructure is ready - you just need to install dependencies and write the
specific test scenarios.
