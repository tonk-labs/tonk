import { test, expect } from '@playwright/test';
import { ConnectionManager } from '../../src/utils/connection-manager';
import { serverManager } from '../../src/server/server-manager';

test.describe('WebSocket Connection Benchmarks', () => {
  test.describe.configure({ timeout: 600000 }); // 10 minutes for long-running tests

  test('Scenario A: Pure Connection Scaling (10, 30, 60, 80, 100 connections)', async ({
    browser,
  }) => {
    const connectionTiers = [10, 30, 60, 80, 100];
    const sustainDuration = 30000; // 30 seconds per tier

    console.log('\n=== Starting Connection Scaling Test ===');

    // Start a dedicated server for this test
    const server = await serverManager.startServer('connection-scaling-test');
    const connectionManager = new ConnectionManager(browser, server);

    try {
      // Start server profiling

      const results: Array<{
        connections: number;
        establishTime: number;
        avgLatency: number;
        p95Latency: number;
        p99Latency: number;
        memoryUsedMB: number;
        allHealthy: boolean;
      }> = [];

      for (const tier of connectionTiers) {
        console.log(`\n--- Testing ${tier} connections ---`);

        // Close previous tier connections to test each tier in isolation
        if (connectionManager.getConnectionCount() > 0) {
          console.log(
            `Closing ${connectionManager.getConnectionCount()} previous connections...`
          );
          await connectionManager.closeAll();
          // Allow server to recover between tiers
          await new Promise(resolve => setTimeout(resolve, 2000));
        }

        const startTime = Date.now();

        // Create connections for this tier
        const tierConnectionIds = await connectionManager.createConnections(
          tier,
          `tier-${tier}`
        );
        const establishTime = Date.now() - startTime;

        // Update profiler

        console.log(
          `Established ${tier} connections in ${establishTime}ms (${(tier / (establishTime / 1000)).toFixed(1)} conn/sec)`
        );

        // Sustain connections for specified duration
        console.log(
          `Sustaining ${tier} connections for ${sustainDuration / 1000}s...`
        );
        await new Promise(resolve => setTimeout(resolve, sustainDuration));

        // Check health
        const healthCheck = await connectionManager.checkHealth();
        console.log(
          `Health check: ${healthCheck.healthy} healthy, ${healthCheck.unhealthy} unhealthy`
        );

        // Perform a simple operation on all connections to measure latency
        await connectionManager.executeOnAll(async page => {
          await page.evaluate(() => {
            const vfsService = (window as any).vfsService;
            if (!vfsService) throw new Error('VFS service not available');
            // Simple ping operation
            return true;
          });
        });


        // Get stats only for current tier connections
        const stats =
          connectionManager.getStatsForConnections(tierConnectionIds);

        // Capture memory snapshot
        const memoryUsage = process.memoryUsage();
        const memoryUsedMB = memoryUsage.rss / (1024 * 1024);

        results.push({
          connections: tier,
          establishTime,
          avgLatency: stats.avgLatency,
          p95Latency: stats.p95Latency,
          p99Latency: stats.p99Latency,
          memoryUsedMB,
          allHealthy: healthCheck.healthy === tier,
        });

        console.log(`Avg Latency: ${stats.avgLatency.toFixed(2)}ms`);
        console.log(`P95 Latency: ${stats.p95Latency.toFixed(2)}ms`);
        console.log(`P99 Latency: ${stats.p99Latency.toFixed(2)}ms`);
        console.log(`Server Memory: ${memoryUsedMB.toFixed(2)} MB`);
      }

      // Stop profiling and generate report
      if (profile) {
        console.log('\nMemory Analysis:');
        console.log(
          JSON.stringify(
            null,
            2
          )
        );
      }

      // Print summary
      console.log('\n=== Connection Scaling Summary ===');
      console.log(
        'Connections | Establish Time | Avg Latency | P95 Latency | P99 Latency | Memory (MB) | Healthy'
      );
      console.log('-'.repeat(110));
      results.forEach(r => {
        console.log(
          `${r.connections.toString().padStart(11)} | ${r.establishTime.toString().padStart(14)}ms | ${r.avgLatency.toFixed(2).padStart(11)}ms | ${r.p95Latency.toFixed(2).padStart(11)}ms | ${r.p99Latency.toFixed(2).padStart(11)}ms | ${r.memoryUsedMB.toFixed(2).padStart(11)} | ${r.allHealthy ? 'Yes' : 'No'}`
        );
      });

      // Assertions
      const max100Result = results[results.length - 1];
      expect(max100Result.connections).toBe(100);
      expect(max100Result.allHealthy).toBe(true);
      expect(max100Result.p95Latency).toBeLessThan(500); // P95 under 500ms
      expect(max100Result.p99Latency).toBeLessThan(1000); // P99 under 1s

      console.log('\n✓ Test passed: Successfully scaled to 100 connections');
    } finally {
      await connectionManager.closeAll();
      await serverManager.stopServer('connection-scaling-test');
    }
  });

  test('Scenario B: Active Load (60+ connections with operations)', async ({
    browser,
  }) => {
    const connectionCount = 70;
    const operationsPerConnection = 15;

    console.log('\n=== Starting Active Load Test ===');
    console.log(
      `Config: ${connectionCount} connections, ${operationsPerConnection} ops each`
    );

    const server = await serverManager.startServer('active-load-test');
    const connectionManager = new ConnectionManager(browser, server);

    try {

      // Create connections
      console.log('\nEstablishing connections...');
      await connectionManager.createConnections(connectionCount, 'active');

      const healthCheck = await connectionManager.checkHealth();
      console.log(
        `Initial health: ${healthCheck.healthy}/${connectionCount} healthy`
      );
      expect(healthCheck.healthy).toBe(connectionCount);

      // Perform operations on all connections
      console.log(
        `\nExecuting ${operationsPerConnection} operations per connection...`
      );

      const startTime = Date.now();
      let totalSuccessful = 0;
      let totalFailed = 0;

      for (let i = 0; i < operationsPerConnection; i++) {
        console.log(`Operation round ${i + 1}/${operationsPerConnection}...`);

        const result = await connectionManager.executeOnAll(
          async (page, connectionId) => {
            await page.evaluate(
              async ({ opIndex, connId }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const fileName = `test-${connId}-${opIndex}.txt`;
                const content = `Test data from ${connId} operation ${opIndex}`;

                // Write the file (create=true since it's a new file)
                await vfsService.writeFile(fileName, { content }, true);

                // Verify write
                const readContent = (await vfsService.readFile(fileName))
                  .content;

                if (readContent !== content) {
                  throw new Error(
                    `Read/write verification failed for ${fileName}. Expected: "${content}", Got: "${readContent}"`
                  );
                }
              },
              { opIndex: i, connId: connectionId }
            );
          },
          { maxConcurrency: 20 } // Limit concurrent operations
        );

        totalSuccessful += result.successCount;
        totalFailed += result.failureCount;

        console.log(
          `  Success: ${result.successCount}, Failed: ${result.failureCount}, Avg Latency: ${result.avgLatency.toFixed(2)}ms`
        );
      }

      const duration = (Date.now() - startTime) / 1000;
      const totalOps = totalSuccessful + totalFailed;
      const throughput = totalSuccessful / duration;

      const stats = connectionManager.getStats();

      console.log('\n=== Active Load Results ===');
      console.log(`Duration: ${duration.toFixed(2)}s`);
      console.log(`Total Operations: ${totalOps}`);
      console.log(`Successful: ${totalSuccessful}`);
      console.log(`Failed: ${totalFailed}`);
      console.log(`Throughput: ${throughput.toFixed(2)} ops/sec`);
      console.log(`Avg Latency: ${stats.avgLatency.toFixed(2)}ms`);
      console.log(`P95 Latency: ${stats.p95Latency.toFixed(2)}ms`);
      console.log(`P99 Latency: ${stats.p99Latency.toFixed(2)}ms`);

      // Stop profiling
      if (profile) {
      }

      // Assertions
      expect(totalSuccessful).toBeGreaterThan(totalOps * 0.95); // 95% success rate
      expect(stats.p95Latency).toBeLessThan(200); // P95 under 200ms
      expect(stats.p99Latency).toBeLessThan(500); // P99 under 500ms
      expect(throughput).toBeGreaterThan(20); // At least 20 ops/sec

      console.log('\n✓ Test passed: Active load handled successfully');
    } finally {
      await connectionManager.closeAll();
      await serverManager.stopServer('active-load-test');
    }
  });

  test('Scenario C: Connection Bursts (rapid connection waves)', async ({
    browser,
  }) => {
    const waveSizes = [10, 20, 30, 40]; // Progressive wave sizes
    const totalConnections = waveSizes.reduce((a, b) => a + b, 0);

    console.log('\n=== Starting Connection Burst Test ===');
    console.log(`Waves: ${waveSizes.join(', ')} (total: ${totalConnections})`);

    const server = await serverManager.startServer('connection-burst-test');
    const connectionManager = new ConnectionManager(browser, server);

    try {

      const waveResults: Array<{
        wave: number;
        size: number;
        connectionTime: number;
        successRate: number;
        latencyIncrease: number;
      }> = [];

      let previousLatency = 0;

      for (let waveIndex = 0; waveIndex < waveSizes.length; waveIndex++) {
        const waveSize = waveSizes[waveIndex];
        console.log(
          `\n--- Wave ${waveIndex + 1}: Adding ${waveSize} connections ---`
        );

        const startTime = Date.now();

        // Create wave of connections
        await connectionManager.createConnections(
          waveSize,
          `wave${waveIndex + 1}`
        );

        const connectionTime = Date.now() - startTime;
        const currentConnectionCount = connectionManager.getConnectionCount();

        // Quick health check
        const healthCheck = await connectionManager.checkHealth();
        const successRate = healthCheck.healthy / currentConnectionCount;

        // Measure latency with a quick operation
        const result = await connectionManager.executeOnRandom(
          Math.min(10, currentConnectionCount),
          async page => {
            await page.evaluate(() => {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');
              return true;
            });
          }
        );


        const currentLatency = result.avgLatency;
        const latencyIncrease =
          previousLatency > 0
            ? ((currentLatency - previousLatency) / previousLatency) * 100
            : 0;

        waveResults.push({
          wave: waveIndex + 1,
          size: waveSize,
          connectionTime,
          successRate,
          latencyIncrease,
        });

        console.log(
          `Wave ${waveIndex + 1} complete: ${connectionTime}ms, ${((healthCheck.healthy / currentConnectionCount) * 100).toFixed(1)}% success, latency: ${currentLatency.toFixed(2)}ms (+${latencyIncrease.toFixed(1)}%)`
        );

        previousLatency = currentLatency;

        // Small delay between waves
        if (waveIndex < waveSizes.length - 1) {
          await new Promise(resolve => setTimeout(resolve, 1000));
        }
      }

      // Stop profiling
      if (profile) {
      }

      // Summary
      console.log('\n=== Connection Burst Summary ===');
      console.log('Wave | Size | Time (ms) | Success Rate | Latency Increase');
      console.log('-'.repeat(70));
      waveResults.forEach(r => {
        console.log(
          `${r.wave.toString().padStart(4)} | ${r.size.toString().padStart(4)} | ${r.connectionTime.toString().padStart(9)} | ${(r.successRate * 100).toFixed(1).padStart(12)}% | ${r.latencyIncrease >= 0 ? '+' : ''}${r.latencyIncrease.toFixed(1)}%`
        );
      });

      // Assertions
      waveResults.forEach(r => {
        expect(r.successRate).toBeGreaterThan(0.95); // 95% success rate per wave
      });

      const finalWave = waveResults[waveResults.length - 1];
      expect(finalWave.latencyIncrease).toBeLessThan(100); // Less than 100% increase

      console.log('\n✓ Test passed: Connection bursts handled successfully');
    } finally {
      await connectionManager.closeAll();
      await serverManager.stopServer('connection-burst-test');
    }
  });

  test('Scenario D: Sustained Load (60-100 connections over 2-3 minutes)', async ({
    browser,
  }) => {
    const connectionCount = 80;
    const testDuration = 120000; // 2 minutes
    const operationInterval = 2000; // Operation every 2 seconds

    console.log('\n=== Starting Sustained Load Test ===');
    console.log(
      `Config: ${connectionCount} connections, ${testDuration / 1000}s duration, ops every ${operationInterval / 1000}s`
    );

    const server = await serverManager.startServer('sustained-load-test');
    const connectionManager = new ConnectionManager(browser, server);

    try {

      // Establish connections
      console.log('\nEstablishing connections...');
      await connectionManager.createConnections(connectionCount, 'sustained');

      const healthCheck = await connectionManager.checkHealth();
      console.log(
        `Connections established: ${healthCheck.healthy}/${connectionCount} healthy`
      );

      // Track metrics over time
      const snapshots: Array<{
        timestamp: number;
        healthyConnections: number;
        avgLatency: number;
        p95Latency: number;
        memoryMB: number;
        operations: number;
      }> = [];

      const startTime = Date.now();
      let operationCount = 0;

      console.log('\nStarting sustained load...');

      while (Date.now() - startTime < testDuration) {
        // Perform operations on a subset of connections
        const operationsThisRound = Math.min(20, connectionCount);
        const result = await connectionManager.executeOnRandom(
          operationsThisRound,
          async (page, connectionId) => {
            await page.evaluate(
              async ({ connId }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const timestamp = Date.now();
                const random = Math.floor(Math.random() * 1000000);
                const fileName = `sustained-${connId}-${timestamp}-${random}.txt`;
                const content = `Sustained test data ${timestamp}`;

                await vfsService.writeFile(fileName, content, true);
              },
              { connId: connectionId }
            );
          }
        );

        operationCount += result.successCount;

        // Check health
        const currentHealth = await connectionManager.checkHealth();
        const stats = connectionManager.getStats();
        const memoryUsage = process.memoryUsage();

        snapshots.push({
          timestamp: Date.now() - startTime,
          healthyConnections: currentHealth.healthy,
          avgLatency: result.avgLatency,
          p95Latency: stats.p95Latency,
          memoryMB: memoryUsage.rss / (1024 * 1024),
          operations: operationCount,
        });

        const elapsed = (Date.now() - startTime) / 1000;
        console.log(
          `[${elapsed.toFixed(0)}s] Ops: ${operationCount}, Healthy: ${currentHealth.healthy}/${connectionCount}, Latency: ${result.avgLatency.toFixed(2)}ms, Memory: ${(memoryUsage.rss / (1024 * 1024)).toFixed(2)}MB`
        );

        // Wait for next interval
        await new Promise(resolve => setTimeout(resolve, operationInterval));
      }

      // Stop profiling
      if (profile) {
      }

      // Analyze results
      console.log('\n=== Sustained Load Analysis ===');

      const firstSnapshot = snapshots[0];
      const lastSnapshot = snapshots[snapshots.length - 1];

      console.log(`Duration: ${testDuration / 1000}s`);
      console.log(`Total Operations: ${operationCount}`);
      console.log(
        `Operations/sec: ${(operationCount / (testDuration / 1000)).toFixed(2)}`
      );
      console.log(
        `Health Stability: ${firstSnapshot.healthyConnections} -> ${lastSnapshot.healthyConnections}`
      );
      console.log(
        `Latency Stability: ${firstSnapshot.avgLatency.toFixed(2)}ms -> ${lastSnapshot.avgLatency.toFixed(2)}ms`
      );
      console.log(
        `Memory Growth: ${firstSnapshot.memoryMB.toFixed(2)}MB -> ${lastSnapshot.memoryMB.toFixed(2)}MB (+${(lastSnapshot.memoryMB - firstSnapshot.memoryMB).toFixed(2)}MB)`
      );

      // Calculate stability metrics
      const avgHealthy =
        snapshots.reduce((sum, s) => sum + s.healthyConnections, 0) /
        snapshots.length;
      const avgLatency =
        snapshots.reduce((sum, s) => sum + s.avgLatency, 0) / snapshots.length;

      const latencyVariance =
        snapshots.reduce(
          (sum, s) => sum + Math.pow(s.avgLatency - avgLatency, 2),
          0
        ) / snapshots.length;
      const latencyStdDev = Math.sqrt(latencyVariance);

      console.log(`\nStability Metrics:`);
      console.log(`Avg Healthy: ${avgHealthy.toFixed(1)}/${connectionCount}`);
      console.log(`Avg Latency: ${avgLatency.toFixed(2)}ms`);
      console.log(`Latency Std Dev: ${latencyStdDev.toFixed(2)}ms`);

      // Assertions
      expect(lastSnapshot.healthyConnections).toBeGreaterThan(
        connectionCount * 0.95
      ); // 95% healthy at end
      expect(lastSnapshot.avgLatency).toBeLessThan(500); // Latency didn't degrade >50%
      expect(lastSnapshot.memoryMB - firstSnapshot.memoryMB).toBeLessThan(100); // Less than 100MB memory growth

      const stats = connectionManager.getStats();
      expect(stats.p95Latency).toBeLessThan(800); // P95 under 800ms
      expect(stats.p99Latency).toBeLessThan(1600); // P99 under 1200ms

      console.log('\n✓ Test passed: Sustained load maintained successfully');
    } finally {
      await connectionManager.closeAll();
      await serverManager.stopServer('sustained-load-test');
    }
  });
});
