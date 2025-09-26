import { test, expect } from '../fixtures';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';
import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';

test.describe('Sequential Operations Throughput Tests', () => {
  test('should achieve target sequential throughput (>100 ops/sec)', async ({
    page,
    serverInstance,
  }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);

    const targetOpsPerSec = 100;
    const testDurationSec = 60; // 1 minute test

    console.log(
      `Starting sequential throughput test: target ${targetOpsPerSec} ops/sec for ${testDurationSec} seconds`
    );

    const metricsCollector = new MetricsCollector('sequential-throughput');
    const startTime = Date.now();
    let operationCount = 0;

    // Generate test data upfront
    const batchSize = 50;
    let currentBatch = await imageGenerator.generateBatch(
      batchSize,
      [0.5, 1.5]
    );
    let batchIndex = 0;

    while (Date.now() - startTime < testDurationSec * 1000) {
      // Refresh batch if needed
      if (batchIndex >= currentBatch.length) {
        currentBatch = await imageGenerator.generateBatch(
          batchSize,
          [0.5, 1.5]
        );
        batchIndex = 0;
      }

      const image = currentBatch[batchIndex++];
      const endOperation = metricsCollector.startOperation();

      try {
        // Sequential write operation
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
            imageName: `seq-${operationCount}.jpg`,
          }
        );

        endOperation();
        metricsCollector.recordBytes(image.size);
        operationCount++;

        // Log progress every 100 operations
        if (operationCount % 100 === 0) {
          const elapsed = (Date.now() - startTime) / 1000;
          const currentThroughput = operationCount / elapsed;
          console.log(
            `${operationCount} ops in ${elapsed.toFixed(1)}s = ${currentThroughput.toFixed(1)} ops/sec`
          );
        }
      } catch (error) {
        endOperation();
        metricsCollector.recordError('sequential-write');
        console.error('Sequential operation failed:', error);
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
    const results: Array<{
      interval: number;
      throughput: number;
      latency: number;
    }> = [];

    console.log(
      `Starting consistency test: ${totalIntervals} intervals of ${intervalSec}s each`
    );

    for (let interval = 1; interval <= totalIntervals; interval++) {
      console.log(`Starting interval ${interval}/${totalIntervals}`);

      const metricsCollector = new MetricsCollector(
        `consistency-interval-${interval}`
      );
      const startTime = Date.now();
      let operationCount = 0;

      // Generate batch for this interval
      const images = await imageGenerator.generateBatch(100, [0.5, 1.5]);
      let imageIndex = 0;

      while (Date.now() - startTime < intervalSec * 1000) {
        if (imageIndex >= images.length) {
          // Regenerate if we run out
          const newImages = await imageGenerator.generateBatch(100, [0.5, 1.5]);
          images.splice(0, images.length, ...newImages);
          imageIndex = 0;
        }

        const image = images[imageIndex++];
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
              imageName: `consistency-${interval}-${operationCount}.jpg`,
            }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
          operationCount++;
        } catch (error) {
          endOperation();
          metricsCollector.recordError('consistency-test');
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
      { name: 'tiny', range: [0.1, 0.3] as [number, number], target: 200 },
      { name: 'small', range: [0.5, 1.0] as [number, number], target: 150 },
      { name: 'medium', range: [1.0, 3.0] as [number, number], target: 100 },
      { name: 'large', range: [3.0, 8.0] as [number, number], target: 50 },
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

      // Generate test images for this size category
      const images = await imageGenerator.generateBatch(50, config.range);
      let imageIndex = 0;

      while (Date.now() - startTime < testDuration * 1000) {
        if (imageIndex >= images.length) {
          imageIndex = 0; // Cycle through images
        }

        const image = images[imageIndex++];
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
              imageName: `size-${config.name}-${operationCount}.jpg`,
            }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
          totalBytes += image.size;
          operationCount++;
        } catch (error) {
          endOperation();
          metricsCollector.recordError(`size-test-${config.name}`);
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
      { name: 'write-only', writeRatio: 1.0 },
      { name: 'read-heavy', writeRatio: 0.2 },
      { name: 'balanced', writeRatio: 0.5 },
      { name: 'write-heavy', writeRatio: 0.8 },
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

      // Pre-populate some files for reading
      const initialFiles = await imageGenerator.generateBatch(20, [0.5, 1.5]);
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

      // Generate additional images for writing during test
      const writeImages = await imageGenerator.generateBatch(100, [0.5, 1.5]);
      let writeImageIndex = 0;

      while (Date.now() - startTime < testDuration * 1000) {
        const shouldWrite = Math.random() < pattern.writeRatio;
        const endOperation = metricsCollector.startOperation();

        try {
          if (shouldWrite) {
            // Write operation
            if (writeImageIndex >= writeImages.length) {
              writeImageIndex = 0; // Cycle through images
            }

            const image = writeImages[writeImageIndex++];
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
                imageName: `pattern-${pattern.name}-${writeOps}.jpg`,
              }
            );

            metricsCollector.recordBytes(image.size);
            writeOps++;
          } else {
            // Read operation
            const fileIndex = Math.floor(
              Math.random() * (initialFiles.length + writeOps)
            );
            const fileName =
              fileIndex < initialFiles.length
                ? `initial-${fileIndex}.jpg`
                : `pattern-${pattern.name}-${fileIndex - initialFiles.length}.jpg`;

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
            readOps++;
          }

          endOperation();
          totalOperations++;
        } catch (error) {
          endOperation();
          metricsCollector.recordError(`pattern-${pattern.name}`);
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
