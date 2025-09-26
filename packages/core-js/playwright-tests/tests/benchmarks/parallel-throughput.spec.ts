import { test, expect } from '../fixtures';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';
import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';

test.describe('Parallel Operations Throughput Tests', () => {
  test('should achieve target parallel throughput (>500 ops/sec)', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const targetOpsPerSec = 500;
    const testDurationSec = 60; // 1 minute test
    const parallelism = 25; // Number of concurrent operations

    console.log(
      `Starting parallel throughput test: target ${targetOpsPerSec} ops/sec with ${parallelism} concurrent operations`
    );

    const metricsCollector = new MetricsCollector('parallel-throughput');
    const startTime = Date.now();
    let totalOperations = 0;

    // Generate test data upfront
    const images = await imageGenerator.generateBatch(200, [0.3, 1.0]);
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
              async (params: { imageData: number[]; imageName: string }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFile(
                  params.imageName,
                  new Uint8Array(params.imageData)
                );
              },
              {
                imageData: Array.from(image.data),
                imageName: `parallel-${workerIndex}-${workerOperations}.jpg`,
              }
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

    // Error rate should be minimal
    expect(metrics.errors.count).toBeLessThan(totalOperations * 0.02); // Less than 2% error rate

    // Latency should remain reasonable despite parallelism
    expect(metrics.latency.p95).toBeLessThan(200); // 95th percentile under 200ms
  });

  test('should scale with increasing parallelism levels', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const parallelismLevels = [1, 5, 10, 20, 50];
    const testDurationSec = 30; // 30 seconds per level
    const results: Array<{
      parallelism: number;
      throughput: number;
      latency: number;
    }> = [];

    console.log(
      `Testing scalability across parallelism levels: ${parallelismLevels.join(', ')}`
    );

    for (const parallelism of parallelismLevels) {
      console.log(`Testing with ${parallelism} parallel operations...`);

      const metricsCollector = new MetricsCollector(
        `scalability-${parallelism}`
      );
      const startTime = Date.now();
      let totalOperations = 0;

      // Generate test data for this level
      const images = await imageGenerator.generateBatch(100, [0.3, 1.0]);
      let imageIndex = 0;

      const getNextImage = () => {
        if (imageIndex >= images.length) imageIndex = 0;
        return images[imageIndex++];
      };

      // Run workers for this parallelism level
      const workers = Array.from(
        { length: parallelism },
        async (_, workerIndex) => {
          let workerOperations = 0;

          while (Date.now() - startTime < testDurationSec * 1000) {
            const image = getNextImage();
            const endOperation = metricsCollector.startOperation();

            try {
              await page.evaluate(
                async (params: { imageData: number[]; imageName: string }) => {
                  const vfsService = (window as any).vfsService;
                  await vfsService.writeFile(
                    params.imageName,
                    new Uint8Array(params.imageData)
                  );
                },
                {
                  imageData: Array.from(image.data),
                  imageName: `scale-${parallelism}-${workerIndex}-${workerOperations}.jpg`,
                }
              );

              endOperation();
              metricsCollector.recordBytes(image.size);
              workerOperations++;
            } catch (error) {
              endOperation();
              metricsCollector.recordError(`scale-worker-${workerIndex}`);
            }
          }

          return workerOperations;
        }
      );

      const workerResults = await Promise.all(workers);
      totalOperations = workerResults.reduce((sum, count) => sum + count, 0);

      const actualDuration = (Date.now() - startTime) / 1000;
      const throughput = totalOperations / actualDuration;
      const metrics = await metricsCollector.getMetrics();

      results.push({
        parallelism: parallelism,
        throughput: throughput,
        latency: metrics.latency.mean,
      });

      console.log(
        `${parallelism} workers: ${throughput.toFixed(1)} ops/sec, ${metrics.latency.mean.toFixed(1)}ms avg latency`
      );
    }

    // Analyze scalability
    console.log('Scalability results:', results);

    // Throughput should generally increase with parallelism (up to a point)
    const singleThreaded = results.find(r => r.parallelism === 1);
    const highParallelism = results.find(r => r.parallelism === 20);

    if (singleThreaded && highParallelism) {
      const scalingFactor =
        highParallelism.throughput / singleThreaded.throughput;
      console.log(`Scaling factor (20x vs 1x): ${scalingFactor.toFixed(2)}x`);

      // Should achieve at least 5x improvement with 20x parallelism
      expect(scalingFactor).toBeGreaterThan(5);
    }

    // All levels should achieve reasonable performance
    results.forEach(result => {
      expect(result.throughput).toBeGreaterThan(20); // Minimum baseline
      expect(result.latency).toBeLessThan(1000); // Latency shouldn't exceed 1 second
    });

    // Higher parallelism should achieve higher throughput (with diminishing returns)
    for (let i = 1; i < results.length; i++) {
      const current = results[i];
      const previous = results[i - 1];

      // Skip the check for very high parallelism where contention might reduce throughput
      if (current.parallelism <= 20) {
        expect(current.throughput).toBeGreaterThanOrEqual(
          previous.throughput * 0.8
        );
      }
    }
  });

  test('should handle mixed parallel read/write workloads', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const testDurationSec = 60; // 1 minute test
    const writeWorkers = 15;
    const readWorkers = 10;
    const totalWorkers = writeWorkers + readWorkers;

    console.log(
      `Testing mixed workload: ${writeWorkers} write workers, ${readWorkers} read workers`
    );

    const metricsCollector = new MetricsCollector('mixed-parallel-workload');
    const startTime = Date.now();

    // Pre-populate some files for reading
    console.log('Pre-populating files for read operations...');
    const initialFiles = await imageGenerator.generateBatch(50, [0.5, 1.5]);
    for (let i = 0; i < initialFiles.length; i++) {
      const image = initialFiles[i];
      await page.evaluate(
        async (params: { imageData: number[]; imageName: string }) => {
          const vfsService = (window as any).vfsService;
          await vfsService.writeFile(
            params.imageName,
            new Uint8Array(params.imageData)
          );
        },
        { imageData: Array.from(image.data), imageName: `initial-${i}.jpg` }
      );
    }

    // Generate images for write operations
    const writeImages = await imageGenerator.generateBatch(200, [0.3, 1.0]);
    let writeImageIndex = 0;
    let writeFileCounter = 0;

    const getNextWriteImage = () => {
      if (writeImageIndex >= writeImages.length) writeImageIndex = 0;
      return writeImages[writeImageIndex++];
    };

    // Write workers
    const writeWorkerPromises = Array.from(
      { length: writeWorkers },
      async (_, workerIndex) => {
        let operations = 0;

        while (Date.now() - startTime < testDurationSec * 1000) {
          const image = getNextWriteImage();
          const endOperation = metricsCollector.startOperation();

          try {
            await page.evaluate(
              async (params: { imageData: number[]; imageName: string }) => {
                const vfsService = (window as any).vfsService;
                await vfsService.writeFile(
                  params.imageName,
                  new Uint8Array(params.imageData)
                );
              },
              {
                imageData: Array.from(image.data),
                imageName: `mixed-write-${workerIndex}-${writeFileCounter++}.jpg`,
              }
            );

            endOperation();
            metricsCollector.recordBytes(image.size);
            operations++;
          } catch (error) {
            endOperation();
            metricsCollector.recordError(`write-worker-${workerIndex}`);
          }
        }

        console.log(
          `Write worker ${workerIndex} completed ${operations} operations`
        );
        return { type: 'write', operations };
      }
    );

    // Read workers
    const readWorkerPromises = Array.from(
      { length: readWorkers },
      async (_, workerIndex) => {
        let operations = 0;

        while (Date.now() - startTime < testDurationSec * 1000) {
          const endOperation = metricsCollector.startOperation();

          try {
            // Read from initial files or recently written files
            const totalAvailableFiles = initialFiles.length + writeFileCounter;
            const fileIndex = Math.floor(
              Math.random() * Math.min(totalAvailableFiles, 100)
            );

            let fileName: string;
            if (fileIndex < initialFiles.length) {
              fileName = `initial-${fileIndex}.jpg`;
            } else {
              const writeFileIndex = fileIndex - initialFiles.length;
              const writeWorkerIndex = writeFileIndex % writeWorkers;
              const fileNumber = Math.floor(writeFileIndex / writeWorkers);
              fileName = `mixed-write-${writeWorkerIndex}-${fileNumber}.jpg`;
            }

            const content = await page.evaluate(
              async (params: { fileName: string }) => {
                const vfsService = (window as any).vfsService;
                try {
                  return await vfsService.readFile(params.fileName);
                } catch (error) {
                  return null; // File might not exist yet
                }
              },
              { fileName }
            );

            if (content) {
              metricsCollector.recordBytes((content as Uint8Array).length);
            }

            endOperation();
            operations++;
          } catch (error) {
            endOperation();
            metricsCollector.recordError(`read-worker-${workerIndex}`);
          }
        }

        console.log(
          `Read worker ${workerIndex} completed ${operations} operations`
        );
        return { type: 'read', operations };
      }
    );

    // Wait for all workers to complete
    const allResults = await Promise.all([
      ...writeWorkerPromises,
      ...readWorkerPromises,
    ]);

    const writeResults = allResults.filter(r => r.type === 'write');
    const readResults = allResults.filter(r => r.type === 'read');

    const totalWriteOps = writeResults.reduce(
      (sum, r) => sum + r.operations,
      0
    );
    const totalReadOps = readResults.reduce((sum, r) => sum + r.operations, 0);
    const totalOperations = totalWriteOps + totalReadOps;

    const actualDuration = (Date.now() - startTime) / 1000;
    const overallThroughput = totalOperations / actualDuration;
    const writeThroughput = totalWriteOps / actualDuration;
    const readThroughput = totalReadOps / actualDuration;
    const metrics = await metricsCollector.getMetrics();

    console.log(`Mixed parallel workload completed:`);
    console.log(`- Duration: ${actualDuration.toFixed(1)}s`);
    console.log(`- Total operations: ${totalOperations}`);
    console.log(
      `- Write operations: ${totalWriteOps} (${writeThroughput.toFixed(1)} ops/sec)`
    );
    console.log(
      `- Read operations: ${totalReadOps} (${readThroughput.toFixed(1)} ops/sec)`
    );
    console.log(
      `- Overall throughput: ${overallThroughput.toFixed(1)} ops/sec`
    );
    console.log(`- Errors: ${metrics.errors.count}`);

    // Performance assertions
    expect(overallThroughput).toBeGreaterThan(300); // At least 300 ops/sec combined
    expect(writeThroughput).toBeGreaterThan(100); // At least 100 write ops/sec
    expect(readThroughput).toBeGreaterThan(150); // At least 150 read ops/sec

    // Error rate should be low
    expect(metrics.errors.count).toBeLessThan(totalOperations * 0.03); // Less than 3% error rate

    // Both operation types should complete reasonable amounts
    expect(totalWriteOps).toBeGreaterThan(testDurationSec * 100 * 0.8); // 80% of target write ops
    expect(totalReadOps).toBeGreaterThan(testDurationSec * 150 * 0.8); // 80% of target read ops
  });

  test('should maintain parallel performance under resource contention', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const testDurationSec = 45; // 45 seconds
    const parallelism = 30; // High parallelism to create contention

    console.log(
      `Testing resource contention with ${parallelism} parallel operations`
    );

    const metricsCollector = new MetricsCollector('resource-contention');
    const startTime = Date.now();
    let totalOperations = 0;
    const completionTimes: number[] = [];

    // Generate diverse test data to create different load patterns
    const images = await imageGenerator.generateBatch(150, [0.2, 2.0]); // Wide size range
    let imageIndex = 0;

    const getNextImage = () => {
      if (imageIndex >= images.length) imageIndex = 0;
      return images[imageIndex++];
    };

    // Create high contention by having all workers compete for resources
    const workers = Array.from(
      { length: parallelism },
      async (_, workerIndex) => {
        let workerOperations = 0;
        const workerStartTime = Date.now();

        while (Date.now() - startTime < testDurationSec * 1000) {
          const image = getNextImage();
          const operationStartTime = Date.now();
          const endOperation = metricsCollector.startOperation();

          try {
            // Mix of different operation types to create varied load
            if (workerOperations % 5 === 0) {
              // Occasionally perform read operations to test read/write contention
              const readFileName = `contention-${Math.floor(Math.random() * Math.min(workerOperations, 50))}.jpg`;

              await page.evaluate(
                async (params: { fileName: string }) => {
                  const vfsService = (window as any).vfsService;
                  try {
                    await vfsService.readFile(params.fileName);
                  } catch (error) {
                    // File might not exist, that's okay
                  }
                },
                { fileName: readFileName }
              );
            } else {
              // Regular write operation
              await page.evaluate(
                async (params: { imageData: number[]; imageName: string }) => {
                  const vfsService = (window as any).vfsService;
                  await vfsService.writeFile(
                    params.imageName,
                    new Uint8Array(params.imageData)
                  );
                },
                {
                  imageData: Array.from(image.data),
                  imageName: `contention-${workerOperations}.jpg`,
                }
              );

              metricsCollector.recordBytes(image.size);
            }

            const operationDuration = Date.now() - operationStartTime;
            completionTimes.push(operationDuration);

            endOperation();
            workerOperations++;

            // Occasionally yield to increase contention patterns
            if (workerOperations % 10 === 0) {
              await new Promise(resolve => setTimeout(resolve, 1));
            }
          } catch (error) {
            endOperation();
            metricsCollector.recordError(`contention-worker-${workerIndex}`);
          }
        }

        const workerDuration = (Date.now() - workerStartTime) / 1000;
        console.log(
          `Worker ${workerIndex}: ${workerOperations} ops in ${workerDuration.toFixed(1)}s (${(workerOperations / workerDuration).toFixed(1)} ops/sec)`
        );

        return workerOperations;
      }
    );

    const workerResults = await Promise.all(workers);
    totalOperations = workerResults.reduce((sum, count) => sum + count, 0);

    const actualDuration = (Date.now() - startTime) / 1000;
    const overallThroughput = totalOperations / actualDuration;
    const metrics = await metricsCollector.getMetrics();

    // Analyze operation timing distribution
    completionTimes.sort((a, b) => a - b);
    const p50 = completionTimes[Math.floor(completionTimes.length * 0.5)];
    const p95 = completionTimes[Math.floor(completionTimes.length * 0.95)];
    const p99 = completionTimes[Math.floor(completionTimes.length * 0.99)];

    console.log(`Resource contention test completed:`);
    console.log(`- Duration: ${actualDuration.toFixed(1)}s`);
    console.log(`- Total operations: ${totalOperations}`);
    console.log(
      `- Overall throughput: ${overallThroughput.toFixed(1)} ops/sec`
    );
    console.log(`- Parallelism: ${parallelism}`);
    console.log(`- Operation latency P50/P95/P99: ${p50}ms/${p95}ms/${p99}ms`);
    console.log(`- Errors: ${metrics.errors.count}`);

    // Performance assertions under contention
    expect(overallThroughput).toBeGreaterThan(200); // Should maintain at least 200 ops/sec under contention
    expect(totalOperations).toBeGreaterThan(testDurationSec * 200 * 0.7); // At least 70% of target

    // Error rate should remain reasonable even under contention
    expect(metrics.errors.count).toBeLessThan(totalOperations * 0.05); // Less than 5% error rate

    // Latency should not become excessive under contention
    expect(p95).toBeLessThan(500); // 95th percentile under 500ms
    expect(p99).toBeLessThan(1000); // 99th percentile under 1 second

    // All workers should complete a reasonable number of operations
    workerResults.forEach((operations, index) => {
      expect(operations).toBeGreaterThan(testDurationSec * 5); // At least 5 ops/sec per worker
    });
  });
});
