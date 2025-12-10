import type { Browser, BrowserContext, Page } from '@playwright/test';
import {
  setupTestWithServer,
  waitForVFSConnection,
} from '../../tests/fixtures';
import type { ServerInstance } from '../test-ui/types';

export interface ConnectionInfo {
  id: string;
  context: BrowserContext;
  page: Page;
  connectedAt: number;
  connectionTime: number; // Time to establish connection (ms)
  latency: number[]; // Operation latencies only
  operationCount: number;
  errorCount: number;
  isHealthy: boolean;
}

export interface ConnectionStats {
  totalConnections: number;
  healthyConnections: number;
  avgLatency: number;
  p95Latency: number;
  p99Latency: number;
  totalOperations: number;
  totalErrors: number;
  avgOperationsPerConnection: number;
}

/**
 * Manages multiple WebSocket connections for benchmark testing
 */
export class ConnectionManager {
  private connections: Map<string, ConnectionInfo> = new Map();
  private browser: Browser;
  private serverInstance: ServerInstance;
  private errorLog: Array<{
    timestamp: number;
    connectionId: string;
    type: string;
    message: string;
    stack?: string;
  }> = [];

  constructor(browser: Browser, serverInstance: ServerInstance) {
    this.browser = browser;
    this.serverInstance = serverInstance;
  }

