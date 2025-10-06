import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Bundle Download', () => {
  test('should download an uploaded bundle', async ({
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

    await page.getByTestId('upload-bundle-btn').click();

    await page.waitForFunction(() => (window as any).uploadedBundleId, {
      timeout: 10000,
    });

    const bundleId = await page.evaluate(
      () => (window as any).uploadedBundleId
    );

    await page.getByTestId('download-bundle-btn').click();

    const downloadedBundle = await page.waitForFunction(
      () => (window as any).downloadedBundle,
      { timeout: 10000 }
    );

    expect(downloadedBundle).toBeTruthy();

    const bundleData = await page.evaluate(
      () => (window as any).downloadedBundle
    );

    expect(bundleData).toBeTruthy();
    console.log(`Downloaded bundle for ID: ${bundleId}`);

    await context.close();
  });

  test('should download bundle from different client', async ({
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

    await page1.getByTestId('increment-btn').click();
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

    const downloadedOnPage2 = await page2.waitForFunction(
      () => (window as any).downloadedBundle,
      { timeout: 10000 }
    );

    expect(downloadedOnPage2).toBeTruthy();

    const bundleData = await page2.evaluate(
      () => (window as any).downloadedBundle
    );
    expect(bundleData).toBeTruthy();

    await context1.close();
    await context2.close();
  });
});
