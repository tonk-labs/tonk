import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import {
  expect,
  setupTestWithServer,
  test,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Sequential Operations Throughput Tests', () => {
  test('should achieve target sequential throughput (>100 ops/sec)', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const targetOpsPerSec = 100;
    const testDurationSec = 60; // 1 minute test
    const batchSize = 20; // Number of operations to batch together

    console.log(
      `Starting sequential throughput test: target ${targetOpsPerSec} ops/sec for ${testDurationSec} seconds`
    );

    const metricsCollector = new MetricsCollector('sequential-throughput');
    const startTime = Date.now();
    let operationCount = 0;

    // Pre-generate enough test data upfront to avoid mid-test regeneration
    const totalExpectedOps = targetOpsPerSec * testDurationSec * 1.2; // 20% buffer
    console.log(`Pre-generating ${totalExpectedOps} images...`);
    const allImages = await imageGenerator.generateBatchForTest(
      'sequential-throughput-batch',
      totalExpectedOps,
      [0.5, 1.5]
    );
    let imageIndex = 0;

    while (Date.now() - startTime < testDurationSec * 1000) {
      // Prepare batch of operations
      const batchOperations = [];
      for (let i = 0; i < batchSize && imageIndex < allImages.length; i++) {
        const image = allImages[imageIndex++];
        batchOperations.push({
          image,
          imageName: `seq-${operationCount + i}.jpg`,
        });
      }

      if (batchOperations.length === 0) {
        // Reset and reuse images if we run out
        imageIndex = 0;
        continue;
      }

      const batchStartTime = Date.now();

      try {
        // Execute batch of writes in a single page.evaluate call
        const results = await page.evaluate(
          async ({ operations }) => {
            const vfsService = (window as any).vfsService;
            if (!vfsService) throw new Error('VFS service not available');

            const metrics = [];
            for (const op of operations) {
              const opStart = performance.now();
              try {
                await vfsService.writeFile(op.imageName, op.image.data);
                metrics.push({
                  success: true,
                  duration: performance.now() - opStart,
                  bytes: op.image.size,
                });
              } catch (error) {
                metrics.push({
                  success: false,
                  duration: performance.now() - opStart,
                  error: String(error),
                });
              }
            }
            return metrics;
          },
          { operations: batchOperations }
        );

        // Record metrics for each operation in the batch
        results.forEach(result => {
          if (result.success) {
            metricsCollector.recordBytes(result.bytes);
            operationCount++;
          } else {
            metricsCollector.recordError('sequential-write');
            console.error('Operation failed:', result.error);
          }
        });

        // Log progress every 100 operations
        if (operationCount % 100 === 0) {
          const elapsed = (Date.now() - startTime) / 1000;
          const currentThroughput = operationCount / elapsed;
          console.log(
            `${operationCount} ops in ${elapsed.toFixed(1)}s = ${currentThroughput.toFixed(1)} ops/sec`
          );
        }
      } catch (error) {
        metricsCollector.recordError('sequential-write-batch');
        console.error('Batch operation failed:', error);
      }
    }

    const totalDuration = (Date.now() - startTime) / 1000;
    const actualThroughput = operationCount / totalDuration;
    const metrics = await metricsCollector.getMetrics();

    console.log(`Sequential throughput test completed:`);
    console.log(`- Duration: ${totalDuration.toFixed(1)}s`);
    console.log(`- Operations: ${operationCount}`);
    console.log(`- Throughput: ${actualThroughput.toFixed(1)} ops/sec`);
    console.log(`- Target: ${targetOpsPerSec} ops/sec`);
    console.log(`- Errors: ${metrics.errors.count}`);

    // Performance assertions
    expect(actualThroughput).toBeGreaterThan(targetOpsPerSec);
    expect(operationCount).toBeGreaterThan(
      targetOpsPerSec * testDurationSec * 0.8
    ); // At least 80% of target

    // Error rate should be minimal
    expect(metrics.errors.count).toBeLessThan(operationCount * 0.01); // Less than 1% error rate

    // Latency should be reasonable
    expect(metrics.latency.p95).toBeLessThan(100); // 95th percentile under 100ms
    expect(metrics.latency.mean).toBeLessThan(50); // Mean latency under 50ms
  });

  test('should maintain consistent sequential performance over time', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const intervalSec = 30; // Measure every 30 seconds
    const totalIntervals = 6; // 3 minutes total
    const batchSize = 15; // Batch operations together
    const results: Array<{
      interval: number;
      throughput: number;
      latency: number;
    }> = [];

    console.log(
      `Starting consistency test: ${totalIntervals} intervals of ${intervalSec}s each`
    );

    // Pre-generate all images for all intervals upfront
    const imagesPerInterval = 100 * intervalSec; // Estimate ~100 ops/sec
    console.log(
      `Pre-generating ${imagesPerInterval * totalIntervals} images...`
    );
    const allImages = await imageGenerator.generateBatchForTest(
      'consistency-interval-batch',
      imagesPerInterval * totalIntervals,
      [0.5, 1.5]
    );
    let globalImageIndex = 0;

    for (let interval = 1; interval <= totalIntervals; interval++) {
      console.log(`Starting interval ${interval}/${totalIntervals}`);

      const metricsCollector = new MetricsCollector(
        `consistency-interval-${interval}`
      );
      const startTime = Date.now();
      let operationCount = 0;

      while (Date.now() - startTime < intervalSec * 1000) {
        // Prepare batch of operations
        const batchOperations = [];
        for (
          let i = 0;
          i < batchSize && globalImageIndex < allImages.length;
          i++
        ) {
          const image = allImages[globalImageIndex++];
          batchOperations.push({
            image,
            imageName: `consistency-${interval}-${operationCount + i}.jpg`,
          });
        }

        if (batchOperations.length === 0) {
          // Reset if we run out (shouldn't happen with proper pre-generation)
          globalImageIndex = 0;
          continue;
        }

        try {
          // Execute batch in browser
          const results = await page.evaluate(
            async ({ operations }) => {
              const vfsService = (window as any).vfsService;
              const metrics = [];
              for (const op of operations) {
                const opStart = performance.now();
                try {
                  await vfsService.writeFile(op.imageName, op.image.data);
                  metrics.push({
                    success: true,
                    duration: performance.now() - opStart,
                    bytes: op.image.size,
                  });
                } catch (error) {
                  metrics.push({
                    success: false,
                    duration: performance.now() - opStart,
                  });
                }
              }
              return metrics;
            },
            { operations: batchOperations }
          );

          results.forEach(result => {
            if (result.success) {
              metricsCollector.recordBytes(result.bytes);
              operationCount++;
            } else {
              metricsCollector.recordError('consistency-test');
            }
          });
        } catch (error) {
          metricsCollector.recordError('consistency-test-batch');
        }
      }

      const intervalDuration = (Date.now() - startTime) / 1000;
      const throughput = operationCount / intervalDuration;
      const metrics = await metricsCollector.getMetrics();

      results.push({
        interval: interval,
        throughput: throughput,
        latency: metrics.latency.mean,
      });

      console.log(
        `Interval ${interval}: ${throughput.toFixed(1)} ops/sec, ${metrics.latency.mean.toFixed(1)}ms avg latency`
      );
    }

    // Analyze consistency
    const throughputs = results.map(r => r.throughput);
    const avgThroughput =
      throughputs.reduce((a, b) => a + b, 0) / throughputs.length;
    const maxThroughput = Math.max(...throughputs);
    const minThroughput = Math.min(...throughputs);
    const throughputVariation =
      ((maxThroughput - minThroughput) / avgThroughput) * 100;

    console.log(`Consistency analysis:`);
    console.log(`- Average throughput: ${avgThroughput.toFixed(1)} ops/sec`);
    console.log(`- Min throughput: ${minThroughput.toFixed(1)} ops/sec`);
    console.log(`- Max throughput: ${maxThroughput.toFixed(1)} ops/sec`);
    console.log(`- Variation: ${throughputVariation.toFixed(1)}%`);

    // Consistency assertions
    expect(avgThroughput).toBeGreaterThan(80); // Average should exceed 80 ops/sec
    expect(throughputVariation).toBeLessThan(30); // Less than 30% variation
    expect(minThroughput).toBeGreaterThan(avgThroughput * 0.7); // Min shouldn't drop below 70% of average
  });

  test('should optimize sequential performance with different file sizes', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const fileSizeConfigs = [
      {
        name: 'tiny',
        range: [0.1, 0.3] as [number, number],
        target: 200,
        batchSize: 25,
      },
      {
        name: 'small',
        range: [0.5, 1.0] as [number, number],
        target: 150,
        batchSize: 20,
      },
      {
        name: 'medium',
        range: [1.0, 3.0] as [number, number],
        target: 100,
        batchSize: 15,
      },
      {
        name: 'large',
        range: [3.0, 8.0] as [number, number],
        target: 50,
        batchSize: 10,
      },
    ];

    const results: Array<{
      size: string;
      throughput: number;
      dataThroughput: number;
      avgLatency: number;
    }> = [];

    for (const config of fileSizeConfigs) {
      console.log(
        `Testing ${config.name} files (${config.range[0]}-${config.range[1]}MB)`
      );

      const metricsCollector = new MetricsCollector(`size-test-${config.name}`);
      const testDuration = 30; // 30 seconds per size
      const startTime = Date.now();
      let operationCount = 0;
      let totalBytes = 0;

      // Pre-generate enough test images for this size category
      const estimatedOps = config.target * testDuration * 1.5; // 50% buffer
      console.log(`Pre-generating ${estimatedOps} ${config.name} images...`);
      const images = await imageGenerator.generateBatchForTest(
        `size-test-${config.name}`,
        estimatedOps,
        config.range
      );
      let imageIndex = 0;

      while (Date.now() - startTime < testDuration * 1000) {
        // Prepare batch
        const batchOperations = [];
        for (
          let i = 0;
          i < config.batchSize && imageIndex < images.length;
          i++
        ) {
          const image = images[imageIndex++];
          batchOperations.push({
            image,
            imageName: `size-${config.name}-${operationCount + i}.jpg`,
          });
        }

        if (batchOperations.length === 0) {
          // Reset and reuse if we run out
          imageIndex = 0;
          continue;
        }

        try {
          // Execute batch in browser
          const results = await page.evaluate(
            async ({ operations }) => {
              const vfsService = (window as any).vfsService;
              const metrics = [];
              for (const op of operations) {
                const opStart = performance.now();
                try {
                  await vfsService.writeFile(op.imageName, op.image.data);
                  metrics.push({
                    success: true,
                    duration: performance.now() - opStart,
                    bytes: op.image.size,
                  });
                } catch (error) {
                  metrics.push({
                    success: false,
                    duration: performance.now() - opStart,
                  });
                }
              }
              return metrics;
            },
            { operations: batchOperations }
          );

          results.forEach(result => {
            if (result.success) {
              metricsCollector.recordBytes(result.bytes);
              totalBytes += result.bytes;
              operationCount++;
            } else {
              metricsCollector.recordError(`size-test-${config.name}`);
            }
          });
        } catch (error) {
          metricsCollector.recordError(`size-test-${config.name}-batch`);
        }
      }

      const actualDuration = (Date.now() - startTime) / 1000;
      const throughput = operationCount / actualDuration;
      const dataThroughput = totalBytes / actualDuration / (1024 * 1024); // MB/sec
      const metrics = await metricsCollector.getMetrics();

      results.push({
        size: config.name,
        throughput: throughput,
        dataThroughput: dataThroughput,
        avgLatency: metrics.latency.mean,
      });

      console.log(
        `${config.name}: ${throughput.toFixed(1)} ops/sec, ${dataThroughput.toFixed(1)} MB/sec, ${metrics.latency.mean.toFixed(1)}ms avg`
      );

      // Individual size category assertions
      expect(throughput).toBeGreaterThan(config.target * 0.8); // At least 80% of target
      expect(metrics.errors.count).toBeLessThan(operationCount * 0.02); // Less than 2% error rate
    }

    // Cross-size analysis
    console.log('File size performance summary:', results);

    // Smaller files should generally have higher operation throughput
    const tinyThroughput =
      results.find(r => r.size === 'tiny')?.throughput || 0;
    const largeThroughput =
      results.find(r => r.size === 'large')?.throughput || 0;
    expect(tinyThroughput).toBeGreaterThan(largeThroughput);

    // All size categories should maintain reasonable performance
    results.forEach(result => {
      expect(result.throughput).toBeGreaterThan(20); // Minimum baseline
      expect(result.dataThroughput).toBeGreaterThan(1); // At least 1 MB/sec
    });
  });

  test('should handle sequential read/write patterns efficiently', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const patterns = [
      { name: 'write-only', writeRatio: 1.0, batchSize: 20 },
      { name: 'read-heavy', writeRatio: 0.2, batchSize: 25 },
      { name: 'balanced', writeRatio: 0.5, batchSize: 20 },
      { name: 'write-heavy', writeRatio: 0.8, batchSize: 18 },
    ];

    const results: Array<{
      pattern: string;
      throughput: number;
      writeOps: number;
      readOps: number;
    }> = [];

    for (const pattern of patterns) {
      console.log(
        `Testing ${pattern.name} pattern (${(pattern.writeRatio * 100).toFixed(0)}% writes)`
      );

      const metricsCollector = new MetricsCollector(`pattern-${pattern.name}`);
      const testDuration = 45; // 45 seconds per pattern
      const startTime = Date.now();
      let totalOperations = 0;
      let writeOps = 0;
      let readOps = 0;

      // Pre-populate some files for reading - using batched approach
      const initialFiles = await imageGenerator.generateBatchForTest(
        `pattern-${pattern.name}-initial`,
        100,
        [0.5, 1.5]
      );

      console.log(`Pre-populating ${initialFiles.length} initial files...`);
      // Batch the initial file writes
      const initBatchSize = 10;
      for (let i = 0; i < initialFiles.length; i += initBatchSize) {
        const batch = initialFiles
          .slice(i, i + initBatchSize)
          .map((image, idx) => ({
            image,
            imageName: `initial-${i + idx}.jpg`,
          }));

        await page.evaluate(
          async ({ operations }) => {
            const vfsService = (window as any).vfsService;
            for (const op of operations) {
              await vfsService.writeFile(op.imageName, op.image.data);
            }
          },
          { operations: batch }
        );
      }

      // Pre-generate all images for writing during test
      const estimatedWrites = Math.ceil(
        100 * testDuration * pattern.writeRatio * 1.2
      );
      console.log(`Pre-generating ${estimatedWrites} write images...`);
      const writeImages = await imageGenerator.generateBatchForTest(
        `pattern-${pattern.name}-writes`,
        estimatedWrites,
        [0.5, 1.5]
      );
      let writeImageIndex = 0;

      while (Date.now() - startTime < testDuration * 1000) {
        // Build a batch of mixed read/write operations
        const batchOps = [];
        for (let i = 0; i < pattern.batchSize; i++) {
          const shouldWrite = Math.random() < pattern.writeRatio;

          if (shouldWrite) {
            if (writeImageIndex >= writeImages.length) {
              writeImageIndex = 0; // Cycle through images
            }
            const image = writeImages[writeImageIndex++];
            batchOps.push({
              type: 'write',
              image,
              imageName: `pattern-${pattern.name}-${writeOps + batchOps.filter(op => op.type === 'write').length}.jpg`,
            });
          } else {
            // Read operation
            const fileIndex = Math.floor(
              Math.random() * (initialFiles.length + writeOps)
            );
            const fileName =
              fileIndex < initialFiles.length
                ? `initial-${fileIndex}.jpg`
                : `pattern-${pattern.name}-${fileIndex - initialFiles.length}.jpg`;

            batchOps.push({
              type: 'read',
              fileName,
            });
          }
        }

        try {
          // Execute batch of mixed operations in browser
          const results = await page.evaluate(
            async ({ operations }) => {
              const vfsService = (window as any).vfsService;
              const metrics = [];

              for (const op of operations) {
                const opStart = performance.now();
                try {
                  if (op.type === 'write') {
                    await vfsService.writeFile(op.imageName, op.image.data);
                    metrics.push({
                      type: 'write',
                      success: true,
                      duration: performance.now() - opStart,
                      bytes: op.image.size,
                    });
                  } else {
                    const content = await vfsService.readFile(op.fileName);
                    metrics.push({
                      type: 'read',
                      success: true,
                      duration: performance.now() - opStart,
                      bytes: content ? content.length : 0,
                    });
                  }
                } catch (error) {
                  metrics.push({
                    type: op.type,
                    success: false,
                    duration: performance.now() - opStart,
                    error: String(error),
                  });
                }
              }
              return metrics;
            },
            { operations: batchOps }
          );

          // Process results
          results.forEach(result => {
            if (result.success) {
              metricsCollector.recordBytes(result.bytes);
              if (result.type === 'write') {
                writeOps++;
              } else {
                readOps++;
              }
              totalOperations++;
            } else {
              metricsCollector.recordError(
                `pattern-${pattern.name}-${result.type}`
              );
            }
          });
        } catch (error) {
          metricsCollector.recordError(`pattern-${pattern.name}-batch`);
          console.error('Batch operation failed:', error);
        }
      }

      const actualDuration = (Date.now() - startTime) / 1000;
      const throughput = totalOperations / actualDuration;

      results.push({
        pattern: pattern.name,
        throughput: throughput,
        writeOps: writeOps,
        readOps: readOps,
      });

      console.log(
        `${pattern.name}: ${throughput.toFixed(1)} ops/sec (${writeOps} writes, ${readOps} reads)`
      );
    }

    // Pattern analysis
    console.log('Read/write pattern results:', results);

    // All patterns should achieve reasonable throughput
    results.forEach(result => {
      expect(result.throughput).toBeGreaterThan(50); // Minimum 50 ops/sec
      expect(result.writeOps + result.readOps).toBeGreaterThan(1000); // Sufficient operations
    });

    // Read-heavy patterns might have higher throughput due to caching
    const readHeavy = results.find(r => r.pattern === 'read-heavy');
    const writeOnly = results.find(r => r.pattern === 'write-only');

    if (readHeavy && writeOnly) {
      // Read operations should generally be faster
      expect(readHeavy.throughput).toBeGreaterThanOrEqual(
        writeOnly.throughput * 0.8
      );
    }
  });
});
