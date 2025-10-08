import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';
import {
  clearIndexedDB,
  stopRelayServer,
  restartRelayServer,
  waitForReconnection,
  verifyFileContent,
  createFile,
  updateFile,
  fileExists,
  getIndexedDBDatabases,
  waitForSync,
} from './indexeddb-helpers';

test.describe('IndexedDB Storage', () => {
  test('should persist data to IndexedDB and survive relay disconnect', async ({
    persistentContext,
    serverInstance,
  }) => {
    console.log(
      `Test running with relay server on port ${serverInstance.port}`
    );

    const page = await persistentContext.newPage();
    page.on('console', msg => console.log(`[Page] ${msg.text()}`));

    await setupTestWithServer(page, serverInstance);
    await clearIndexedDB(page);
    await waitForVFSConnection(page);

    await page.getByTestId('enable-sync-btn').click();
    await page.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );

    await createFile(page, '/test.txt', { message: 'Hello IndexedDB!' });
    await waitForSync(page, 2000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    const dbs = await getIndexedDBDatabases(page);
    console.log('[Test] IndexedDB databases:', dbs);
    expect(dbs.length).toBeGreaterThan(0);

    await stopRelayServer(serverInstance);

    const existsWhileOffline = await fileExists(page, '/test.txt');
    expect(existsWhileOffline).toBe(true);

    await verifyFileContent(page, '/test.txt', { message: 'Hello IndexedDB!' });

    await page.close();
  });

  test('should make offline changes and sync after server restart', async ({
    persistentContext,
    serverInstance,
  }) => {
    const page = await persistentContext.newPage();
    page.on('console', msg => console.log(`[Page1] ${msg.text()}`));

    await setupTestWithServer(page, serverInstance);
    await clearIndexedDB(page);
    await waitForVFSConnection(page);

    await page.getByTestId('enable-sync-btn').click();

    await createFile(page, '/online.txt', { message: 'Created online' });

    // TODO: this should actually work, manual timeout shouldn't be necessary
    await waitForSync(page, 2000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    await stopRelayServer(serverInstance);

    await updateFile(page, '/online.txt', { message: 'Updated offline' });
    await createFile(page, '/offline.txt', { message: 'Created offline' });

    await waitForSync(page, 2000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    const newServer = await restartRelayServer(serverInstance);

    await waitForReconnection(page, 1000);

    const page2 = await persistentContext.newPage();
    page2.on('console', msg => console.log(`[Page2] ${msg.text()}`));

    const page2Config = {
      port: newServer.port,
      wsUrl: newServer.wsUrl,
      manifestUrl: newServer.manifestUrl,
    };

    await page2.addInitScript({
      content: `window.serverConfig = ${JSON.stringify(page2Config)};`,
    });

    await page2.goto('http://localhost:5173');
    await page2.waitForLoadState('load');
    await waitForVFSConnection(page2);

    await waitForSync(page2, 3000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    const exists1 = await fileExists(page2, '/online.txt');
    const exists2 = await fileExists(page2, '/offline.txt');

    expect(exists1).toBe(true);
    expect(exists2).toBe(true);

    await verifyFileContent(page2, '/online.txt', {
      message: 'Updated offline',
    });
    await verifyFileContent(page2, '/offline.txt', {
      message: 'Created offline',
    });

    await page.close();
    await page2.close();
  });

  test('should support cold start from IndexedDB after page reload', async ({
    persistentContext,
    serverInstance,
  }) => {
    const page = await persistentContext.newPage();
    page.on('console', msg => console.log(`[Session1] ${msg.text()}`));

    await clearIndexedDB(page);
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    await page.getByTestId('enable-sync-btn').click();

    await createFile(page, '/persistent.txt', {
      message: 'Persisted data',
      version: 1,
    });

    await waitForSync(page, 1000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    await stopRelayServer(serverInstance);

    await updateFile(page, '/persistent.txt', {
      message: 'Persisted data',
      version: 2,
    });

    await waitForSync(page, 1000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    await page.reload();
    await page.waitForLoadState('load');

    await waitForSync(page, 1000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    const exists = await fileExists(page, '/persistent.txt');
    expect(exists).toBe(true);

    await verifyFileContent(page, '/persistent.txt', {
      message: 'Persisted data',
      version: 2,
    });

    const newServer = await restartRelayServer(serverInstance);
    await waitForReconnection(page, 15000);

    const page2 = await persistentContext.newPage();
    const page2Config = {
      port: newServer.port,
      wsUrl: newServer.wsUrl,
      manifestUrl: newServer.manifestUrl,
    };

    await page2.addInitScript({
      content: `window.serverConfig = ${JSON.stringify(page2Config)};`,
    });

    await page2.goto('http://localhost:5173');
    await page2.waitForLoadState('load');
    await waitForVFSConnection(page2);
    await waitForSync(page2, 3000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    await verifyFileContent(page2, '/persistent.txt', {
      message: 'Persisted data',
      version: 2,
    });

    await page2.close();
  });

  test('should handle multiple disconnect/reconnect cycles', async ({
    persistentContext,
    serverInstance,
  }) => {
    const page = await persistentContext.newPage();
    page.on('console', msg => console.log(`[Page] ${msg.text()}`));

    await clearIndexedDB(page);
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    await page.getByTestId('enable-sync-btn').click();

    await createFile(page, '/cycle-test.txt', { cycle: 0 });
    await waitForSync(page, 1000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    let currentServer = serverInstance;

    for (let i = 1; i <= 3; i++) {
      console.log(`[Test] Starting cycle ${i}`);

      await stopRelayServer(currentServer);

      await updateFile(page, '/cycle-test.txt', { cycle: i });
      await new Promise(resolve => setTimeout(resolve, 1000));

      currentServer = await restartRelayServer(currentServer);
      await waitForReconnection(page, 15000);
    }

    await new Promise(resolve => setTimeout(resolve, 1000));
    await verifyFileContent(page, '/cycle-test.txt', { cycle: 3 });

    const page2 = await persistentContext.newPage();
    const page2Config = {
      port: currentServer.port,
      wsUrl: currentServer.wsUrl,
      manifestUrl: currentServer.manifestUrl,
    };

    await page2.addInitScript({
      content: `window.serverConfig = ${JSON.stringify(page2Config)};`,
    });

    await page2.goto('http://localhost:5173');
    await page2.waitForLoadState('load');
    await waitForVFSConnection(page2);
    await waitForSync(page2, 3000);
    await new Promise(resolve => setTimeout(resolve, 1000));

    await verifyFileContent(page2, '/cycle-test.txt', { cycle: 3 });

    await page.close();
    await page2.close();
  });
});
