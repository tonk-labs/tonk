import {
  expect,
  setupTestWithServer,
  test,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Stress Test - Rapid Updates', () => {
  test('should handle rapid successive state changes', async ({
    browser,
    serverInstance,
  }) => {
    console.log(`Testing rapid updates on port ${serverInstance.port}`);

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

    console.log('Performing 20 rapid increments on page1');
    for (let i = 0; i < 20; i++) {
      await page1.getByTestId('increment-btn').click();
      await page1.waitForTimeout(50);
    }

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 20;
      },
      { timeout: 30000 }
    );

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    const value1 = parseInt(counter1?.match(/\d+/)?.[0] || '0');
    const value2 = parseInt(counter2?.match(/\d+/)?.[0] || '0');

    expect(value1).toBe(20);
    expect(value2).toBe(20);

    console.log('All rapid updates synced successfully!');

    await context1.close();
    await context2.close();
  });

  test('should handle alternating rapid updates from two clients', async ({
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

    console.log('Performing alternating rapid updates');
    for (let i = 0; i < 10; i++) {
      await page1.getByTestId('increment-btn').click();
      await page1.waitForTimeout(100);
      await page2.getByTestId('increment-btn').click();
      await page2.waitForTimeout(100);
    }

    await page1.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 20;
      },
      { timeout: 30000 }
    );

    await page2.waitForFunction(
      () => {
        const counter = document.querySelector('[data-testid="counter-value"]');
        const value = parseInt(counter?.textContent?.match(/\d+/)?.[0] || '0');
        return value >= 20;
      },
      { timeout: 30000 }
    );

    await new Promise(resolve => setTimeout(resolve, 2000));

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    const value1 = parseInt(counter1?.match(/\d+/)?.[0] || '0');
    const value2 = parseInt(counter2?.match(/\d+/)?.[0] || '0');

    expect(value1).toBe(value2);
    expect(value1).toBe(20);

    console.log('Alternating rapid updates converged successfully!');

    await context1.close();
    await context2.close();
  });

  test('should handle burst updates with pauses', async ({
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

    for (let burst = 0; burst < 3; burst++) {
      console.log(`Burst ${burst + 1}: Performing 5 rapid updates`);
      for (let i = 0; i < 5; i++) {
        await page1.getByTestId('increment-btn').click();
        await page1.waitForTimeout(50);
      }

      const expectedValue = (burst + 1) * 5;
      await page2.waitForFunction(
        (expected: number) => {
          const counter = document.querySelector(
            '[data-testid="counter-value"]'
          );
          const value = parseInt(
            counter?.textContent?.match(/\d+/)?.[0] || '0'
          );
          return value >= expected;
        },
        expectedValue,
        { timeout: 15000 }
      );

      console.log(`Burst ${burst + 1} synced, pausing...`);
      await page1.waitForTimeout(1000);
    }

    const counter1 = await page1.getByTestId('counter-value').textContent();
    const counter2 = await page2.getByTestId('counter-value').textContent();

    const value1 = parseInt(counter1?.match(/\d+/)?.[0] || '0');
    const value2 = parseInt(counter2?.match(/\d+/)?.[0] || '0');

    expect(value1).toBe(15);
    expect(value2).toBe(15);

    console.log('All burst updates synced successfully!');

    await context1.close();
    await context2.close();
  });
});
