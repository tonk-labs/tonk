import { test, expect } from '../fixtures';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';

test.describe('Throughput Benchmarks', () => {
  test.beforeEach(async ({ page, serverInstance }) => {
    await setupTestWithServer(page, serverInstance);
  });

  test('should display connection status', async ({ page, serverInstance }) => {
    // Check that the test UI loads
    await expect(page.locator('h1')).toContainText('Tonk Test Suite');

    // Check for connection status element
    const connectionStatus = page.getByTestId('connection-status');
    await expect(connectionStatus).toBeVisible();

    // Verify connection to the correct server
    await waitForVFSConnection(page);
    await expect(connectionStatus).toContainText('Connected');

    // Verify it's using the right port
    const serverInfo = page.getByTestId('server-info');
    await expect(serverInfo).toContainText(`Port: ${serverInstance.port}`);
  });

  test('should have test control buttons', async ({ page }) => {
    // Wait for connection first
    await waitForVFSConnection(page);

    // Check for test buttons
    const throughputBtn = page.getByTestId('throughput-test-btn');
    const batchBtn = page.getByTestId('batch-operations-btn');

    await expect(throughputBtn).toBeVisible();
    await expect(batchBtn).toBeVisible();

    // Buttons should be enabled when connected
    await expect(throughputBtn).toBeEnabled();
    await expect(batchBtn).toBeEnabled();
  });

  test('should display metrics', async ({ page }) => {
    // Wait for connection first
    await waitForVFSConnection(page);

    // Check for metrics display
    const operationsCount = page.getByTestId('operations-count');
    const throughputValue = page.getByTestId('throughput-value');
    const errorCount = page.getByTestId('error-count');
    const memoryUsage = page.getByTestId('memory-usage');

    await expect(operationsCount).toBeVisible();
    await expect(throughputValue).toBeVisible();
    await expect(errorCount).toBeVisible();
    await expect(memoryUsage).toBeVisible();

    // Check initial values
    await expect(operationsCount).toContainText('0');
    await expect(errorCount).toContainText('0');
  });

  test('should run throughput test when server is connected', async ({
    page,
    serverInstance,
  }) => {
    // Wait for connection to be established
    await waitForVFSConnection(page);

    // Get the throughput test button
    const throughputBtn = page.getByTestId('throughput-test-btn');
    await expect(throughputBtn).toBeEnabled();

    // Run the test
    await throughputBtn.click();

    // Check that operations count increases
    await expect(page.getByTestId('operations-count')).not.toContainText('0', {
      timeout: 10000,
    });

    // Verify no errors occurred
    await expect(page.getByTestId('error-count')).toContainText('0');

    console.log(
      `Throughput test completed successfully on port ${serverInstance.port}`
    );
  });

  test('should run batch operations with server', async ({
    page,
    serverInstance,
  }) => {
    // Wait for connection
    await waitForVFSConnection(page);

    // Get the batch operations button
    const batchBtn = page.getByTestId('batch-operations-btn');
    await expect(batchBtn).toBeEnabled();

    // Run the test
    await batchBtn.click();

    // Check that operations count increases significantly (should be 10 operations)
    await expect(page.getByTestId('operations-count')).toContainText('10', {
      timeout: 30000,
    });

    // Verify no errors occurred
    await expect(page.getByTestId('error-count')).toContainText('0');

    console.log(
      `Batch operations test completed successfully on port ${serverInstance.port}`
    );
  });
});
