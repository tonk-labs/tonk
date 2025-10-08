import { Page } from '@playwright/test';
import { serverManager } from '../../src/server/server-manager';
import { ServerInstance } from '../../src/test-ui/types';

export async function clearIndexedDB(page: Page): Promise<void> {
  try {
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
      console.log('[Test] IndexedDB cleared');
    });
  } catch (error) {
    console.log('[Test] IndexedDB clear skipped (page not navigated yet)');
  }
}

export async function stopRelayServer(
  serverInstance: ServerInstance
): Promise<void> {
  console.log(
    `[Test] Stopping relay server on port ${serverInstance.port} to simulate offline`
  );
  await serverManager.stopServer(serverInstance.testId);
}

export async function restartRelayServer(
  serverInstance: ServerInstance
): Promise<ServerInstance> {
  console.log('[Test] Restarting relay server to simulate reconnection');
  const server = await serverManager.startServer(serverInstance.testId);
  console.log(
    `[Test] Relay server restarted and healthy on port ${server.port}`
  );
  return server;
}

export async function waitForReconnection(
  page: Page,
  timeout: number = 10000
): Promise<void> {
  const startTime = Date.now();

  while (Date.now() - startTime < timeout) {
    try {
      const connected = await page.evaluate(() => {
        const vfsService = (window as any).__vfsService;
        return vfsService?.isInitialized?.() === true;
      });

      if (connected) {
        console.log('[Test] Reconnection detected');
        return;
      }
    } catch (error) {
      // Continue waiting
    }

    await page.waitForTimeout(100);
  }

  throw new Error('Reconnection timeout');
}

export async function verifyFileContent(
  page: Page,
  filePath: string,
  expectedContent: any
): Promise<void> {
  const result = await page.evaluate(
    async ({ path, expected }) => {
      const vfsService = (window as any).__vfsService;
      if (!vfsService) {
        throw new Error('VFS service not available');
      }

      try {
        const doc = await vfsService.readFile(path);
        return {
          success: true,
          content: doc.content,
          expected,
        };
      } catch (error) {
        return {
          success: false,
          error: error instanceof Error ? error.message : String(error),
        };
      }
    },
    { path: filePath, expected: expectedContent }
  );

  if (!result.success) {
    throw new Error(`Failed to read ${filePath}: ${(result as any).error}`);
  }

  if (JSON.stringify(result.content) !== JSON.stringify(expectedContent)) {
    throw new Error(
      `Content mismatch for ${filePath}:\nExpected: ${JSON.stringify(expectedContent)}\nGot: ${JSON.stringify(result.content)}`
    );
  }
}

export async function createFile(
  page: Page,
  filePath: string,
  content: any
): Promise<void> {
  await page.evaluate(
    async ({ path, data }) => {
      const vfsService = (window as any).__vfsService;
      if (!vfsService) {
        throw new Error('VFS service not available');
      }

      await vfsService.writeFile(path, { content: data }, true);
      console.log('[Test] Created file:', path);
    },
    { path: filePath, data: content }
  );
}

export async function updateFile(
  page: Page,
  filePath: string,
  content: any
): Promise<void> {
  await page.evaluate(
    async ({ path, data }) => {
      const vfsService = (window as any).__vfsService;
      if (!vfsService) {
        throw new Error('VFS service not available');
      }

      await vfsService.writeFile(path, { content: data }, false);
      console.log('[Test] Updated file:', path);
    },
    { path: filePath, data: content }
  );
}

export async function deleteFile(page: Page, filePath: string): Promise<void> {
  await page.evaluate(async path => {
    const vfsService = (window as any).__vfsService;
    if (!vfsService) {
      throw new Error('VFS service not available');
    }

    await vfsService.deleteFile(path);
    console.log('[Test] Deleted file:', path);
  }, filePath);
}

export async function fileExists(
  page: Page,
  filePath: string
): Promise<boolean> {
  return await page.evaluate(async path => {
    const vfsService = (window as any).__vfsService;
    if (!vfsService) {
      throw new Error('VFS service not available');
    }

    return await vfsService.exists(path);
  }, filePath);
}

export async function listDirectory(
  page: Page,
  directoryPath: string
): Promise<any[]> {
  return await page.evaluate(async path => {
    const vfsService = (window as any).__vfsService;
    if (!vfsService) {
      throw new Error('VFS service not available');
    }

    return await vfsService.listDirectory(path);
  }, directoryPath);
}

export async function getIndexedDBDatabases(page: Page): Promise<string[]> {
  try {
    return await page.evaluate(async () => {
      const dbs = await indexedDB.databases();
      return dbs.map(db => db.name || '').filter(name => name.length > 0);
    });
  } catch (error) {
    console.log('[Test] Unable to access IndexedDB (page not navigated yet)');
    return [];
  }
}

export async function waitForSync(
  page: Page,
  timeout: number = 5000
): Promise<void> {
  const startTime = Date.now();

  const initialStats = await page.evaluate(() => {
    const vfsService = (window as any).__vfsService;
    return vfsService?.getOperationStats?.() || { totalOperations: 0 };
  });

  const initialOps = initialStats.totalOperations || 0;

  let lastOpCount = initialOps;
  let stableTime = 0;
  const stabilityThreshold = 100;

  while (Date.now() - startTime < timeout) {
    await page.waitForTimeout(50);

    const currentStats = await page.evaluate(() => {
      const vfsService = (window as any).__vfsService;
      return vfsService?.getOperationStats?.() || { totalOperations: 0 };
    });

    const currentOps = currentStats.totalOperations || 0;

    if (currentOps === lastOpCount) {
      stableTime += 50;
      if (stableTime >= stabilityThreshold) {
        console.log(
          `[Test] Sync complete (${currentOps - initialOps} operations)`
        );
        return;
      }
    } else {
      stableTime = 0;
      lastOpCount = currentOps;
    }
  }

  console.warn('[Test] Sync timeout - operations may still be pending');
}
