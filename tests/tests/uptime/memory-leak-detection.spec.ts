import { test, expect } from '../fixtures';
import { ConnectionManager } from '../../src/utils/connection-manager';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import { UptimeLogger } from '../../src/utils/uptime-logger';

test.describe('Uptime Test - Memory Leak Detection', () => {
  test('should not leak memory through cyclic connect/disconnect operations', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'memory-leak-cyclic-connections';
    const durationMinutes = 20;
    const cyclesPerMinute = 1;
    const clientsPerCycle = 5;
    const operationsPerCycle = 100;

    console.log(
      `Starting ${durationMinutes}-minute memory leak detection test`
    );
    console.log(
      `  ${cyclesPerMinute} cycle/min, ${clientsPerCycle} clients/cycle, ${operationsPerCycle} ops/cycle`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(testName, metricsCollector);

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.setServerInstance(serverInstance);
    uptimeLogger.startLogging(30000);

    const startTime = Date.now();
    const endTime = startTime + durationMinutes * 60 * 1000;
    let cycleCount = 0;
    let totalOperations = 0;

    const memorySnapshots: number[] = [];

    const captureServerMemory = async () => {
      try {
        const metricsUrl = `http://localhost:${serverInstance.port}/metrics`;
        const response = await fetch(metricsUrl);
        if (response.ok) {
          const serverMetrics = await response.json();
          return serverMetrics.memory.rss / (1024 * 1024);
        }
      } catch (error) {
        console.warn('Failed to fetch server metrics:', error);
      }
      return 0;
    };

    const baselineMemory = await captureServerMemory();
    console.log(`Baseline server memory: ${baselineMemory.toFixed(2)} MB`);

    while (Date.now() < endTime) {
      cycleCount++;
      const cycleStartTime = Date.now();

      console.log(`\n--- Cycle ${cycleCount} ---`);

      console.log(`Creating ${clientsPerCycle} new clients...`);
      const connectionIds = await connectionManager.createConnections(
        clientsPerCycle,
        `cycle-${cycleCount}`
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

      console.log(`Performing ${operationsPerCycle} operations...`);
      for (let op = 0; op < operationsPerCycle; op++) {
        const randomConnId =
          connectionIds[Math.floor(Math.random() * connectionIds.length)];

        const endOp = metricsCollector.startOperation();

        const result = await connectionManager.executeOperation(
          randomConnId,
          async page => {
            await page.getByTestId('increment-btn').click();
            await page.waitForTimeout(20);
          }
        );

        endOp();

        if (!result.success) {
          metricsCollector.recordError('operation-failed');
        }

        totalOperations++;
      }

      console.log(`Disconnecting ${clientsPerCycle} clients...`);
      await connectionManager.closeConnections(connectionIds);

      await new Promise(resolve => setTimeout(resolve, 1000));

      const currentMemory = await captureServerMemory();
      memorySnapshots.push(currentMemory);

      const memoryGrowth = currentMemory - baselineMemory;
      console.log(
        `Cycle ${cycleCount} complete: Memory = ${currentMemory.toFixed(2)} MB (growth: ${memoryGrowth >= 0 ? '+' : ''}${memoryGrowth.toFixed(2)} MB)`
      );

      const cycleDuration = (Date.now() - cycleStartTime) / 1000;
      const remainingTime = Math.max(
        0,
        60000 / cyclesPerMinute - cycleDuration * 1000
      );

      if (remainingTime > 0) {
        await new Promise(resolve => setTimeout(resolve, remainingTime));
      }
    }

    console.log(
      `\nCompleted ${cycleCount} cycles with ${totalOperations} total operations`
    );

    const report = await uptimeLogger.stopLogging();
    const savedFiles = await uptimeLogger.saveReport();

    console.log(`Reports saved to: ${savedFiles.join(', ')}`);

    await connectionManager.closeAll();

    const quarterSize = Math.floor(memorySnapshots.length / 4);
    const firstQuarter = memorySnapshots.slice(0, quarterSize);
    const lastQuarter = memorySnapshots.slice(-quarterSize);

    const avgFirstQuarter =
      firstQuarter.reduce((sum, m) => sum + m, 0) / firstQuarter.length;
    const avgLastQuarter =
      lastQuarter.reduce((sum, m) => sum + m, 0) / lastQuarter.length;

    const memoryIncrease = avgLastQuarter - avgFirstQuarter;

    console.log(`\n=== Memory Leak Analysis ===`);
    console.log(`First quarter avg memory: ${avgFirstQuarter.toFixed(2)} MB`);
    console.log(`Last quarter avg memory: ${avgLastQuarter.toFixed(2)} MB`);
    console.log(
      `Memory increase: ${memoryIncrease >= 0 ? '+' : ''}${memoryIncrease.toFixed(2)} MB`
    );

    const leakDetected = memoryIncrease > 75;

    if (leakDetected) {
      console.warn(
        `⚠️  Potential memory leak detected! Memory increased by ${memoryIncrease.toFixed(2)} MB`
      );
    } else {
      console.log(
        `✓ No memory leak detected (${memoryIncrease.toFixed(2)} MB increase is acceptable)`
      );
    }

    expect(report.summary.totalOperations).toBeGreaterThan(0);
    expect(report.summary.errorRate).toBeLessThan(5);
    expect(memoryIncrease).toBeLessThan(100);
    expect(leakDetected).toBe(false);

    console.log(
      `✓ Memory leak detection test passed! Health score: ${report.summary.healthScore.toFixed(0)}/100`
    );
  });

  test('should maintain stable memory under sustained load', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'memory-sustained-load';
    const durationMinutes = 15;
    const clientCount = 10;
    const operationsPerMinute = 60;

    console.log(
      `Starting ${durationMinutes}-minute sustained load memory test with ${clientCount} clients`
    );

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(testName, metricsCollector);

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.setServerInstance(serverInstance);
    uptimeLogger.startLogging(20000);

    console.log(`Creating ${clientCount} clients...`);
    const connectionIds = await connectionManager.createConnections(
      clientCount,
      'sustained'
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

    await metricsCollector.collectMemoryMetrics();
    const initialMemory = (await metricsCollector.getMetrics()).memory.heapUsed;

    const startTime = Date.now();
    const endTime = startTime + durationMinutes * 60 * 1000;
    let totalOperations = 0;
    const operationInterval = (60 * 1000) / operationsPerMinute;

    console.log(
      `Running sustained operations for ${durationMinutes} minutes...`
    );

    while (Date.now() < endTime) {
      const randomConnId =
        connectionIds[Math.floor(Math.random() * connectionIds.length)];

      const endOp = metricsCollector.startOperation();

      await connectionManager.executeOperation(randomConnId, async page => {
        await page.getByTestId('increment-btn').click();
        await page.waitForTimeout(50);
      });

      endOp();
      totalOperations++;

      if (totalOperations % 100 === 0) {
        const elapsedMinutes = (Date.now() - startTime) / 60000;
        await metricsCollector.collectMemoryMetrics();
        const currentMetrics = await metricsCollector.getMetrics();
        console.log(
          `[${elapsedMinutes.toFixed(1)}m] ${totalOperations} ops, ` +
            `Memory: ${(currentMetrics.memory.heapUsed / (1024 * 1024)).toFixed(2)} MB`
        );
      }

      await new Promise(resolve => setTimeout(resolve, operationInterval));
    }

    await metricsCollector.collectMemoryMetrics();
    const finalMemory = (await metricsCollector.getMetrics()).memory.heapUsed;
    const memoryGrowthMB = (finalMemory - initialMemory) / (1024 * 1024);

    console.log(
      `\nMemory analysis: ${(initialMemory / (1024 * 1024)).toFixed(2)} MB → ${(finalMemory / (1024 * 1024)).toFixed(2)} MB (${memoryGrowthMB >= 0 ? '+' : ''}${memoryGrowthMB.toFixed(2)} MB)`
    );

    const memoryTrend = metricsCollector.getMemoryTrend();
    const leakDetected = metricsCollector.detectMemoryLeak(30);

    if (leakDetected) {
      console.warn(
        `⚠️  Memory leak detected in sustained load test! Growth: ${memoryGrowthMB.toFixed(2)} MB`
      );
    } else {
      console.log(
        `✓ No memory leak detected (${memoryGrowthMB.toFixed(2)} MB growth is normal)`
      );
    }

    const report = await uptimeLogger.stopLogging();
    await uptimeLogger.saveReport();

    await connectionManager.closeAll();

    expect(report.summary.totalOperations).toBe(totalOperations);
    expect(report.summary.errorRate).toBeLessThan(2);
    expect(memoryGrowthMB).toBeLessThan(100);
    expect(leakDetected).toBe(false);

    console.log(
      `✓ Sustained load memory test passed! ${memoryTrend.length} memory snapshots collected`
    );
  });
});
