import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Connection Resilience', () => {
  test('should reconnect after WebSocket disconnect', async ({
    browser,
    serverInstance,
  }) => {
    console.log(
      `Test running with relay server on port ${serverInstance.port}`
    );

    const context = await browser.newContext();
    const page = await context.newPage();

    page.on('console', msg => console.log(`[Page] ${msg.text()}`));
    page.on('pageerror', err => console.error(`[Page Error] ${err.message}`));

    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    await page.getByTestId('enable-sync-btn').click();
    await page.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );

    await page.getByTestId('increment-btn').click();

    await page.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('1');
      },
      { timeout: 5000 }
    );

    await page.evaluate(() => {
      const vfsService = (window as any).__vfsService;
      if (vfsService?.close) {
        vfsService.close();
        console.log('Closed VFS connection');
      }
    });

    await page.waitForTimeout(2000);

    await page.evaluate(async () => {
      const config = (window as any).__testConfig;
      const { initVFSService } = await import('./vfs-service.js');
      const newVfsService = await initVFSService(
        config.relayUrl,
        config.storageAdapterId
      );
      (window as any).__vfsService = newVfsService;
      console.log('Reconnected VFS');
    });

    await waitForVFSConnection(page);

    await page.getByTestId('increment-btn').click();

    await page.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 10000 }
    );

    const counter = await page.getByTestId('counter-value').textContent();
    expect(counter).toContain('2');

    await context.close();
  });

  test('should recover state after reconnection', async ({
    browser,
    serverInstance,
  }) => {
    const context1 = await browser.newContext();
    const context2 = await browser.newContext();

    const page1 = await context1.newPage();
    const page2 = await context2.newPage();

    await setupTestWithServer(page1, serverInstance);
    await setupTestWithServer(page2, serverInstance);

    await waitForVFSConnection(page1);
    await waitForVFSConnection(page2);

    await page1.getByTestId('enable-sync-btn').click();
    await page2.getByTestId('enable-sync-btn').click();

    await page1.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );
    await page2.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );

    await page1.getByTestId('increment-btn').click();

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('1');
      },
      { timeout: 10000 }
    );

    await page2.evaluate(() => {
      const vfsService = (window as any).__vfsService;
      if (vfsService?.close) {
        vfsService.close();
        console.log('Page2: Closed VFS connection');
      }
    });

    await page1.getByTestId('increment-btn').click();

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 5000 }
    );

    await page2.evaluate(async () => {
      const config = (window as any).__testConfig;
      const { initVFSService } = await import('./vfs-service.js');
      const newVfsService = await initVFSService(
        config.relayUrl,
        config.storageAdapterId
      );
      (window as any).__vfsService = newVfsService;
      console.log('Page2: Reconnected VFS');
    });

    await waitForVFSConnection(page2);

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 10000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    expect(counter1).toContain('2');
    expect(counter2).toContain('2');

    await context1.close();
    await context2.close();
  });
});
