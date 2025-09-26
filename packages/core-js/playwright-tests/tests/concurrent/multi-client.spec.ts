import { test, expect } from '@playwright/test';
import { setupTestWithServer, waitForVFSConnection } from '../fixtures';
import { imageGenerator } from '../../src/utils/image-generator';
import { MetricsCollector } from '../../src/utils/metrics-collector';

test.describe('Concurrent Operations Tests', () => {
  test('should handle 10 concurrent clients performing operations', async ({
    browser,
  }) => {
    const clientCount = 10;
    const operationsPerClient = 20;

    console.log(
      `Starting concurrent test with ${clientCount} clients, ${operationsPerClient} operations each`
    );

    // Create a server instance for this test
    const { serverManager } = await import('../../src/server/server-manager');
    const server = await serverManager.startServer('concurrent-test');

    try {
      const metricsCollector = new MetricsCollector('concurrent-10-clients');
      const clients: Array<{ context: any; page: any }> = [];

      // Create multiple browser contexts (simulating different clients)
      for (let i = 0; i < clientCount; i++) {
        const context = await browser.newContext();
        const page = await context.newPage();
        await setupTestWithServer(page, server);
        await waitForVFSConnection(page);
        clients.push({ context, page });
        console.log(`Client ${i + 1} connected`);
      }

      // Generate test images for all clients
      const images = await imageGenerator.generateBatch(
        clientCount * operationsPerClient,
        [0.5, 2]
      );

      const startTime = Date.now();

      // Run operations concurrently across all clients
      const clientPromises = clients.map(async (client, clientIndex) => {
        const clientImages = images.slice(
          clientIndex * operationsPerClient,
          (clientIndex + 1) * operationsPerClient
        );

        // Perform operations for this client
        for (let opIndex = 0; opIndex < clientImages.length; opIndex++) {
          const image = clientImages[opIndex];
          const endOperation = metricsCollector.startOperation();

          try {
            // Write operation
            await client.page.evaluate(
              async (params: { imageData: number[]; imageName: string }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFile(
                  params.imageName,
                  new Uint8Array(params.imageData)
                );
              },
              { imageData: Array.from(image.data), imageName: image.name }
            );

            // Read operation to verify
            await client.page.evaluate(
              async (params: { imageName: string }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                const readData = await vfsService.readFile(params.imageName);
                if (!readData)
                  throw new Error(`Failed to read ${params.imageName}`);
              },
              { imageName: image.name }
            );

            endOperation();
            metricsCollector.recordBytes(image.size * 2); // Count both write and read
          } catch (error) {
            endOperation();
            metricsCollector.recordError(
              `client-${clientIndex}-operation-${opIndex}`
            );
            console.error(
              `Client ${clientIndex} operation ${opIndex} failed:`,
              error
            );
          }
        }

        console.log(`Client ${clientIndex + 1} completed all operations`);
      });

      // Wait for all clients to complete
      await Promise.all(clientPromises);

      const duration = Date.now() - startTime;
      const metrics = await metricsCollector.getMetrics();

      // Verify operations completed across all clients
      let totalOperations = 0;
      for (const client of clients) {
        const operationsCount = await client.page
          .getByTestId('operations-count')
          .textContent();
        totalOperations += parseInt(operationsCount || '0');
      }

      expect(totalOperations).toBeGreaterThanOrEqual(
        clientCount * operationsPerClient
      );

      // Performance assertions
      const totalExpectedOps = clientCount * operationsPerClient * 2; // read + write
      const throughput = (totalExpectedOps / duration) * 1000;
      expect(throughput).toBeGreaterThan(50); // At least 50 ops/sec across all clients

      // Error rate should be low
      expect(metrics.errors.count).toBeLessThan(totalExpectedOps * 0.05); // Less than 5% error rate

      console.log(
        `Concurrent test completed: ${throughput.toFixed(2)} ops/sec, ${metrics.errors.count} errors`
      );

      // Cleanup clients
      for (const client of clients) {
        await client.context.close();
      }
    } finally {
      await serverManager.stopServer('concurrent-test');
    }
  });

  test('should handle race conditions in file operations', async ({
    browser,
  }) => {
    const clientCount = 5;
    const fileName = 'shared-file.txt';
    const iterations = 50;

    console.log(
      `Starting race condition test with ${clientCount} clients on shared file`
    );

    // Create a server instance for this test
    const { serverManager } = await import('../../src/server/server-manager');
    const server = await serverManager.startServer('race-condition-test');

    try {
      const metricsCollector = new MetricsCollector('race-condition-test');
      const clients: Array<{ context: any; page: any }> = [];

      // Create multiple browser contexts
      for (let i = 0; i < clientCount; i++) {
        const context = await browser.newContext();
        const page = await context.newPage();
        await setupTestWithServer(page, server);
        await waitForVFSConnection(page);
        clients.push({ context, page });
      }

      const startTime = Date.now();

      // Each client performs rapid read/write operations on the same file
      const clientPromises = clients.map(async (client, clientIndex) => {
        for (let i = 0; i < iterations; i++) {
          const endOperation = metricsCollector.startOperation();
          const content = `Client ${clientIndex} - Iteration ${i} - ${Date.now()}`;

          try {
            // Write to shared file
            await client.page.evaluate(
              async (params: { fileName: string; content: string }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                await vfsService.writeFile(params.fileName, params.content);
              },
              { fileName, content }
            );

            // Immediately read back
            const readContent = await client.page.evaluate(
              async (params: { fileName: string }) => {
                const vfsService = (window as any).vfsService;
                if (!vfsService) throw new Error('VFS service not available');

                return await vfsService.readFile(params.fileName);
              },
              { fileName }
            );

            // Verify we can read something (may not be our exact content due to races)
            if (!readContent) {
              throw new Error('Failed to read shared file');
            }

            endOperation();
            metricsCollector.recordBytes(
              content.length + (readContent as string).length
            );
          } catch (error) {
            endOperation();
            metricsCollector.recordError(
              `race-condition-client-${clientIndex}`
            );
          }
        }

        console.log(`Client ${clientIndex + 1} completed race condition test`);
      });

      // Run all clients concurrently
      await Promise.all(clientPromises);

      const duration = Date.now() - startTime;
      const metrics = await metricsCollector.getMetrics();

      // Verify system remained stable despite race conditions
      const totalOperations = clientCount * iterations * 2; // read + write
      const throughput = (totalOperations / duration) * 1000;

      // Should handle some level of concurrency even with race conditions
      expect(throughput).toBeGreaterThan(20);

      // System should not crash (some errors expected due to race conditions)
      expect(metrics.errors.count).toBeLessThan(totalOperations * 0.5); // Less than 50% error rate

      console.log(
        `Race condition test completed: ${throughput.toFixed(2)} ops/sec, ${metrics.errors.count} errors`
      );

      // Cleanup
      for (const client of clients) {
        await client.context.close();
      }
    } finally {
      await serverManager.stopServer('race-condition-test');
    }
  });

  test('should scale performance with client count', async ({ browser }) => {
    const clientCounts = [2, 5, 10, 20];
    const operationsPerClient = 10;
    const results: Array<{
      clients: number;
      throughput: number;
      errors: number;
    }> = [];

    console.log('Starting scalability test across different client counts');

    for (const clientCount of clientCounts) {
      console.log(`Testing with ${clientCount} clients...`);

      // Create a server instance for this test
      const { serverManager } = await import('../../src/server/server-manager');
      const testId = `scalability-${clientCount}-clients`;
      const server = await serverManager.startServer(testId);

      try {
        const metricsCollector = new MetricsCollector(testId);
        const clients: Array<{ context: any; page: any }> = [];

        // Create clients
        for (let i = 0; i < clientCount; i++) {
          const context = await browser.newContext();
          const page = await context.newPage();
          await setupTestWithServer(page, server);
          await waitForVFSConnection(page);
          clients.push({ context, page });
        }

        // Generate test data
        const images = await imageGenerator.generateBatch(
          clientCount * operationsPerClient,
          [0.5, 1]
        );

        const startTime = Date.now();

        // Run concurrent operations
        const clientPromises = clients.map(async (client, clientIndex) => {
          const clientImages = images.slice(
            clientIndex * operationsPerClient,
            (clientIndex + 1) * operationsPerClient
          );

          for (const image of clientImages) {
            const endOperation = metricsCollector.startOperation();

            try {
              await client.page.evaluate(
                async (params: { imageData: number[]; imageName: string }) => {
                  const vfsService = (window as any).vfsService;
                  if (!vfsService) throw new Error('VFS service not available');

                  await vfsService.writeFile(
                    params.imageName,
                    new Uint8Array(params.imageData)
                  );
                },
                { imageData: Array.from(image.data), imageName: image.name }
              );

              endOperation();
              metricsCollector.recordBytes(image.size);
            } catch (error) {
              endOperation();
              metricsCollector.recordError(`scalability-client-${clientIndex}`);
            }
          }
        });

        await Promise.all(clientPromises);

        const duration = Date.now() - startTime;
        const metrics = await metricsCollector.getMetrics();
        const throughput =
          ((clientCount * operationsPerClient) / duration) * 1000;

        results.push({
          clients: clientCount,
          throughput: throughput,
          errors: metrics.errors.count,
        });

        console.log(
          `${clientCount} clients: ${throughput.toFixed(2)} ops/sec, ${metrics.errors.count} errors`
        );

        // Cleanup
        for (const client of clients) {
          await client.context.close();
        }
      } finally {
        await serverManager.stopServer(testId);
      }
    }

    // Analyze scalability results
    console.log('Scalability results:', results);

    // Performance should generally increase with more clients (up to a point)
    results.forEach(result => {
      expect(result.throughput).toBeGreaterThan(5); // Minimum baseline
      expect(result.errors).toBeLessThan(
        result.clients * operationsPerClient * 0.1
      ); // Less than 10% error rate
    });

    // Check that we can handle the maximum client count without major degradation
    const maxResult = results[results.length - 1];
    expect(maxResult.throughput).toBeGreaterThan(20); // Should still maintain reasonable throughput

    console.log('Scalability test completed successfully');
  });

  test('should handle mixed operation types concurrently', async ({
    browser,
  }) => {
    const clientCount = 8;
    console.log(`Starting mixed operations test with ${clientCount} clients`);

    // Create a server instance for this test
    const { serverManager } = await import('../../src/server/server-manager');
    const server = await serverManager.startServer('mixed-operations-test');

    try {
      const metricsCollector = new MetricsCollector('mixed-operations-test');
      const clients: Array<{ context: any; page: any }> = [];

      // Create clients
      for (let i = 0; i < clientCount; i++) {
        const context = await browser.newContext();
        const page = await context.newPage();
        await setupTestWithServer(page, server);
        await waitForVFSConnection(page);
        clients.push({ context, page });
      }

      const startTime = Date.now();

      // Different operation types for different clients
      const clientPromises = clients.map(async (client, clientIndex) => {
        const operationType = clientIndex % 4;

        switch (operationType) {
          case 0: // Heavy write operations
            const images = await imageGenerator.generateBatch(15, [2, 5]);
            for (const image of images) {
              const endOperation = metricsCollector.startOperation();
              try {
                await client.page.evaluate(
                  async (params: {
                    imageData: number[];
                    imageName: string;
                  }) => {
                    const vfsService = (window as any).vfsService;
                    await vfsService.writeFile(
                      params.imageName,
                      new Uint8Array(params.imageData)
                    );
                  },
                  { imageData: Array.from(image.data), imageName: image.name }
                );
                endOperation();
                metricsCollector.recordBytes(image.size);
              } catch (error) {
                endOperation();
                metricsCollector.recordError('heavy-write');
              }
            }
            break;

          case 1: // Rapid small writes
            for (let i = 0; i < 50; i++) {
              const endOperation = metricsCollector.startOperation();
              try {
                await client.page.evaluate(
                  async (params: { fileName: string; content: string }) => {
                    const vfsService = (window as any).vfsService;
                    await vfsService.writeFile(params.fileName, params.content);
                  },
                  {
                    fileName: `small-${clientIndex}-${i}.txt`,
                    content: `Small content ${i}`,
                  }
                );
                endOperation();
                metricsCollector.recordBytes(20);
              } catch (error) {
                endOperation();
                metricsCollector.recordError('rapid-small-write');
              }
            }
            break;

          case 2: // Read operations on existing files
            // First create some files to read
            for (let i = 0; i < 20; i++) {
              const fileName = `read-target-${i}.txt`;
              await client.page.evaluate(
                async (params: { fileName: string }) => {
                  const vfsService = (window as any).vfsService;
                  await vfsService.writeFile(
                    params.fileName,
                    `Content for reading ${Math.random()}`
                  );
                },
                { fileName }
              );
            }

            // Then perform read operations
            for (let i = 0; i < 30; i++) {
              const endOperation = metricsCollector.startOperation();
              try {
                const content = await client.page.evaluate(
                  async (params: { fileName: string }) => {
                    const vfsService = (window as any).vfsService;
                    return await vfsService.readFile(params.fileName);
                  },
                  { fileName: `read-target-${i % 20}.txt` }
                );
                endOperation();
                metricsCollector.recordBytes((content as string)?.length || 0);
              } catch (error) {
                endOperation();
                metricsCollector.recordError('read-operation');
              }
            }
            break;

          case 3: // Mixed read/write pattern
            for (let i = 0; i < 25; i++) {
              const endOperation = metricsCollector.startOperation();
              try {
                const fileName = `mixed-${clientIndex}-${i}.txt`;
                const content = `Mixed content ${i} ${Date.now()}`;

                // Write
                await client.page.evaluate(
                  async (params: { fileName: string; content: string }) => {
                    const vfsService = (window as any).vfsService;
                    await vfsService.writeFile(params.fileName, params.content);
                  },
                  { fileName, content }
                );

                // Read back
                const readContent = await client.page.evaluate(
                  async (params: { fileName: string }) => {
                    const vfsService = (window as any).vfsService;
                    return await vfsService.readFile(params.fileName);
                  },
                  { fileName }
                );

                if (readContent !== content) {
                  throw new Error('Read/write mismatch');
                }

                endOperation();
                metricsCollector.recordBytes(content.length * 2);
              } catch (error) {
                endOperation();
                metricsCollector.recordError('mixed-read-write');
              }
            }
            break;
        }

        console.log(
          `Client ${clientIndex + 1} (type ${operationType}) completed operations`
        );
      });

      await Promise.all(clientPromises);

      const duration = Date.now() - startTime;
      const metrics = await metricsCollector.getMetrics();

      // Verify system handled mixed workload effectively
      const throughput = (metrics.throughput.totalOperations / duration) * 1000;
      expect(throughput).toBeGreaterThan(30); // Should handle mixed load efficiently

      // Error rate should remain reasonable
      expect(metrics.errors.count).toBeLessThan(
        metrics.throughput.totalOperations * 0.1
      );

      console.log(
        `Mixed operations test completed: ${throughput.toFixed(2)} ops/sec, ${metrics.errors.count} errors`
      );

      // Cleanup
      for (const client of clients) {
        await client.context.close();
      }
    } finally {
      await serverManager.stopServer('mixed-operations-test');
    }
  });
});
