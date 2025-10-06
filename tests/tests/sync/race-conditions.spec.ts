import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Race Conditions - Concurrent Writes', () => {
  test('should handle concurrent writes from multiple clients', async ({
    browser,
    serverInstance,
  }) => {
    console.log(
      `Test running with relay server on port ${serverInstance.port}`
    );

    const context1 = await browser.newContext();
    const context2 = await browser.newContext();
    const context3 = await browser.newContext();

    const page1 = await context1.newPage();
    const page2 = await context2.newPage();
    const page3 = await context3.newPage();

    page1.on('console', msg => console.log(`[Page1] ${msg.text()}`));
    page2.on('console', msg => console.log(`[Page2] ${msg.text()}`));
    page3.on('console', msg => console.log(`[Page3] ${msg.text()}`));

    await setupTestWithServer(page1, serverInstance);
    await setupTestWithServer(page2, serverInstance);
    await setupTestWithServer(page3, serverInstance);

    await waitForVFSConnection(page1);
    await waitForVFSConnection(page2);
    await waitForVFSConnection(page3);

    await page1.getByTestId('enable-sync-btn').click();
    await page2.getByTestId('enable-sync-btn').click();
    await page3.getByTestId('enable-sync-btn').click();

    await page1.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );
    await page2.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );
    await page3.waitForSelector(
      '[data-testid="sync-status"]:has-text("Enabled")'
    );

    await Promise.all([
      page1.getByTestId('increment-btn').click(),
      page2.getByTestId('increment-btn').click(),
      page3.getByTestId('increment-btn').click(),
    ]);

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 3;
      },
      { timeout: 10000 }
    );

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 3;
      },
      { timeout: 10000 }
    );

    await page3.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 3;
      },
      { timeout: 10000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();
    const counter3 = await page3.getByTestId('counter-value').textContent();

    const value1 = parseInt(counter1?.match(/\d+/)?.[0] || '0');
    const value2 = parseInt(counter2?.match(/\d+/)?.[0] || '0');
    const value3 = parseInt(counter3?.match(/\d+/)?.[0] || '0');

    expect(value1).toBe(3);
    expect(value2).toBe(3);
    expect(value3).toBe(3);

    await context1.close();
    await context2.close();
    await context3.close();
  });

  test('should eventually converge after rapid concurrent updates', async ({
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

    for (let i = 0; i < 5; i++) {
      await page1.getByTestId('increment-btn').click();
    }
    for (let i = 0; i < 5; i++) {
      await page2.getByTestId('increment-btn').click();
    }

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 10;
      },
      { timeout: 15000 }
    );

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 10;
      },
      { timeout: 15000 }
    );

    await new Promise(resolve => setTimeout(resolve, 2000));

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    const value1 = parseInt(counter1?.match(/\d+/)?.[0] || '0');
    const value2 = parseInt(counter2?.match(/\d+/)?.[0] || '0');

    expect(value1).toBe(value2);
    expect(value1).toBe(10);

    await context1.close();
    await context2.close();
  });
});