  /**
   * Create and connect N new connections
   */
  async createConnections(
    count: number,
    prefix: string = 'conn',
    startIndex: number = 0
  ): Promise<string[]> {
    const connectionIds: string[] = [];
    const startTime = Date.now();

    console.log(
      `Creating ${count} connections with prefix "${prefix}" starting at index ${startIndex}...`
    );

    // Create connections in parallel batches to avoid overwhelming the browser
    const batchSize = 10;
    for (let i = 0; i < count; i += batchSize) {
      const batchCount = Math.min(batchSize, count - i);
      const batchPromises = [];

      for (let j = 0; j < batchCount; j++) {
        const index = startIndex + i + j;
        const connId = `${prefix}-${index}`;
        batchPromises.push(this.createSingleConnection(connId));
        connectionIds.push(connId);
      }

      await Promise.all(batchPromises);

      // Small delay between batches to avoid overwhelming server
      if (i + batchSize < count) {
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    }

    const duration = Date.now() - startTime;
    console.log(
      `Created ${count} connections in ${duration}ms (${(count / (duration / 1000)).toFixed(1)} conn/sec)`
    );

    return connectionIds;
  }

  /**
   * Create a single connection
   */
  private async createSingleConnection(id: string): Promise<void> {
    const startTime = Date.now();

    try {
      // Create browser context
      const context = await this.browser.newContext();

      // Create page
      const page = await context.newPage();

      // Setup test with server
      await setupTestWithServer(page, this.serverInstance);

      // Wait for VFS connection
      await waitForVFSConnection(page);

      const connectionTime = Date.now() - startTime;

      // Store connection info
      const connInfo: ConnectionInfo = {
        id,
        context,
        page,
        connectedAt: Date.now(),
        connectionTime, // Store separately from operation latencies
        latency: [], // Only operation latencies, not connection time
        operationCount: 0,
        errorCount: 0,
        isHealthy: true,
      };

      this.connections.set(id, connInfo);

      // Setup error monitoring
      page.on('pageerror', error => {
        const errorDetail = {
          timestamp: Date.now(),
          connectionId: id,
          type: 'page-error',
          message: error.message,
          stack: error.stack,
        };
        this.errorLog.push(errorDetail);
        console.error(`❌ [${id}] Page error:`, {
          message: error.message,
          stack: error.stack,
          time: new Date().toISOString(),
        });
        connInfo.errorCount++;
        connInfo.isHealthy = false;
      });

      page.on('console', msg => {
        if (msg.type() === 'error') {
          const errorDetail = {
            timestamp: Date.now(),
            connectionId: id,
            type: 'console-error',
            message: msg.text(),
          };
          this.errorLog.push(errorDetail);
          console.error(`❌ [${id}] Console error:`, {
            message: msg.text(),
            time: new Date().toISOString(),
          });
          connInfo.errorCount++;
        }
      });
    } catch (error) {
      console.error(`Failed to create connection ${id}:`, error);
      throw error;
    }
  }

  /**
   * Execute an operation on a specific connection
   */
  async executeOperation(
    connectionId: string,
    operation: (page: Page) => Promise<void>
  ): Promise<{ success: boolean; latency: number }> {
    const conn = this.connections.get(connectionId);
    if (!conn) {
      throw new Error(`Connection ${connectionId} not found`);
    }

    const startTime = Date.now();
    let success = false;

    try {
      await operation(conn.page);
      success = true;
      conn.operationCount++;
    } catch (error) {
      const errorDetail = {
        timestamp: Date.now(),
        connectionId,
        type: 'operation-failed',
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
      };
      this.errorLog.push(errorDetail);
      console.error(`❌ [${connectionId}] Operation failed:`, {
        message: errorDetail.message,
        stack: errorDetail.stack,
        time: new Date().toISOString(),
      });
      conn.errorCount++;
      conn.isHealthy = false;
    }

    const latency = Date.now() - startTime;
    conn.latency.push(latency);

    return { success, latency };
  }

  /**
   * Execute operation on all connections in parallel
   */
  async executeOnAll(
    operation: (page: Page, connectionId: string) => Promise<void>,
    options?: { maxConcurrency?: number }
  ): Promise<{
    successCount: number;
    failureCount: number;
    avgLatency: number;
  }> {
    const maxConcurrency = options?.maxConcurrency || this.connections.size;
    const connectionIds = Array.from(this.connections.keys());

    let successCount = 0;
    let failureCount = 0;
    let totalLatency = 0;

    // Process in batches based on max concurrency
    for (let i = 0; i < connectionIds.length; i += maxConcurrency) {
      const batch = connectionIds.slice(i, i + maxConcurrency);

      const results = await Promise.all(
        batch.map(id => this.executeOperation(id, page => operation(page, id)))
      );

      results.forEach(result => {
        if (result.success) {
          successCount++;
        } else {
          failureCount++;
        }
        totalLatency += result.latency;
      });
    }

    const avgLatency = totalLatency / connectionIds.length;

    return { successCount, failureCount, avgLatency };
  }

  /**
   * Execute operation on multiple random connections
   */
  async executeOnRandom(
    count: number,
    operation: (page: Page, connectionId: string) => Promise<void>
  ): Promise<{
    successCount: number;
    failureCount: number;
    avgLatency: number;
  }> {
    const allIds = Array.from(this.connections.keys());
    const selectedIds = [];

    // Randomly select connections
    for (let i = 0; i < count && allIds.length > 0; i++) {
      const randomIndex = Math.floor(Math.random() * allIds.length);
      selectedIds.push(allIds[randomIndex]);
    }

    let successCount = 0;
    let failureCount = 0;
    let totalLatency = 0;

    const results = await Promise.all(
      selectedIds.map(id =>
        this.executeOperation(id, page => operation(page, id))
      )
    );

    results.forEach(result => {
      if (result.success) {
        successCount++;
      } else {
        failureCount++;
      }
      totalLatency += result.latency;
    });

    return {
      successCount,
      failureCount,
      avgLatency: totalLatency / selectedIds.length,
    };
  }

  /**
   * Check health of all connections
   */
  async checkHealth(): Promise<{ healthy: number; unhealthy: number }> {
    let healthy = 0;
    let unhealthy = 0;

    for (const [, conn] of this.connections) {
      try {
        // Try to evaluate a simple script to check if connection is alive
        await conn.page.evaluate(() => {
          return (window as any).vfsService !== undefined;
        });

        if (conn.isHealthy) {
          healthy++;
        } else {
          unhealthy++;
        }
      } catch (error) {
        conn.isHealthy = false;
        unhealthy++;
      }
    }

    return { healthy, unhealthy };
  }

  /**
   * Get statistics for all connections
   */
  getStats(): ConnectionStats {
    return this.getStatsForConnections(Array.from(this.connections.keys()));
  }

  /**
   * Get statistics for a specific subset of connections
   */
  getStatsForConnections(connectionIds: string[]): ConnectionStats {
    let totalOperations = 0;
    let totalErrors = 0;
    let healthyConnections = 0;
    const allLatencies: number[] = [];

    for (const id of connectionIds) {
      const conn = this.connections.get(id);
      if (!conn) continue;

      totalOperations += conn.operationCount;
      totalErrors += conn.errorCount;
      if (conn.isHealthy) healthyConnections++;
      allLatencies.push(...conn.latency);
    }

    // Calculate latency percentiles
    const sortedLatencies = allLatencies.sort((a, b) => a - b);
    const p95Index = Math.floor(sortedLatencies.length * 0.95);
    const p99Index = Math.floor(sortedLatencies.length * 0.99);

    return {
      totalConnections: connectionIds.length,
      healthyConnections,
      avgLatency:
        allLatencies.length > 0
          ? allLatencies.reduce((a, b) => a + b, 0) / allLatencies.length
          : 0,
      p95Latency: sortedLatencies[p95Index] || 0,
      p99Latency: sortedLatencies[p99Index] || 0,
      totalOperations,
      totalErrors,
      avgOperationsPerConnection:
        connectionIds.length > 0 ? totalOperations / connectionIds.length : 0,
    };
  }

  /**
   * Get connection info by ID
   */
  getConnection(id: string): ConnectionInfo | undefined {
    return this.connections.get(id);
  }

  /**
   * Get all connection IDs
   */
  getConnectionIds(): string[] {
    return Array.from(this.connections.keys());
  }

  /**
   * Get connection count
   */
  getConnectionCount(): number {
    return this.connections.size;
  }

  /**
   * Close a specific connection
   */
  async closeConnection(id: string): Promise<void> {
    const conn = this.connections.get(id);
    if (!conn) {
      console.warn(`Connection ${id} not found`);
      return;
    }

    try {
      await conn.context.close();
      this.connections.delete(id);
    } catch (error) {
      console.error(`Error closing connection ${id}:`, error);
    }
  }

  /**
   * Close multiple connections
   */
  async closeConnections(ids: string[]): Promise<void> {
    await Promise.all(ids.map(id => this.closeConnection(id)));
  }

  /**
   * Close all connections
   */
  async closeAll(): Promise<void> {
    console.log(`Closing ${this.connections.size} connections...`);

    const closePromises: Promise<void>[] = [];
    for (const [id, conn] of this.connections) {
      closePromises.push(
        conn.context.close().catch(error => {
          console.error(`Error closing connection ${id}:`, error);
        })
      );
    }

    await Promise.all(closePromises);
    this.connections.clear();

    console.log('All connections closed');
  }

  /**
   * Generate a report of all connections
   */
  generateReport(): string {
    const stats = this.getStats();

    const report = `
=== Connection Manager Report ===
Total Connections: ${stats.totalConnections}
Healthy Connections: ${stats.healthyConnections}
Unhealthy Connections: ${stats.totalConnections - stats.healthyConnections}

LATENCY:
- Average: ${stats.avgLatency.toFixed(2)}ms
- P95: ${stats.p95Latency.toFixed(2)}ms
- P99: ${stats.p99Latency.toFixed(2)}ms

OPERATIONS:
- Total Operations: ${stats.totalOperations}
- Total Errors: ${stats.totalErrors}
- Avg Operations/Connection: ${stats.avgOperationsPerConnection.toFixed(1)}
- Error Rate: ${stats.totalOperations > 0 ? ((stats.totalErrors / stats.totalOperations) * 100).toFixed(2) : 0}%
`;

    return report;
  }

  getErrorLog() {
    return [...this.errorLog];
  }

  getRecentErrors(limit: number = 10) {
    return this.errorLog.slice(-limit);
  }

  async getCounterValue(connectionId: string): Promise<number | null> {
    const conn = this.connections.get(connectionId);
    if (!conn) {
      return null;
    }

    try {
      const counterText = await conn.page
        .getByTestId('counter-value')
        .textContent();
      if (!counterText) return null;

      const match = counterText.match(/Counter:\s*(\d+)/);
      return match ? parseInt(match[1], 10) : null;
    } catch (error) {
      console.warn(`Failed to read counter from ${connectionId}:`, error);
      return null;
    }
  }

  async verifySync(sourceOfTruthId?: string): Promise<{
    inSync: boolean;
    sourceOfTruthValue: number | null;
    sourceOfTruthId: string | null;
    totalConnections: number;
    valueDistribution: Map<number, number>;
    inSyncCount: number;
    outOfSyncCount: number;
    minValue: number | null;
    maxValue: number | null;
    medianValue: number | null;
    modeValue: number | null;
    sampleOutliers: Array<{
      connectionId: string;
      value: number;
      drift: number;
    }>;
  }> {
    const connectionIds = Array.from(this.connections.keys());

    if (connectionIds.length === 0) {
      return {
        inSync: true,
        sourceOfTruthValue: null,
        sourceOfTruthId: null,
        totalConnections: 0,
        valueDistribution: new Map(),
        inSyncCount: 0,
        outOfSyncCount: 0,
        minValue: null,
        maxValue: null,
        medianValue: null,
        modeValue: null,
        sampleOutliers: [],
      };
    }

    const actualSourceId = sourceOfTruthId || connectionIds[0];
    const sourceValue = await this.getCounterValue(actualSourceId);

    const allValues: Array<{ id: string; value: number | null }> = [];
    for (const connId of connectionIds) {
      const value = await this.getCounterValue(connId);
      allValues.push({ id: connId, value });
    }

    const validValues = allValues
      .filter((v): v is { id: string; value: number } => v.value !== null)
      .map(v => v.value);

    if (validValues.length === 0 || sourceValue === null) {
      return {
        inSync: true,
        sourceOfTruthValue: sourceValue,
        sourceOfTruthId: actualSourceId,
        totalConnections: connectionIds.length,
        valueDistribution: new Map(),
        inSyncCount: 0,
        outOfSyncCount: 0,
        minValue: null,
        maxValue: null,
        medianValue: null,
        modeValue: null,
        sampleOutliers: [],
      };
    }

    const valueDistribution = new Map<number, number>();
    for (const value of validValues) {
      valueDistribution.set(value, (valueDistribution.get(value) || 0) + 1);
    }

    const sortedValues = [...validValues].sort((a, b) => a - b);
    const minValue = sortedValues[0];
    const maxValue = sortedValues[sortedValues.length - 1];
    const medianValue = sortedValues[Math.floor(sortedValues.length / 2)];

    let modeValue = validValues[0];
    let modeCount = 0;
    for (const [value, count] of valueDistribution.entries()) {
      if (count > modeCount) {
        modeCount = count;
        modeValue = value;
      }
    }

    const inSyncCount = allValues.filter(v => v.value === sourceValue).length;
    const outOfSyncCount = connectionIds.length - inSyncCount;

    const outliers = allValues
      .filter(v => v.value !== null && v.value !== sourceValue)
      .map(v => ({
        connectionId: v.id,
        value: v.value!,
        drift: v.value! - sourceValue,
      }))
      .sort((a, b) => Math.abs(b.drift) - Math.abs(a.drift));

    const sampleOutliers = outliers.slice(0, 5);

    return {
      inSync: outOfSyncCount === 0,
      sourceOfTruthValue: sourceValue,
      sourceOfTruthId: actualSourceId,
      totalConnections: connectionIds.length,
      valueDistribution,
      inSyncCount,
      outOfSyncCount,
      minValue,
      maxValue,
      medianValue,
      modeValue,
      sampleOutliers,
    };
  }
}
