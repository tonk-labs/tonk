import {
  expect,
  setupTestWithServer,
  test,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Bundle Upload', () => {
  test('should upload a bundle successfully', async ({
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

    const uploadedBundleId = await page.waitForFunction(
      () => (window as any).uploadedBundleId,
      { timeout: 10000 }
    );

    expect(uploadedBundleId).toBeTruthy();

    const bundleId = await page.evaluate(
      () => (window as any).uploadedBundleId
    );
    console.log(`Uploaded bundle ID: ${bundleId}`);

    expect(bundleId).toBeTruthy();
    expect(typeof bundleId).toBe('string');

    await context.close();
  });

  test('should handle multiple bundle uploads', async ({
    browser,
    serverInstance,
  }) => {
    const context = await browser.newContext();
    const page = await context.newPage();

    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const bundleIds: string[] = [];

    for (let i = 0; i < 3; i++) {
      await page.getByTestId('increment-btn').click();
      await page.getByTestId('upload-bundle-btn').click();

      await page.waitForFunction(() => (window as any).uploadedBundleId, {
        timeout: 10000,
      });

      const bundleId = await page.evaluate(
        () => (window as any).uploadedBundleId
      );
      bundleIds.push(bundleId);

      await page.evaluate(() => {
        delete (window as any).uploadedBundleId;
      });

      await page.waitForTimeout(1000);
    }

    expect(bundleIds.length).toBe(3);

    await context.close();
  });
});
