import {
  expect,
  setupTestWithServer,
  test,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Multi-Client WebSocket Sync', () => {
  test('should sync state between two clients using sync middleware', async ({
    browser,
    serverInstance,
  }) => {
    console.log(
      `Test running with relay server on port ${serverInstance.port}`
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

    await waitForVFSConnection(page2, 60000);

    await page1.getByTestId('increment-btn').click();

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('1');
      },
      { timeout: 5000 }
    );

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        return counter?.textContent?.includes('1');
      },
      { timeout: 10000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    expect(counter1).toContain('1');
    expect(counter2).toContain('1');

    await context1.close();
    await context2.close();
  });
});
