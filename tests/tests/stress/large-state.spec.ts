import {
  test,
  expect,
  setupTestWithServer,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Stress Test - Large State', () => {
  test('should sync large state objects between clients', async ({
    browser,
    serverInstance,
  }) => {
    console.log(`Testing large state sync on port ${serverInstance.port}`);

    const context1 = await browser.newContext();
    const context2 = await browser.newContext();

    const page1 = await context1.newPage();
    const page2 = await context2.newPage();

    page1.on('console', msg => console.log(`[Page1] ${msg.text()}`));
    page2.on('console', msg => console.log(`[Page2] ${msg.text()}`));

    await setupTestWithServer(page1, serverInstance);
    await setupTestWithServer(page2, serverInstance);

    await waitForVFSConnection(page1);
    await waitForVFSConnection(page2);

    const largeData = {
      id: 'test-large-object',
      timestamp: Date.now(),
      data: Array.from({ length: 1000 }, (_, i) => ({
        index: i,
        value: `Value ${i}`,
        metadata: {
          created: new Date().toISOString(),
          random: Math.random(),
        },
      })),
    };

    await page1.evaluate(async data => {
      const store = (window as any).__counterStore;
      if (store) {
        store.getState().setData(data);
      }
    }, largeData);

    await page2.waitForFunction(
      () => {
        const store = (window as any).__counterStore;
        const state = store?.getState();
        return state?.largeData?.data?.length >= 1000;
      },
      { timeout: 30000 }
    );

    const syncedData = await page2.evaluate(() => {
      const store = (window as any).__counterStore;
      const state = store?.getState();
      return state?.largeData;
    });

    expect(syncedData).toBeTruthy();
    expect(syncedData.id).toBe(largeData.id);
    expect(syncedData.data.length).toBe(1000);

    console.log('Large state successfully synced!');

    await context1.close();
    await context2.close();
  });

  test('should handle multiple large updates', async ({
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

    for (let iteration = 0; iteration < 5; iteration++) {
      const largeData = {
        iteration,
        timestamp: Date.now(),
        data: Array.from({ length: 500 }, (_, i) => ({
          index: i,
          iteration,
          value: `Iteration ${iteration} - Item ${i}`,
        })),
      };

      await page1.evaluate(async data => {
        const store = (window as any).__counterStore;
        if (store) {
          store.getState().setData(data);
        }
      }, largeData);

      await page2.waitForFunction(
        (expectedIteration: number) => {
          const store = (window as any).__counterStore;
          const state = store?.getState();
          return state?.largeData?.iteration === expectedIteration;
        },
        iteration,
        { timeout: 20000 }
      );

      console.log(`Iteration ${iteration} synced successfully`);
    }

    const finalData = await page2.evaluate(() => {
      const store = (window as any).__counterStore;
      const state = store?.getState();
      return state?.largeData;
    });

    expect(finalData.iteration).toBe(4);
    expect(finalData.data.length).toBe(500);

    console.log('All large state updates synced successfully!');

    await context1.close();
    await context2.close();
  });
});
