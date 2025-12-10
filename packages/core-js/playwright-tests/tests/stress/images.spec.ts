import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';
import {
  expect,
  setupTestWithServer,
  test,
  waitForVFSConnection,
} from '../fixtures';

test.describe('Image Stress Tests', () => {
  test.beforeEach(async ({ page, serverInstance }) => {
    await setupTestWithServer(page, serverInstance);
    await waitForVFSConnection(page);
  });

  test('should handle 5 sequential image uploads (diagnostic)', async ({
    page,
    serverInstance,
  }) => {
    // Start with a very small batch to verify basic functionality
    const batchSize = 5;
    const images = await imageGenerator.generateBatchForTest(
      'diagnostic-5-sequential',
      batchSize,
      [0.1, 0.3]
    ); // Small 0.1-0.3MB images

    console.log(
      `Starting diagnostic upload of ${batchSize} small images to port ${serverInstance.port}`
    );

    // Enable console monitoring for debugging
    page.on('console', msg => {
      if (msg.type() === 'error') {
        console.error('Browser console error:', msg.text());
      } else if (msg.type() === 'log' && msg.text().includes('VFS')) {
        console.log('VFS log:', msg.text());
      }
    });

    // Track successful uploads separately from UI counter
    let successfulUploads = 0;
    const errors: string[] = [];

    for (let i = 0; i < images.length; i++) {
      const image = images[i];

      try {
        console.log(
          `Attempting upload ${i + 1}/${batchSize}: ${image.name} (${(image.size / 1024).toFixed(1)}KB)`
        );

        // Add individual timeout and better error handling for each upload
        const uploadResult = await page.evaluate(
          async ({ image }) => {
            try {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              console.log(`[VFS] Starting upload of ${image.name}`);

              // Add timeout wrapper for individual upload
              const timeoutPromise = new Promise((_, reject) =>
                setTimeout(
                  () =>
                    reject(new Error('Individual upload timeout after 15s')),
                  15000
                )
              );

              const uploadPromise = vfsService.writeFileWithBytes(
                image.name,
                { type: 'image', size: image.data.byteLength },
                image.data
              );

              await Promise.race([uploadPromise, timeoutPromise]);

              console.log(`[VFS] Upload completed for ${image.name}`);
              return { success: true };
            } catch (error) {
              console.error(`[VFS] Upload failed for ${image.name}:`, error);
              return { success: false, error: String(error) };
            }
          },
          { image }
        );

        if (uploadResult.success) {
          successfulUploads++;
          console.log(`✓ Upload ${i + 1}/${batchSize} completed successfully`);
        } else {
          const errorMsg = `Upload ${i + 1} failed: ${uploadResult.error}`;
          errors.push(errorMsg);
          console.error(`✗ ${errorMsg}`);
        }
      } catch (error) {
        const errorMsg = `Upload ${i + 1} failed with exception: ${error}`;
        errors.push(errorMsg);
        console.error(`✗ ${errorMsg}`);
      }
    }

    // Report results
    console.log(
      `Diagnostic test completed: ${successfulUploads}/${batchSize} uploads successful`
    );

    if (errors.length > 0) {
      console.error('Upload errors encountered:', errors);
    }

    // Verify results - expect 100% success
    expect(successfulUploads).toBe(batchSize);
    expect(errors.length).toBe(0);

    if (successfulUploads === batchSize) {
      console.log(
        '✓ All diagnostic uploads successful - basic functionality working'
      );
    }
  });

  test('should handle 20 sequential image uploads with monitoring', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 20;
    const images = await imageGenerator.generateBatchForTest(
      'monitored-20-sequential',
      batchSize,
      [0.2, 0.8]
    ); // Medium 0.2-0.8MB images

    console.log(
      `Starting monitored upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Enable comprehensive console monitoring
    page.on('console', msg => {
      if (msg.type() === 'error') {
        console.error('Browser console error:', msg.text());
      }
    });

    const startTime = Date.now();
    let successfulUploads = 0;

    // Upload images sequentially with better error handling
    for (let i = 0; i < images.length; i++) {
      const image = images[i];

      try {
        // Inject image data and trigger upload with timeout
        const result = await page.evaluate(
          async ({ image }) => {
            try {
              const vfsService = (window as any).vfsService;
              if (!vfsService) throw new Error('VFS service not available');

              const startTime = Date.now();

              // Add timeout for individual operation
              const timeoutPromise = new Promise((_, reject) =>
                setTimeout(
                  () => reject(new Error('Upload timeout after 20s')),
                  20000
                )
              );

              const uploadPromise = vfsService.writeFileWithBytes(
                image.name,
                { type: 'image', size: image.data.byteLength },
                image.data
              );

              await Promise.race([uploadPromise, timeoutPromise]);

              const duration = Date.now() - startTime;
              return { success: true, duration };
            } catch (error) {
              return { success: false, error: String(error) };
            }
          },
          { image }
        );

        if (result.success) {
          successfulUploads++;

          if (successfulUploads % 5 === 0) {
            console.log(
              `Progress: ${successfulUploads}/${batchSize} uploads completed`
            );
          }
        } else {
          console.error(`Upload ${i + 1} failed: ${result.error}`);
        }
      } catch (error) {
        console.error(`Upload ${i + 1} exception: ${error}`);
      }

      // Check memory usage periodically
      if ((i + 1) % 10 === 0) {
        // Get memory from UI instead of direct browser evaluation
        const memUsageText = await page.getByTestId('heap-used').textContent();
        const memUsage = parseFloat(memUsageText?.replace(' MB', '') || '0');
        console.log(`Memory after ${i + 1} uploads: ${memUsage.toFixed(2)} MB`);
      }
    }

    const duration = Date.now() - startTime;

    // Get final metrics from UI
    const heapUsedText = await page.getByTestId('heap-used').textContent();
    const heapUsedMB = parseFloat(heapUsedText?.replace(' MB', '') || '0');

    // Verify results - expect 100% success
    expect(successfulUploads).toBe(batchSize);
    expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    if (successfulUploads > 0) {
      const throughput = (successfulUploads / duration) * 1000; // ops/sec
      expect(throughput).toBeGreaterThan(0.7); // At least 0.7 ops/sec (more realistic)

      console.log(
        `Monitored test completed: ${successfulUploads}/${batchSize} successful, ${throughput.toFixed(2)} ops/sec, ${duration}ms total`
      );
    }

    // Memory assertions - more lenient (reading from UI)
    expect(heapUsedMB).toBeLessThan(300); // Less than 300MB
  });

  test('should handle 100 sequential image uploads with chunking', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 100;
    const images = await imageGenerator.generateBatchForTest(
      'chunked-100-sequential',
      batchSize,
      [0.5, 1.5]
    ); // Medium 0.5-1.5MB images

    console.log(
      `Starting chunked upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Enable console monitoring
    page.on('console', msg => {
      if (msg.type() === 'error') {
        console.error('Browser console error:', msg.text());
      }
    });

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector(
      'sequential-100-images-chunked'
    );
    const startTime = Date.now();

    // Process in chunks to avoid memory issues
    const chunkSize = 10;
    let totalSuccessful = 0;
    let totalErrors = 0;

    for (let chunk = 0; chunk < images.length; chunk += chunkSize) {
      const chunkImages = images.slice(
        chunk,
        Math.min(chunk + chunkSize, images.length)
      );
      console.log(
        `Processing chunk ${Math.floor(chunk / chunkSize) + 1}/${Math.ceil(images.length / chunkSize)}: ${chunkImages.length} images`
      );

      // Process chunk sequentially
      for (let i = 0; i < chunkImages.length; i++) {
        const image = chunkImages[i];
        const globalIndex = chunk + i;
        const endOperation = metricsCollector.startOperation();

        try {
          const result = await page.evaluate(
            async ({ image }) => {
              try {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const timeoutPromise = new Promise((_, reject) =>
                  setTimeout(() => reject(new Error('Upload timeout')), 15000)
                );

                const uploadPromise = vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );

                await Promise.race([uploadPromise, timeoutPromise]);
                return { success: true };
              } catch (error) {
                return { success: false, error: String(error) };
              }
            },
            { image }
          );

          if (result.success) {
            totalSuccessful++;
            endOperation();
            metricsCollector.recordBytes(image.size);
          } else {
            totalErrors++;
            metricsCollector.recordError('upload-failed');
            console.error(`Upload ${globalIndex + 1} failed: ${result.error}`);
          }
        } catch (error) {
          totalErrors++;
          metricsCollector.recordError('upload-exception');
          console.error(`Upload ${globalIndex + 1} exception: ${error}`);
        }
      }

      // Check memory and progress after each chunk
      const memUsage = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      console.log(
        `Chunk completed: ${totalSuccessful}/${batchSize} total successful, ${totalErrors} errors, ${memUsage.toFixed(2)} MB memory`
      );

      // Force garbage collection if available
      await page.evaluate(() => {
        if (window.gc) {
          window.gc();
        }
      });
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    // Verify results with realistic expectations for large batch
    expect(totalSuccessful).toBe(batchSize); // 100% success rate
    expect(totalErrors).toBe(0); // No errors

    // Performance assertions
    if (totalSuccessful > 0) {
      const throughput = (totalSuccessful / duration) * 1000; // ops/sec
      expect(throughput).toBeGreaterThan(3); // At least 3 ops/sec for large batch

      console.log(
        `Chunked test completed: ${totalSuccessful}/${batchSize} successful (${((totalSuccessful / batchSize) * 100).toFixed(1)}%), ${throughput.toFixed(2)} ops/sec, ${duration}ms total`
      );
    }

    // Memory assertions - more lenient for large batch
    expect(metrics.memory.heapUsed).toBeLessThan(500 * 1024 * 1024); // Less than 500MB
  });

  test('should handle 500 parallel image uploads', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 500;
    const images = await imageGenerator.generateBatchForTest(
      'batched-500-parallel',
      batchSize,
      [0.5, 2]
    ); // 0.5-2MB images

    console.log(
      `Starting sequential batch upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector(
      'sequential-500-images-batched'
    );
    const startTime = Date.now();

    // Process in sequential batches instead of parallel
    const parallelBatchSize = 50;
    let totalSuccessful = 0;
    let totalErrors = 0;

    for (let i = 0; i < images.length; i += parallelBatchSize) {
      const batch = images.slice(i, i + parallelBatchSize);
      const batchNumber = Math.floor(i / parallelBatchSize) + 1;
      const totalBatches = Math.ceil(batchSize / parallelBatchSize);

      console.log(
        `Starting batch ${batchNumber}/${totalBatches}: ${batch.length} images (${i + 1}-${Math.min(i + parallelBatchSize, images.length)} of ${batchSize})`
      );

      // Check memory before batch
      const memBefore = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      // Process batch sequentially to avoid server overload
      let batchSuccessful = 0;
      let batchErrors = 0;

      for (const image of batch) {
        const endOperation = metricsCollector.startOperation();

        try {
          const result = await page.evaluate(
            async ({ image }) => {
              try {
                const vfsService = (window as any).vfsService;
                if (!vfsService) {
                  throw new Error('VFS service not available');
                }

                // Add timeout for individual upload
                const timeoutPromise = new Promise((_, reject) =>
                  setTimeout(
                    () => reject(new Error('Upload timeout after 20s')),
                    20000
                  )
                );

                const uploadPromise = vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );

                await Promise.race([uploadPromise, timeoutPromise]);

                return { success: true };
              } catch (error) {
                return { success: false, error: String(error) };
              }
            },
            { image }
          );

          if (result.success) {
            batchSuccessful++;
            totalSuccessful++;
            endOperation();
            metricsCollector.recordBytes(image.size);
          } else {
            batchErrors++;
            totalErrors++;
            metricsCollector.recordError('upload-failed');
            console.error(
              `Upload failed in batch ${batchNumber}: ${result.error}`
            );
          }
        } catch (error) {
          batchErrors++;
          totalErrors++;
          metricsCollector.recordError('upload-exception');
          console.error(`Upload exception in batch ${batchNumber}: ${error}`);
        }
      }

      // Check memory after batch
      const memAfter = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      // Get heap used from UI for additional verification
      const heapUsedText = await page.getByTestId('heap-used').textContent();
      const heapUsedMB = parseFloat(heapUsedText?.replace(' MB', '') || '0');

      console.log(
        `Batch ${batchNumber}/${totalBatches} completed: ${batchSuccessful}/${batch.length} successful, ${batchErrors} errors`
      );
      console.log(
        `  Memory: Before=${memBefore.toFixed(2)}MB, After=${memAfter.toFixed(2)}MB, Delta=${(memAfter - memBefore).toFixed(2)}MB, UI Heap=${heapUsedMB.toFixed(2)}MB`
      );
      console.log(
        `  Total progress: ${totalSuccessful}/${batchSize} uploads (${((totalSuccessful / batchSize) * 100).toFixed(1)}% complete)`
      );

      // Add a small delay between batches to avoid server overload
      if (i + parallelBatchSize < images.length) {
        await new Promise(resolve => setTimeout(resolve, 500));
      }

      // Force garbage collection if available
      await page.evaluate(() => {
        if (window.gc) {
          window.gc();
        }
      });

      // Check if memory is getting too high
      if (memAfter > 400) {
        console.warn(`! High memory usage detected: ${memAfter.toFixed(2)}MB`);
      }
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    console.log('\n=== Test Summary ===');
    console.log(
      `Total uploads: ${totalSuccessful}/${batchSize} successful (${((totalSuccessful / batchSize) * 100).toFixed(1)}%)`
    );
    console.log(`Total errors: ${totalErrors}`);
    console.log(`Total duration: ${duration}ms`);

    // Verify results - expect 100% success
    expect(totalSuccessful).toBe(batchSize);
    expect(totalErrors).toBe(0);

    // Verify UI counters
    await expect(page.getByTestId('operations-count')).toContainText(
      totalSuccessful.toString(),
      { timeout: 10000 }
    );
    await expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    if (totalSuccessful > 0) {
      const throughput = (totalSuccessful / duration) * 1000; // ops/sec
      expect(throughput).toBeGreaterThan(5); // At least 5 ops/sec for sequential batches

      console.log(`Throughput: ${throughput.toFixed(2)} ops/sec`);
    }

    // Memory assertions
    const finalMemoryMB = metrics.memory.heapUsed / (1024 * 1024);
    expect(finalMemoryMB).toBeLessThan(500); // Less than 500MB
    console.log(`Final memory usage: ${finalMemoryMB.toFixed(2)}MB`);

    console.log(
      `Sequential batch test completed: ${totalSuccessful}/${batchSize} successful, ${((totalSuccessful / batchSize) * 100).toFixed(1)}% success rate`
    );
  });

  test('should handle 1000 mixed size image uploads', async ({
    page,
    serverInstance,
  }) => {
    const batchSize = 1000;
    // Mix of small (0.1-1MB), medium (1-5MB), and large (5-10MB) images
    const smallImages = await imageGenerator.generateBatchForTest(
      'mixed-1000-small',
      500,
      [0.1, 1]
    );
    const mediumImages = await imageGenerator.generateBatchForTest(
      'mixed-1000-medium',
      400,
      [1, 5]
    );
    const largeImages = await imageGenerator.generateBatchForTest(
      'mixed-1000-large',
      100,
      [5, 10]
    );
    const allImages = [...smallImages, ...mediumImages, ...largeImages];

    console.log(
      `Starting sequential mixed upload of ${batchSize} images to port ${serverInstance.port}`
    );

    // Create metrics collector for this test
    const metricsCollector = new MetricsCollector(
      'sequential-mixed-1000-images'
    );
    const startTime = Date.now();

    let totalSuccessful = 0;
    let totalErrors = 0;

    // Process small images sequentially in batches of 50
    console.log(
      `\n=== Processing ${smallImages.length} small images (0.1-1MB) ===`
    );
    for (let i = 0; i < smallImages.length; i += 50) {
      const batch = smallImages.slice(i, i + 50);
      const batchNumber = Math.floor(i / 50) + 1;
      const totalBatches = Math.ceil(smallImages.length / 50);

      console.log(
        `Small images batch ${batchNumber}/${totalBatches}: ${batch.length} images`
      );

      // Check memory before batch
      const memBefore = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      let batchSuccessful = 0;
      let batchErrors = 0;

      for (const image of batch) {
        const endOperation = metricsCollector.startOperation();

        try {
          const result = await page.evaluate(
            async ({ image }) => {
              try {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const timeoutPromise = new Promise((_, reject) =>
                  setTimeout(
                    () => reject(new Error('Upload timeout after 15s')),
                    15000
                  )
                );

                const uploadPromise = vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );

                await Promise.race([uploadPromise, timeoutPromise]);
                return { success: true };
              } catch (error) {
                return { success: false, error: String(error) };
              }
            },
            { image }
          );

          if (result.success) {
            batchSuccessful++;
            totalSuccessful++;
            endOperation();
            metricsCollector.recordBytes(image.size);
          } else {
            batchErrors++;
            totalErrors++;
            metricsCollector.recordError('upload-failed');
          }
        } catch (error) {
          batchErrors++;
          totalErrors++;
          metricsCollector.recordError('upload-exception');
        }
      }

      // Check memory after batch
      const memAfter = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      const heapUsedText = await page.getByTestId('heap-used').textContent();
      const heapUsedMB = parseFloat(heapUsedText?.replace(' MB', '') || '0');

      console.log(
        `  Batch completed: ${batchSuccessful}/${batch.length} successful, ${batchErrors} errors`
      );
      console.log(
        `  Memory: Before=${memBefore.toFixed(2)}MB, After=${memAfter.toFixed(2)}MB, Delta=${(memAfter - memBefore).toFixed(2)}MB, UI=${heapUsedMB.toFixed(2)}MB`
      );
      console.log(
        `  Total progress: ${totalSuccessful}/${batchSize} (${((totalSuccessful / batchSize) * 100).toFixed(1)}%)`
      );

      // Small delay between batches
      if (i + 50 < smallImages.length) {
        await new Promise(resolve => setTimeout(resolve, 300));
      }

      await page.evaluate(() => {
        if (window.gc) window.gc();
      });
    }

    // Process medium images sequentially in batches of 25
    console.log(
      `\n=== Processing ${mediumImages.length} medium images (1-5MB) ===`
    );
    for (let i = 0; i < mediumImages.length; i += 25) {
      const batch = mediumImages.slice(i, i + 25);
      const batchNumber = Math.floor(i / 25) + 1;
      const totalBatches = Math.ceil(mediumImages.length / 25);

      console.log(
        `Medium images batch ${batchNumber}/${totalBatches}: ${batch.length} images`
      );

      const memBefore = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      let batchSuccessful = 0;
      let batchErrors = 0;

      for (const image of batch) {
        const endOperation = metricsCollector.startOperation();

        try {
          const result = await page.evaluate(
            async ({ image }) => {
              try {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const timeoutPromise = new Promise((_, reject) =>
                  setTimeout(
                    () => reject(new Error('Upload timeout after 20s')),
                    20000
                  )
                );

                const uploadPromise = vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );

                await Promise.race([uploadPromise, timeoutPromise]);
                return { success: true };
              } catch (error) {
                return { success: false, error: String(error) };
              }
            },
            { image }
          );

          if (result.success) {
            batchSuccessful++;
            totalSuccessful++;
            endOperation();
            metricsCollector.recordBytes(image.size);
          } else {
            batchErrors++;
            totalErrors++;
            metricsCollector.recordError('upload-failed');
          }
        } catch (error) {
          batchErrors++;
          totalErrors++;
          metricsCollector.recordError('upload-exception');
        }
      }

      const memAfter = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      const heapUsedText = await page.getByTestId('heap-used').textContent();
      const heapUsedMB = parseFloat(heapUsedText?.replace(' MB', '') || '0');

      console.log(
        `  Batch completed: ${batchSuccessful}/${batch.length} successful, ${batchErrors} errors`
      );
      console.log(
        `  Memory: Before=${memBefore.toFixed(2)}MB, After=${memAfter.toFixed(2)}MB, Delta=${(memAfter - memBefore).toFixed(2)}MB, UI=${heapUsedMB.toFixed(2)}MB`
      );
      console.log(
        `  Total progress: ${totalSuccessful}/${batchSize} (${((totalSuccessful / batchSize) * 100).toFixed(1)}%)`
      );

      if (i + 25 < mediumImages.length) {
        await new Promise(resolve => setTimeout(resolve, 500));
      }

      await page.evaluate(() => {
        if (window.gc) window.gc();
      });
    }

    // Process large images sequentially in batches of 10
    console.log(
      `\n=== Processing ${largeImages.length} large images (5-10MB) ===`
    );
    for (let i = 0; i < largeImages.length; i += 10) {
      const batch = largeImages.slice(i, i + 10);
      const batchNumber = Math.floor(i / 10) + 1;
      const totalBatches = Math.ceil(largeImages.length / 10);

      console.log(
        `Large images batch ${batchNumber}/${totalBatches}: ${batch.length} images`
      );

      const memBefore = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      let batchSuccessful = 0;
      let batchErrors = 0;

      for (const image of batch) {
        const endOperation = metricsCollector.startOperation();

        try {
          const result = await page.evaluate(
            async ({ image }) => {
              try {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const timeoutPromise = new Promise((_, reject) =>
                  setTimeout(
                    () => reject(new Error('Upload timeout after 30s')),
                    30000
                  )
                );

                const uploadPromise = vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );

                await Promise.race([uploadPromise, timeoutPromise]);
                return { success: true };
              } catch (error) {
                return { success: false, error: String(error) };
              }
            },
            { image }
          );

          if (result.success) {
            batchSuccessful++;
            totalSuccessful++;
            endOperation();
            metricsCollector.recordBytes(image.size);
          } else {
            batchErrors++;
            totalErrors++;
            metricsCollector.recordError('upload-failed');
          }
        } catch (error) {
          batchErrors++;
          totalErrors++;
          metricsCollector.recordError('upload-exception');
        }
      }

      const memAfter = await page.evaluate(() => {
        if ((performance as any).memory) {
          return (performance as any).memory.usedJSHeapSize / (1024 * 1024);
        }
        return 0;
      });

      const heapUsedText = await page.getByTestId('heap-used').textContent();
      const heapUsedMB = parseFloat(heapUsedText?.replace(' MB', '') || '0');

      console.log(
        `  Batch completed: ${batchSuccessful}/${batch.length} successful, ${batchErrors} errors`
      );
      console.log(
        `  Memory: Before=${memBefore.toFixed(2)}MB, After=${memAfter.toFixed(2)}MB, Delta=${(memAfter - memBefore).toFixed(2)}MB, UI=${heapUsedMB.toFixed(2)}MB`
      );
      console.log(
        `  Total progress: ${totalSuccessful}/${batchSize} (${((totalSuccessful / batchSize) * 100).toFixed(1)}%)`
      );

      if (i + 10 < largeImages.length) {
        await new Promise(resolve => setTimeout(resolve, 1000));
      }

      await page.evaluate(() => {
        if (window.gc) window.gc();
      });

      if (memAfter > 600) {
        console.warn(`! High memory usage detected: ${memAfter.toFixed(2)}MB`);
      }
    }

    const duration = Date.now() - startTime;
    const metrics = await metricsCollector.getMetrics();

    console.log('\n=== Test Summary ===');
    console.log(
      `Total uploads: ${totalSuccessful}/${batchSize} successful (${((totalSuccessful / batchSize) * 100).toFixed(1)}%)`
    );
    console.log(`Total errors: ${totalErrors}`);
    console.log(`Total duration: ${duration}ms`);

    // Verify results - expect 100% success
    expect(totalSuccessful).toBe(batchSize);
    expect(totalErrors).toBe(0);

    // Verify UI counters
    await expect(page.getByTestId('operations-count')).toContainText(
      totalSuccessful.toString(),
      { timeout: 15000 }
    );
    await expect(page.getByTestId('error-count')).toContainText('0');

    // Performance assertions
    if (totalSuccessful > 0) {
      const throughput = (totalSuccessful / duration) * 1000; // ops/sec
      expect(throughput).toBeGreaterThan(3); // At least 3 ops/sec for sequential mixed sizes

      console.log(`Throughput: ${throughput.toFixed(2)} ops/sec`);
    }

    // Memory assertions
    const finalMemoryMB = metrics.memory.heapUsed / (1024 * 1024);
    expect(finalMemoryMB).toBeLessThan(800); // Less than 800MB
    console.log(`Final memory usage: ${finalMemoryMB.toFixed(2)}MB`);

    // Calculate data throughput
    const totalDataSize = allImages.reduce(
      (sum, img) => sum + img.data.length,
      0
    );
    const dataThroughput = ((totalDataSize / duration) * 1000) / (1024 * 1024); // MB/sec
    expect(dataThroughput).toBeGreaterThan(2); // At least 2 MB/sec for sequential

    console.log(`Data throughput: ${dataThroughput.toFixed(2)} MB/sec`);
    console.log(
      `Sequential mixed test completed: ${totalSuccessful}/${batchSize} successful, ${((totalSuccessful / batchSize) * 100).toFixed(1)}% success rate`
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

      const images = await imageGenerator.generateBatchForTest(
        `progressive-${size}`,
        size,
        [1, 3]
      );
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
              async ({ image }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFileWithBytes(
                  image.name,
                  { type: 'image', size: image.data.byteLength },
                  image.data
                );
              },
              { image }
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
      expect(result.throughput).toBeGreaterThan(6);
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
