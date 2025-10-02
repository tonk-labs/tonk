import { test, expect } from '../fixtures';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';
import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';

test.describe('Parallel Operations Throughput Tests', () => {
  test('should achieve target parallel throughput (>30 ops/sec)', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const targetOpsPerSec = 30;
    const testDurationSec = 60; // 1 minute test
    const parallelism = 25; // Number of concurrent operations

    console.log(
      `Starting parallel throughput test: target ${targetOpsPerSec} ops/sec with ${parallelism} concurrent operations`
    );

    const metricsCollector = new MetricsCollector('parallel-throughput');
    const startTime = Date.now();
    let totalOperations = 0;

    // Generate test data upfront
    const images = await imageGenerator.generateBatchForTest(
      'parallel-throughput-test',
      200,
      [0.3, 1.0]
    );
    let imageIndex = 0;

    // Function to get next image cyclically
    const getNextImage = () => {
      if (imageIndex >= images.length) imageIndex = 0;
      return images[imageIndex++];
    };

    // Run parallel operations
    const workers = Array.from(
      { length: parallelism },
      async (_, workerIndex) => {
        let workerOperations = 0;

        while (Date.now() - startTime < testDurationSec * 1000) {
          const image = getNextImage();
          const endOperation = metricsCollector.startOperation();

          try {
            await page.evaluate(
              async ({ image }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFile(image.name, image.data);
              },
              { image }
            );

            endOperation();
            metricsCollector.recordBytes(image.size);
            workerOperations++;
          } catch (error) {
            endOperation();
            metricsCollector.recordError(`parallel-worker-${workerIndex}`);
          }
        }

        console.log(
          `Worker ${workerIndex} completed ${workerOperations} operations`
        );
        return workerOperations;
      }
    );

    // Wait for all workers to complete
    const workerResults = await Promise.all(workers);
    totalOperations = workerResults.reduce((sum, count) => sum + count, 0);

    const actualDuration = (Date.now() - startTime) / 1000;
    const actualThroughput = totalOperations / actualDuration;
    const metrics = await metricsCollector.getMetrics();

    console.log(`Parallel throughput test completed:`);
    console.log(`- Duration: ${actualDuration.toFixed(1)}s`);
    console.log(`- Operations: ${totalOperations}`);
    console.log(`- Throughput: ${actualThroughput.toFixed(1)} ops/sec`);
    console.log(`- Target: ${targetOpsPerSec} ops/sec`);
    console.log(`- Parallelism: ${parallelism}`);
    console.log(`- Errors: ${metrics.errors.count}`);

    // Performance assertions
    expect(actualThroughput).toBeGreaterThan(targetOpsPerSec);
    expect(totalOperations).toBeGreaterThan(
      targetOpsPerSec * testDurationSec * 0.8
    ); // At least 80% of target

    // No errors
    expect(metrics.errors.count).toBe(0);

    // Latency should remain reasonable despite parallelism
    expect(metrics.latency.p95).toBeLessThan(1000); // 95th percentile under 1sec
  });
});
