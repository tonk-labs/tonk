import { test, expect } from '../fixtures';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';
import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';

test.describe('Image Stress Tests', () => {
  test.beforeEach(async ({ page, serverInstance }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);
  });

  test('should handle 100 sequential image uploads', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 100;
    const images = await imageGenerator.generateBatch(batchSize, [1, 3]); // 1-3MB images

    console.log(
      `Starting sequential upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector('sequential-100-images');
    const startTime = Date.now();

    // Upload images sequentially
    for (let i = 0; i < images.length; i++) {
      const image = images[i];
      const endOperation = metricsCollector.startOperation();

      // Inject image data and trigger upload
      await page.evaluate(
        async ({ imageData, imageName }) => {
          const vfsService = (window as any).vfsService;
          if (!vfsService) throw new Error('VFS service not available');

          await vfsService.writeFile(imageName, new Uint8Array(imageData));
        },
        { imageData: Array.from(image.data), imageName: image.name }
      );

      endOperation();
      metricsCollector.recordBytes(image.size);

      // Check progress every 10 uploads
      if ((i + 1) % 10 === 0) {
        const operationsCount = await page
          .getByTestId('operations-count')
          .textContent();
        console.log(
          `Progress: ${i + 1}/${batchSize} uploads, operations: ${operationsCount}`
        );
      }
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    // Verify all operations completed
    await expect(page.getByTestId('operations-count')).toContainText(
      batchSize.toString(),
      { timeout: 30000 }
    );

    // Verify no errors
    await expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    const throughput = (batchSize / duration) * 1000; // ops/sec
    expect(throughput).toBeGreaterThan(10); // At least 10 ops/sec

    // Memory assertions
    expect(metrics.memory.heapUsed).toBeLessThan(200 * 1024 * 1024); // Less than 200MB

    console.log(
      `Sequential test completed: ${throughput.toFixed(2)} ops/sec, ${duration}ms total`
    );
  });

  test('should handle 500 parallel image uploads', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 500;
    const images = await imageGenerator.generateBatch(batchSize, [0.5, 2]); // 0.5-2MB images

    console.log(
      `Starting parallel upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector('parallel-500-images');
    const startTime = Date.now();

    // Upload images in parallel batches of 50
    const parallelBatchSize = 50;
    for (let i = 0; i < images.length; i += parallelBatchSize) {
      const batch = images.slice(i, i + parallelBatchSize);

      // Upload batch in parallel
      await Promise.all(
        batch.map(async image => {
          const endOperation = metricsCollector.startOperation();

          await page.evaluate(
            async ({ imageData, imageName }) => {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              await vfsService.writeFile(imageName, new Uint8Array(imageData));
            },
            { imageData: Array.from(image.data), imageName: image.name }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
        })
      );

      console.log(
        `Completed batch ${Math.floor(i / parallelBatchSize) + 1}/${Math.ceil(batchSize / parallelBatchSize)}`
      );
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    // Verify all operations completed
    await expect(page.getByTestId('operations-count')).toContainText(
      batchSize.toString(),
      { timeout: 60000 }
    );

    // Verify no errors
    await expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    const throughput = (batchSize / duration) * 1000; // ops/sec
    expect(throughput).toBeGreaterThan(50); // At least 50 ops/sec for parallel

    // Memory assertions
    expect(metrics.memory.heapUsed).toBeLessThan(500 * 1024 * 1024); // Less than 500MB

    console.log(
      `Parallel test completed: ${throughput.toFixed(2)} ops/sec, ${duration}ms total`
    );
  });

  test('should handle 1000 mixed size image uploads', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 1000;
    // Mix of small (0.1-1MB), medium (1-5MB), and large (5-10MB) images
    const smallImages = await imageGenerator.generateBatch(500, [0.1, 1]);
    const mediumImages = await imageGenerator.generateBatch(400, [1, 5]);
    const largeImages = await imageGenerator.generateBatch(100, [5, 10]);
    const allImages = [...smallImages, ...mediumImages, ...largeImages];

    console.log(
      `Starting mixed upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector('mixed-1000-images');
    const startTime = Date.now();

    // Upload in adaptive batches based on size
    let uploadedCount = 0;

    // Upload small images in large batches
    for (let i = 0; i < smallImages.length; i += 100) {
      const batch = smallImages.slice(i, i + 100);
      await Promise.all(
        batch.map(async image => {
          const endOperation = metricsCollector.startOperation();

          await page.evaluate(
            async ({ imageData, imageName }) => {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              await vfsService.writeFile(imageName, new Uint8Array(imageData));
            },
            { imageData: Array.from(image.data), imageName: image.name }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
        })
      );
      uploadedCount += batch.length;
      console.log(`Small images progress: ${uploadedCount}/${batchSize}`);
    }

    // Upload medium images in medium batches
    for (let i = 0; i < mediumImages.length; i += 50) {
      const batch = mediumImages.slice(i, i + 50);
      await Promise.all(
        batch.map(async image => {
          const endOperation = metricsCollector.startOperation();

          await page.evaluate(
            async ({ imageData, imageName }) => {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              await vfsService.writeFile(imageName, new Uint8Array(imageData));
            },
            { imageData: Array.from(image.data), imageName: image.name }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
        })
      );
      uploadedCount += batch.length;
      console.log(`Medium images progress: ${uploadedCount}/${batchSize}`);
    }

    // Upload large images in small batches
    for (let i = 0; i < largeImages.length; i += 10) {
      const batch = largeImages.slice(i, i + 10);
      await Promise.all(
        batch.map(async image => {
          const endOperation = metricsCollector.startOperation();

          await page.evaluate(
            async ({ imageData, imageName }) => {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              await vfsService.writeFile(imageName, new Uint8Array(imageData));
            },
            { imageData: Array.from(image.data), imageName: image.name }
          );

          endOperation();
          metricsCollector.recordBytes(image.size);
        })
      );
      uploadedCount += batch.length;
      console.log(`Large images progress: ${uploadedCount}/${batchSize}`);
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    // Verify all operations completed
    await expect(page.getByTestId('operations-count')).toContainText(
      batchSize.toString(),
      { timeout: 120000 }
    );

    // Verify no errors
    await expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    const throughput = (batchSize / duration) * 1000; // ops/sec
    expect(throughput).toBeGreaterThan(20); // At least 20 ops/sec for mixed sizes

    // Memory assertions
    expect(metrics.memory.heapUsed).toBeLessThan(800 * 1024 * 1024); // Less than 800MB

    // Calculate data throughput
    const totalDataSize = allImages.reduce(
      (sum, img) => sum + img.data.length,
      0
    );
    const dataThroughput = ((totalDataSize / duration) * 1000) / (1024 * 1024); // MB/sec
    expect(dataThroughput).toBeGreaterThan(5); // At least 5 MB/sec

    console.log(
      `Mixed size test completed: ${throughput.toFixed(2)} ops/sec, ${dataThroughput.toFixed(2)} MB/sec, ${duration}ms total`
    );
  });

  test('should maintain performance with progressive load', async ({
    page,
    serverInstance,
  }) => {
    const testSizes = [50, 100, 200, 500];
    const results: Array<{
      size: number;
      throughput: number;
      memoryMB: number;
    }> = [];

    console.log(
      `Starting progressive load test on port ${serverInstance.port}`
    );

    for (const size of testSizes) {
      console.log(`Testing with ${size} images...`);

      const images = await imageGenerator.generateBatch(size, [1, 3]);
      const metricsCollector = new MetricsCollector(
        `progressive-${size}-images`
      );
      const startTime = Date.now();

      // Upload in optimal batches
      const batchSize = Math.min(25, size);
      for (let i = 0; i < images.length; i += batchSize) {
        const batch = images.slice(i, i + batchSize);
        await Promise.all(
          batch.map(async image => {
            const endOperation = metricsCollector.startOperation();

            await page.evaluate(
              async ({ imageData, imageName }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFile(
                  imageName,
                  new Uint8Array(imageData)
                );
              },
              { imageData: Array.from(image.data), imageName: image.name }
            );

            endOperation();
            metricsCollector.recordBytes(image.size);
          })
        );
      }

      const duration = Date.now() - startTime;
      const metrics = await metricsCollector.getMetrics();
      const throughput = (size / duration) * 1000;
      const memoryMB = metrics.memory.heapUsed / (1024 * 1024);

      results.push({ size, throughput, memoryMB });

      // Verify operations completed
      await expect(page.getByTestId('operations-count')).toContainText(
        size.toString(),
        { timeout: 30000 }
      );
      await expect(page.getByTestId('error-count')).toContainText('0');

      console.log(
        `${size} images: ${throughput.toFixed(2)} ops/sec, ${memoryMB.toFixed(1)} MB memory`
      );

      // Reset for next test
      await page.reload();
      await waitForVFSConnection(page);
    }

    // Analyze performance trends
    console.log('Progressive load results:', results);

    // Throughput should remain reasonable
    results.forEach(result => {
      expect(result.throughput).toBeGreaterThan(10);
    });

    // Memory should scale reasonably (not exponentially)
    const memoryGrowthRatio =
      results[results.length - 1].memoryMB / results[0].memoryMB;
    const sizeGrowthRatio = results[results.length - 1].size / results[0].size;
    expect(memoryGrowthRatio).toBeLessThan(sizeGrowthRatio * 2); // Memory shouldn't grow more than 2x the size ratio

    console.log(
      `Progressive load completed. Memory growth ratio: ${memoryGrowthRatio.toFixed(2)}x vs size ratio: ${sizeGrowthRatio}x`
    );
  });
});
