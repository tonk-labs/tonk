import { test, expect } from '../fixtures';
import { ConnectionManager } from '../../src/utils/connection-manager';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import { serverProfiler } from '../../src/utils/server-profiler';
import { UptimeLogger } from '../../src/utils/uptime-logger';

interface ThrottleScenario {
  name: string;
  delayMs: number;
  operations: number;
  expectedMaxLatency: number;
}

test.describe('Uptime Test - Throttling and Network Stress', () => {
  test('should handle various network throttling scenarios', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'throttling-stress-test';
    const clientCount = 10;

    const scenarios: ThrottleScenario[] = [
      {
        name: 'Baseline (No Throttling)',
        delayMs: 0,
        operations: 50,
        expectedMaxLatency: 200,
      },
      {
        name: 'Light Latency (50ms delay)',
        delayMs: 50,
        operations: 50,
        expectedMaxLatency: 300,
      },
      {
        name: 'Medium Latency (100ms delay)',
        delayMs: 100,
        operations: 50,
        expectedMaxLatency: 400,
      },
      {
        name: 'High Latency (250ms delay)',
        delayMs: 250,
        operations: 50,
        expectedMaxLatency: 700,
      },
    ];

    console.log(
      `Starting throttling stress test with ${scenarios.length} scenarios`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(
      testName,
      metricsCollector,
      serverProfiler
    );

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.startLogging(20000);

    for (const scenario of scenarios) {
      console.log(`\n${'='.repeat(60)}\n${scenario.name}\n${'='.repeat(60)}`);

      const context = await browser.newContext();

      if (scenario.delayMs > 0) {
        await context.route('**/*', async route => {
          await new Promise(resolve => setTimeout(resolve, scenario.delayMs));
          await route.continue();
        });
        console.log(`Applied ${scenario.delayMs}ms network delay`);
      }

      const pages = [];
      for (let i = 0; i < clientCount; i++) {
        const page = await context.newPage();
        await page.addInitScript({
          content: `
            window.serverConfig = ${JSON.stringify({
              port: serverInstance.port,
              wsUrl: serverInstance.wsUrl,
              manifestUrl: serverInstance.manifestUrl,
            })};
          `,
        });
        await page.goto('http://localhost:5173');
        await page.waitForLoadState('load');

        await page.waitForFunction(
          () => {
            const status = document.querySelector(
              '[data-testid="connection-status"]'
            );
            return status?.textContent?.includes('Connected');
          },
          { timeout: 30000 }
        );

        await page.getByTestId('enable-sync-btn').click();
        await page.waitForSelector(
          '[data-testid="sync-status"]:has-text("Enabled")',
          { timeout: 10000 }
        );

        pages.push(page);
      }

      console.log(`Created ${clientCount} clients with throttling enabled`);

      const scenarioStartTime = Date.now();
      let completedOps = 0;

      for (let op = 0; op < scenario.operations; op++) {
        const randomPage = pages[Math.floor(Math.random() * pages.length)];

        const endOp = metricsCollector.startOperation();

        try {
          await randomPage.getByTestId('increment-btn').click();
          await randomPage.waitForTimeout(50);
          completedOps++;
        } catch (error) {
          metricsCollector.recordError('throttled-operation-failed');
          console.error(`Operation ${op + 1} failed:`, error);
        }

        endOp();

        if ((op + 1) % 20 === 0) {
          console.log(
            `  Completed ${op + 1}/${scenario.operations} operations`
          );
        }
      }

      const scenarioDuration = (Date.now() - scenarioStartTime) / 1000;
      const metrics = await metricsCollector.getMetrics();

      console.log(
        `Scenario complete in ${scenarioDuration.toFixed(1)}s: ${completedOps}/${scenario.operations} operations`
      );
      console.log(
        `  P95 Latency: ${metrics.latency.p95.toFixed(2)}ms, P99: ${metrics.latency.p99.toFixed(2)}ms`
      );
      console.log(`  Errors: ${metrics.errors.count}`);

      expect(completedOps).toBe(scenario.operations);
      expect(metrics.latency.p95).toBeLessThan(scenario.expectedMaxLatency);

      await context.close();
      console.log(`Cleaned up ${clientCount} throttled clients`);

      await new Promise(resolve => setTimeout(resolve, 2000));
    }

    const report = uptimeLogger.stopLogging();
    const savedFiles = uptimeLogger.saveReport();

    console.log(`\nReports saved to: ${savedFiles.join(', ')}`);

    await connectionManager.closeAll();

    expect(report.summary.totalOperations).toBeGreaterThan(0);
    expect(report.summary.errorRate).toBeLessThan(10);
    expect(report.summary.healthScore).toBeGreaterThan(50);

    console.log(
      `✓ Throttling stress test passed! Health score: ${report.summary.healthScore.toFixed(0)}/100`
    );
  });

  test('should recover from intermittent disconnections', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'intermittent-disconnection-test';
    const clientCount = 5;
    const testDurationMinutes = 10;

    console.log(
      `Starting ${testDurationMinutes}-minute intermittent disconnection test`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(
      testName,
      metricsCollector,
      serverProfiler
    );

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.startLogging(30000);

    console.log(`Creating ${clientCount} clients...`);
    const connectionIds = await connectionManager.createConnections(
      clientCount,
      'disconnect-test'
    );

    for (const connId of connectionIds) {
      const conn = connectionManager.getConnection(connId);
      if (conn) {
        await conn.page.getByTestId('enable-sync-btn').click();
        await conn.page.waitForSelector(
          '[data-testid="sync-status"]:has-text("Enabled")',
          { timeout: 10000 }
        );
      }
    }

    const startTime = Date.now();
    const endTime = startTime + testDurationMinutes * 60 * 1000;
    let totalOperations = 0;
    let disconnectionEvents = 0;

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

      if (totalOperations % 30 === 0 && Math.random() < 0.3) {
        const victimConnId =
          connectionIds[Math.floor(Math.random() * connectionIds.length)];
        const conn = connectionManager.getConnection(victimConnId);

        if (conn) {
          console.log(
            `Simulating disconnection for ${victimConnId} (event ${disconnectionEvents + 1})`
          );

          await conn.page.evaluate(() => {
            const vfs = (window as any).vfsService;
            if (vfs && vfs.disconnect) {
              vfs.disconnect();
            }
          });

          disconnectionEvents++;

          await new Promise(resolve => setTimeout(resolve, 2000));

          await conn.page.evaluate(() => {
            const vfs = (window as any).vfsService;
            if (vfs && vfs.reconnect) {
              vfs.reconnect();
            }
          });

          console.log(`Reconnected ${victimConnId}`);
        }
      }

      if (totalOperations % 50 === 0) {
        const elapsedMinutes = (Date.now() - startTime) / 60000;
        console.log(
          `[${elapsedMinutes.toFixed(1)}m] ${totalOperations} ops, ${disconnectionEvents} disconnection events`
        );
      }

      await new Promise(resolve => setTimeout(resolve, 1000));
    }

    console.log(
      `Test complete: ${totalOperations} operations, ${disconnectionEvents} disconnection events`
    );

    const finalHealth = await connectionManager.checkHealth();
    console.log(
      `Final health: ${finalHealth.healthy}/${clientCount} clients healthy`
    );

    const report = uptimeLogger.stopLogging();
    uptimeLogger.saveReport();

    await connectionManager.closeAll();

    expect(report.summary.totalOperations).toBeGreaterThan(0);
    expect(finalHealth.healthy).toBeGreaterThanOrEqual(clientCount * 0.8);
    expect(report.summary.errorRate).toBeLessThan(20);

    console.log(
      `✓ Disconnection recovery test passed! ${finalHealth.healthy}/${clientCount} clients recovered`
    );
  });
});
