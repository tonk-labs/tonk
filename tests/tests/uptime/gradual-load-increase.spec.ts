import { test, expect } from '../fixtures';
import { ConnectionManager } from '../../src/utils/connection-manager';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import { serverProfiler } from '../../src/utils/server-profiler';
import { UptimeLogger } from '../../src/utils/uptime-logger';

interface LoadPhase {
  name: string;
  targetClients: number;
  durationMinutes: number;
  operationsPerMinute: number;
}

test.describe('Uptime Test - Gradual Load Increase', () => {
  test('should handle gradually increasing load from 1 to 50 clients', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'gradual-load-increase';

    const phases: LoadPhase[] = [
      {
        name: 'Phase 1: Baseline',
        targetClients: 1,
        durationMinutes: 5,
        operationsPerMinute: 30,
      },
      {
        name: 'Phase 2: Light Load',
        targetClients: 5,
        durationMinutes: 5,
        operationsPerMinute: 30,
      },
      {
        name: 'Phase 3: Medium Load',
        targetClients: 15,
        durationMinutes: 5,
        operationsPerMinute: 30,
      },
      {
        name: 'Phase 4: Heavy Load',
        targetClients: 30,
        durationMinutes: 5,
        operationsPerMinute: 30,
      },
      {
        name: 'Phase 5: Stress Load',
        targetClients: 50,
        durationMinutes: 5,
        operationsPerMinute: 30,
      },
      {
        name: 'Phase 6: Cool Down',
        targetClients: 50,
        durationMinutes: 5,
        operationsPerMinute: 0,
      },
    ];

    console.log(
      `Starting gradual load test with ${phases.length} phases over ${phases.reduce((sum, p) => sum + p.durationMinutes, 0)} minutes`
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

    let currentConnections = 0;
    let allConnectionIds: string[] = [];

    for (const phase of phases) {
      console.log(`\n${'='.repeat(60)}\n${phase.name}\n${'='.repeat(60)}`);
      console.log(
        `Target: ${phase.targetClients} clients, Duration: ${phase.durationMinutes}min, Ops/min: ${phase.operationsPerMinute}`
      );

      const clientsToAdd = phase.targetClients - currentConnections;

      if (clientsToAdd > 0) {
        console.log(`Adding ${clientsToAdd} new clients...`);
        const newConnectionIds = await connectionManager.createConnections(
          clientsToAdd,
          `phase-${phases.indexOf(phase)}`
        );

        for (const connId of newConnectionIds) {
          const conn = connectionManager.getConnection(connId);
          if (conn) {
            await conn.page.getByTestId('enable-sync-btn').click();
            await conn.page.waitForSelector(
              '[data-testid="sync-status"]:has-text("Enabled")',
              { timeout: 10000 }
            );
          }
        }

        allConnectionIds = [...allConnectionIds, ...newConnectionIds];
        currentConnections = phase.targetClients;

        console.log(
          `Now running with ${currentConnections} total clients (${allConnectionIds.length} connections)`
        );
      }

      const phaseStartTime = Date.now();
      const phaseEndTime = phaseStartTime + phase.durationMinutes * 60 * 1000;
      let phaseOperations = 0;

      if (phase.operationsPerMinute > 0) {
        const operationInterval = (60 * 1000) / phase.operationsPerMinute;

        while (Date.now() < phaseEndTime) {
          const randomConnId =
            allConnectionIds[
              Math.floor(Math.random() * allConnectionIds.length)
            ];

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

          phaseOperations++;

          if (phaseOperations % 50 === 0) {
            const elapsedSeconds = (Date.now() - phaseStartTime) / 1000;
            console.log(
              `  [${elapsedSeconds.toFixed(0)}s] ${phaseOperations} ops in this phase`
            );
          }

          await new Promise(resolve => setTimeout(resolve, operationInterval));
        }
      } else {
        console.log('Cool down phase - no operations, just monitoring...');
        await new Promise(resolve =>
          setTimeout(resolve, phase.durationMinutes * 60 * 1000)
        );
      }

      const health = await connectionManager.checkHealth();
      console.log(
        `Phase complete: ${phaseOperations} operations, ${health.healthy}/${currentConnections} healthy`
      );

      const stats = connectionManager.getStats();
      console.log(
        `  Avg Latency: ${stats.avgLatency.toFixed(2)}ms, P95: ${stats.p95Latency.toFixed(2)}ms, Errors: ${stats.totalErrors}`
      );
    }

    console.log('\nAll phases complete, generating final report...');

    const report = uptimeLogger.stopLogging();
    const savedFiles = uptimeLogger.saveReport();

    console.log(`Reports saved to: ${savedFiles.join(', ')}`);

    await connectionManager.closeAll();

    expect(report.summary.totalOperations).toBeGreaterThan(0);
    expect(report.summary.errorRate).toBeLessThan(5);
    expect(report.summary.p95Latency).toBeLessThan(500);
    expect(report.summary.healthScore).toBeGreaterThan(60);

    console.log(
      `✓ Gradual load increase test passed! Health score: ${report.summary.healthScore.toFixed(0)}/100`
    );
    console.log(
      `  Total operations: ${report.summary.totalOperations}, Error rate: ${report.summary.errorRate.toFixed(2)}%`
    );
    console.log(
      `  P95 latency: ${report.summary.p95Latency.toFixed(2)}ms, Memory growth: ${report.summary.memoryGrowthMB.toFixed(2)}MB`
    );
  });

  test('should maintain low latency under increasing load', async ({
    browser,
    serverInstance,
  }) => {
    const testName = 'gradual-load-latency-test';

    const phases: LoadPhase[] = [
      {
        name: 'Baseline',
        targetClients: 5,
        durationMinutes: 3,
        operationsPerMinute: 60,
      },
      {
        name: 'Light',
        targetClients: 10,
        durationMinutes: 3,
        operationsPerMinute: 60,
      },
      {
        name: 'Medium',
        targetClients: 20,
        durationMinutes: 3,
        operationsPerMinute: 60,
      },
      {
        name: 'Heavy',
        targetClients: 30,
        durationMinutes: 3,
        operationsPerMinute: 60,
      },
    ];

    const metricsCollector = new MetricsCollector(testName);
    const connectionManager = new ConnectionManager(browser, serverInstance);
    const uptimeLogger = new UptimeLogger(
      testName,
      metricsCollector,
      serverProfiler
    );

    uptimeLogger.setConnectionManager(connectionManager);
    uptimeLogger.startLogging(20000);

    let currentConnections = 0;
    let allConnectionIds: string[] = [];
    const phaseLatencies: number[] = [];

    for (const phase of phases) {
      console.log(`\n--- ${phase.name}: ${phase.targetClients} clients ---`);

      const clientsToAdd = phase.targetClients - currentConnections;
      if (clientsToAdd > 0) {
        const newConnectionIds = await connectionManager.createConnections(
          clientsToAdd,
          `latency-phase-${phases.indexOf(phase)}`
        );

        for (const connId of newConnectionIds) {
          const conn = connectionManager.getConnection(connId);
          if (conn) {
            await conn.page.getByTestId('enable-sync-btn').click();
            await conn.page.waitForSelector(
              '[data-testid="sync-status"]:has-text("Enabled")',
              { timeout: 10000 }
            );
          }
        }

        allConnectionIds = [...allConnectionIds, ...newConnectionIds];
        currentConnections = phase.targetClients;
      }

      const phaseStartTime = Date.now();
      const phaseEndTime = phaseStartTime + phase.durationMinutes * 60 * 1000;
      const operationInterval = (60 * 1000) / phase.operationsPerMinute;

      while (Date.now() < phaseEndTime) {
        const randomConnId =
          allConnectionIds[Math.floor(Math.random() * allConnectionIds.length)];

        const endOp = metricsCollector.startOperation();
        await connectionManager.executeOperation(randomConnId, async page => {
          await page.getByTestId('increment-btn').click();
          await page.waitForTimeout(50);
        });
        endOp();

        await new Promise(resolve => setTimeout(resolve, operationInterval));
      }

      const stats = connectionManager.getStats();
      phaseLatencies.push(stats.p95Latency);
      console.log(
        `Phase P95 latency: ${stats.p95Latency.toFixed(2)}ms (${currentConnections} clients)`
      );
    }

    const report = uptimeLogger.stopLogging();
    uptimeLogger.saveReport();

    await connectionManager.closeAll();

    for (let i = 1; i < phaseLatencies.length; i++) {
      const increase = phaseLatencies[i] - phaseLatencies[0];
      const percentIncrease = (increase / phaseLatencies[0]) * 100;
      console.log(
        `Phase ${i}: Latency increase from baseline: ${increase.toFixed(2)}ms (${percentIncrease.toFixed(1)}%)`
      );
      expect(percentIncrease).toBeLessThan(100);
    }

    expect(report.summary.errorRate).toBeLessThan(5);
    expect(report.summary.healthScore).toBeGreaterThan(60);

    console.log(
      `✓ Latency test passed! Max P95: ${Math.max(...phaseLatencies).toFixed(2)}ms`
    );
  });
});
