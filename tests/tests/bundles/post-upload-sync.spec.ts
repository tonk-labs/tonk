import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Post-Upload Sync', () => {
  test('should maintain websocket sync after bundle upload', async ({
    browser,
    serverInstance,
  }) => {
    console.log(
      `CRITICAL TEST: Testing sync after bundle upload on port ${serverInstance.port}`
    );

    const context1 = await browser.newContext();
    const context2 = await browser.newContext();

    const page1 = await context1.newPage();
    const page2 = await context2.newPage();

    page1.on('console', msg => console.log(`[Page1] ${msg.text()}`));
    page2.on('console', msg => console.log(`[Page2] ${msg.text()}`));

    page1.on('pageerror', err => console.error(`[Page1 Error] ${err.message}`));
    page2.on('pageerror', err => console.error(`[Page2 Error] ${err.message}`));

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

    console.log('[CRITICAL] Testing sync BEFORE bundle upload');
    await page1.getByTestId('increment-btn').click();

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('1');
      },
      { timeout: 10000 }
    );

    let counter1 = await page1.getByTestId('counter-value').textContent();
    let counter2 = await page2.getByTestId('counter-value').textContent();

    expect(counter1).toContain('1');
    expect(counter2).toContain('1');

    console.log('[CRITICAL] Uploading bundle from page1');
    await page1.getByTestId('upload-bundle-btn').click();

    await page1.waitForFunction(() => (window as any).uploadedBundleId, {
      timeout: 10000,
    });

    const bundleId = await page1.evaluate(
      () => (window as any).uploadedBundleId
    );
    console.log(`[CRITICAL] Bundle uploaded: ${bundleId}`);

    console.log('[CRITICAL] Downloading bundle on page2');
    await page2.evaluate(async bundleIdParam => {
      (window as any).__targetBundleId = bundleIdParam;
    }, bundleId);

    await page2.getByTestId('download-bundle-btn').click();

    await page2.waitForFunction(() => (window as any).downloadedBundle, {
      timeout: 10000,
    });

    console.log('[CRITICAL] Bundle downloaded on page2');

    console.log(
      '[CRITICAL] Testing sync AFTER bundle operations - incrementing on page1'
    );
    await page1.getByTestId('increment-btn').click();

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 5000 }
    );

    console.log('[CRITICAL] Waiting for sync to page2...');
    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 15000 }
    );

    counter1 = await page1.getByTestId('counter-value').textContent();
    counter2 = await page2.getByTestId('counter-value').textContent();

    console.log(
      `[CRITICAL] Final counters - Page1: ${counter1}, Page2: ${counter2}`
    );

    expect(counter1).toContain('2');
    expect(counter2).toContain('2');

    console.log('[CRITICAL] ✅ SYNC STILL WORKS AFTER BUNDLE OPERATIONS!');

    await context1.close();
    await context2.close();
  });

  test('should sync bidirectionally after bundle operations', async ({
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

    await page1.getByTestId('upload-bundle-btn').click();

    await page1.waitForFunction(() => (window as any).uploadedBundleId, {
      timeout: 10000,
    });

    const bundleId = await page1.evaluate(
      () => (window as any).uploadedBundleId
    );

    await page2.evaluate(async bundleIdParam => {
      (window as any).__targetBundleId = bundleIdParam;
    }, bundleId);

    await page2.getByTestId('download-bundle-btn').click();

    await page2.waitForFunction(() => (window as any).downloadedBundle, {
      timeout: 10000,
    });

    await page2.getByTestId('increment-btn').click();

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 15000 }
    );

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('2');
      },
      { timeout: 15000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    expect(counter1).toContain('2');
    expect(counter2).toContain('2');

    await context1.close();
    await context2.close();
  });

  test('should handle multiple bundle operations without breaking sync', async ({
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

    for (let i = 1; i <= 3; i++) {
      console.log(`[Round ${i}] Incrementing and uploading bundle`);

      await page1.getByTestId('increment-btn').click();

      await page2.waitForFunction(
        (expectedValue: number) => {
          const counter = document.querySelector(
            '[data-testid="counter-value"]'
          );
          return counter?.textContent?.includes(expectedValue.toString());
        },
        i,
        { timeout: 10000 }
      );

      await page1.getByTestId('upload-bundle-btn').click();

      await page1.waitForFunction(() => (window as any).uploadedBundleId, {
        timeout: 10000,
      });

      const bundleId = await page1.evaluate(
        () => (window as any).uploadedBundleId
      );

      await page2.evaluate(async bundleIdParam => {
        (window as any).__targetBundleId = bundleIdParam;
      }, bundleId);

      await page2.getByTestId('download-bundle-btn').click();

      await page2.waitForFunction(() => (window as any).downloadedBundle, {
        timeout: 10000,
      });

      await page1.evaluate(() => {
        delete (window as any).uploadedBundleId;
      });
      await page2.evaluate(() => {
        delete (window as any).downloadedBundle;
      });

      console.log(`[Round ${i}] Bundle operations complete, testing sync...`);
    }

    await page1.getByTestId('increment-btn').click();

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('4');
      },
      { timeout: 15000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    expect(counter1).toContain('4');
    expect(counter2).toContain('4');

    console.log('[CRITICAL] ✅ Sync works after multiple bundle operations!');

    await context1.close();
    await context2.close();
  });
});
