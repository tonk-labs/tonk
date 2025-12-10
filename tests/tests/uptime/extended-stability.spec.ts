import { ConnectionManager } from '../../src/utils/connection-manager';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import { UptimeLogger } from '../../src/utils/uptime-logger';
import { expect, test } from '../fixtures';

test.describe('Uptime Test - Extended Stability', () => {
  test('should maintain stable performance over 30 minutes', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'extended-stability-30min';
    const durationMinutes = 30;
    const clientCount = 10;
    const operationsPerMinute = 60;

    console.log(
      `Starting ${durationMinutes}-minute stability test with ${clientCount} clients`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(testName, metricsCollector);

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.setServerInstance(serverInstance);

    uptimeLogger.startLogging(30000);

    console.log(`Creating ${clientCount} concurrent clients...`);
    const connectionIds = await connectionManager.createConnections(
      clientCount,
      'stability'
    );

    console.log(
      `Running steady-state operations for ${durationMinutes} minutes...`
    );

    const startTime = Date.now();
    const endTime = startTime + durationMinutes * 60 * 1000;
    let totalOperations = 0;

    const operationInterval = (60 * 1000) / operationsPerMinute;

    while (Date.now() < endTime) {
      const randomConnId =
        connectionIds[Math.floor(Math.random() * connectionIds.length)];

      const endOp = metricsCollector.startOperation();

      const result = await connectionManager.executeOperation(
        randomConnId,
        async page => {
          await page.getByTestId('increment-btn').click();
          await page.waitForTimeout(50);
        }
      );

      endOp();

      if (!result.success) {
        metricsCollector.recordError('operation-failed');
      }

      totalOperations++;

      const elapsedMinutes = (Date.now() - startTime) / 60000;
      if (totalOperations % 100 === 0) {
        console.log(
          `[${elapsedMinutes.toFixed(1)}m] Completed ${totalOperations} operations`
        );

        const health = await connectionManager.checkHealth();
        console.log(
          `  Health: ${health.healthy} healthy, ${health.unhealthy} unhealthy`
        );

        if (health.unhealthy > clientCount * 0.1) {
          console.warn(
            `!  More than 10% of connections are unhealthy! (${health.unhealthy}/${clientCount})`
          );
        }
      }

      await new Promise(resolve => setTimeout(resolve, operationInterval));
    }

    console.log(
      `Completed ${totalOperations} operations over ${durationMinutes} minutes`
    );

    const finalHealth = await connectionManager.checkHealth();
    console.log(
      `Final health check: ${finalHealth.healthy} healthy, ${finalHealth.unhealthy} unhealthy`
    );

    const report = await uptimeLogger.stopLogging();
    const savedFiles = await uptimeLogger.saveReport();

    console.log(`Reports saved to: ${savedFiles.join(', ')}`);

    await connectionManager.closeAll();

    expect(report.summary.totalOperations).toBeGreaterThan(0);
    expect(report.summary.errorRate).toBeLessThan(1);
    expect(report.summary.degradationDetected).toBe(false);
    expect(report.summary.memoryGrowthMB).toBeLessThan(100);
    expect(report.summary.healthScore).toBeGreaterThan(80);
    expect(finalHealth.healthy).toBe(clientCount);

    console.log(
      `✓ Extended stability test passed! Health score: ${report.summary.healthScore.toFixed(0)}/100`
    );
  });

  test('should handle 20 clients for 15 minutes', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'extended-stability-20-clients';
    const durationMinutes = 15;
    const clientCount = 20;
    const operationsPerMinute = 40;

    console.log(
      `Starting ${durationMinutes}-minute stability test with ${clientCount} clients`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(testName, metricsCollector);

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.startLogging(30000);

    const connectionIds = await connectionManager.createConnections(
      clientCount,
      'stability-20'
    );

    const startTime = Date.now();
    const endTime = startTime + durationMinutes * 60 * 1000;
    let totalOperations = 0;
    const operationInterval = (60 * 1000) / operationsPerMinute;

    while (Date.now() < endTime) {
      const randomConnId =
        connectionIds[Math.floor(Math.random() * connectionIds.length)];

      const endOp = metricsCollector.startOperation();

      const result = await connectionManager.executeOperation(
        randomConnId,
        async page => {
          await page.getByTestId('increment-btn').click();
          await page.waitForTimeout(50);
        }
      );

      endOp();

      if (!result.success) {
        metricsCollector.recordError('operation-failed');
      }

      totalOperations++;

      if (totalOperations % 100 === 0) {
        const elapsedMinutes = (Date.now() - startTime) / 60000;
        console.log(
          `[${elapsedMinutes.toFixed(1)}m] Completed ${totalOperations} operations`
        );
      }

      await new Promise(resolve => setTimeout(resolve, operationInterval));
    }

    const report = await uptimeLogger.stopLogging();
    await uptimeLogger.saveReport();

    await connectionManager.closeAll();

    expect(report.summary.errorRate).toBeLessThan(2);
    expect(report.summary.degradationDetected).toBe(false);
    expect(report.summary.healthScore).toBeGreaterThan(70);

    console.log(
      `✓ 20-client stability test passed! Health score: ${report.summary.healthScore.toFixed(0)}/100`
    );
  });
});
