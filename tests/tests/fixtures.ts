import { test as base, Page } from '@playwright/test';
import { serverManager } from '../src/server/server-manager';
import { ServerInstance } from '../src/test-ui/types';

// Extend basic test with server fixture
export const test = base.extend<{
  serverInstance: ServerInstance;
}>({
  serverInstance: async ({}, use, testInfo) => {
    // Start a unique server for this test
    const testId = testInfo.testId;
    console.log(`Starting server for test: ${testInfo.title}`);

    let server: ServerInstance;
    try {
      server = await serverManager.startServer(testId);
      console.log(
        `Server started on port ${server.port} for test: ${testInfo.title}`
      );

      // Verify server is actually responding
      await verifyServerHealth(server);
      console.log(`Server health check passed for test: ${testInfo.title}`);
    } catch (error) {
      console.error(
        `Failed to start server for test: ${testInfo.title}`,
        error
      );
      throw error;
    }

    // Use the server in the test
    await use(server);

    // Cleanup after test
    console.log(
      `Stopping server on port ${server.port} for test: ${testInfo.title}`
    );
    try {
      await serverManager.stopServer(testId);
      console.log(`Server cleanup completed for test: ${testInfo.title}`);
    } catch (error) {
      console.error(
        `Error during server cleanup for test: ${testInfo.title}`,
        error
      );
    }
  },
});

// Add a page cleanup fixture to unregister service workers and clear IndexedDB
export const testWithCleanup = test.extend<{ cleanupPage: Page }>({
  cleanupPage: async ({ page }, use) => {
    await use(page);

    // Cleanup after test
    try {
      // Unregister service workers
      await page.evaluate(async () => {
        const registrations = await navigator.serviceWorker.getRegistrations();
        await Promise.all(registrations.map(r => r.unregister()));
      });

      // Clear IndexedDB databases
      await page.evaluate(async () => {
        const dbs = await indexedDB.databases();
        await Promise.all(
          dbs.map(db => {
            if (db.name) {
              return new Promise<void>((resolve, reject) => {
                const request = indexedDB.deleteDatabase(db.name!);
                request.onsuccess = () => resolve();
                request.onerror = () => reject(request.error);
              });
            }
            return Promise.resolve();
          })
        );
      });

      console.log(
        'Page cleanup completed: service workers unregistered, IndexedDB cleared'
      );
    } catch (error) {
      console.error('Error during page cleanup:', error);
    }
  },
});

/**
 * Verify that the server is healthy and responding correctly
 */
async function verifyServerHealth(server: ServerInstance): Promise<void> {
  const maxRetries = 10;
  const retryDelay = 1000;

  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      // Check basic health endpoint
      const healthResponse = await fetch(`http://localhost:${server.port}/`);
      if (!healthResponse.ok) {
        throw new Error(`Health check failed: ${healthResponse.status}`);
      }

      // Check manifest endpoint - this is crucial for Tonk initialization
      const manifestResponse = await fetch(server.manifestUrl);
      if (!manifestResponse.ok) {
        throw new Error(`Manifest endpoint failed: ${manifestResponse.status}`);
      }

      // Verify we can get the manifest as bytes
      const manifestBytes = await manifestResponse.arrayBuffer();
      if (manifestBytes.byteLength === 0) {
        throw new Error('Manifest is empty');
      }

      console.log(
        `Server health verified - manifest size: ${manifestBytes.byteLength} bytes`
      );
      return;
    } catch (error) {
      if (attempt === maxRetries) {
        throw new Error(
          `Server health check failed after ${maxRetries} attempts: ${error}`
        );
      }

      console.log(
        `Health check attempt ${attempt}/${maxRetries} failed, retrying in ${retryDelay}ms...`
      );
      await new Promise(resolve => setTimeout(resolve, retryDelay));
    }
  }
}

/**
 * Helper function to setup a test page with server configuration
 */
export async function setupTestWithServer(
  page: Page,
  serverInstance: ServerInstance,
  additionalConfig?: Record<string, any>
): Promise<void> {
  // Inject server configuration into the page before navigation
  const config = {
    port: serverInstance.port,
    wsUrl: serverInstance.wsUrl,
    manifestUrl: serverInstance.manifestUrl,
    ...additionalConfig,
  };

  await page.addInitScript({
    content: `
      window.serverConfig = ${JSON.stringify(config)};
      console.log('Server config injected:', window.serverConfig);
    `,
  });

  // Navigate to the test UI
  await page.goto('http://localhost:5173');

  // Wait for the page to load
  await page.waitForLoadState('networkidle');
}

/**
 * Wait for the VFS connection to be established
 */
export async function waitForVFSConnection(
  page: Page,
  timeout: number = 30000
): Promise<void> {
  await page.waitForFunction(
    () => {
      const status = document.querySelector(
        '[data-testid="connection-status"]'
      );
      return status?.textContent?.includes('Connected');
    },
    { timeout }
  );
}

export { expect } from '@playwright/test';
