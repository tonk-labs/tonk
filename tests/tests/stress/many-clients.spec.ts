import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Stress Test - Many Clients', () => {
  test('should handle 10 concurrent clients syncing', async ({
    browser,
    serverInstance,
  }) => {
    console.log(`Stress test with 10 clients on port ${serverInstance.port}`);

    const contexts = [];
    const pages = [];

    for (let i = 0; i < 10; i++) {
      const context = await browser.newContext();
      const page = await context.newPage();

      page.on('console', msg => console.log(`[Page${i}] ${msg.text()}`));

      await setupTestWithServer(page, serverInstance);
      await waitForVFSConnection(page);

      contexts.push(context);
      pages.push(page);
    }

    for (const page of pages) {
      await page.getByTestId('enable-sync-btn').click();
    }

    for (const page of pages) {
      await page.waitForSelector(
        '[data-testid="sync-status"]:has-text("Enabled")',
        { timeout: 10000 }
      );
    }

    console.log('All clients synced, incrementing on first client');
    await pages[0].getByTestId('increment-btn').click();

    for (let i = 0; i < 10; i++) {
      await pages[i].waitForFunction(
        () => {
          const counter = document.querySelector(
            '[data-testid="counter-value"]'
          );
          return counter?.textContent?.includes('1');
        },
        { timeout: 20000 }
      );
    }

    const counters = [];
    for (const page of pages) {
      const counter = await page.getByTestId('counter-value').textContent();
      counters.push(counter);
    }

    for (const counter of counters) {
      expect(counter).toContain('1');
    }

    console.log('All 10 clients successfully synced!');

    for (const context of contexts) {
      await context.close();
    }
  });

  test('should handle concurrent writes from 5 clients', async ({
    browser,
    serverInstance,
  }) => {
    const contexts = [];
    const pages = [];

    for (let i = 0; i < 5; i++) {
      const context = await browser.newContext();
      const page = await context.newPage();

      await setupTestWithServer(page, serverInstance);
      await waitForVFSConnection(page);

      contexts.push(context);
      pages.push(page);
    }

    for (const page of pages) {
      await page.getByTestId('enable-sync-btn').click();
    }

    for (const page of pages) {
      await page.waitForSelector(
        '[data-testid="sync-status"]:has-text("Enabled")',
        { timeout: 10000 }
      );
    }

    await Promise.all(
      pages.map(page => page.getByTestId('increment-btn').click())
    );

    for (const page of pages) {
      await page.waitForFunction(
        () => {
          const counter = document.querySelector(
            '[data-testid="counter-value"]'
          );
          const value = parseInt(
            counter?.textContent?.match(/\d+/)?.[0] || '0'
          );
          return value >= 5;
        },
        { timeout: 20000 }
      );
    }

    await new Promise(resolve => setTimeout(resolve, 3000));

    const counters = [];
    for (const page of pages) {
      const counter = await page.getByTestId('counter-value').textContent();
      const value = parseInt(counter?.match(/\d+/)?.[0] || '0');
      counters.push(value);
    }

    const allEqual = counters.every(v => v === counters[0]);
    expect(allEqual).toBe(true);
    expect(counters[0]).toBe(5);

    console.log(`All 5 clients converged to the same value: ${counters[0]}`);

    for (const context of contexts) {
      await context.close();
    }
  });
});
