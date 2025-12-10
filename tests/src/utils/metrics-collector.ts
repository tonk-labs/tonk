import type {
  BenchmarkMetrics,
  LatencyMetrics,
  MemoryMetrics,
} from '../test-ui/types';

/**
 * Collects and analyzes performance metrics during tests
 */
export class MetricsCollector {
  private operationTimings: number[] = [];
  private bytesProcessed: number = 0;
  private operationCount: number = 0;
  private errors: Map<string, number> = new Map();
  private errorDetails: Array<{
    timestamp: number;
    type: string;
    message: string;
    stack?: string;
    context?: any;
  }> = [];
  private startTime: number = 0;
  private testName: string = '';
  private memorySnapshots: MemoryMetrics[] = [];

  constructor(testName: string = 'unnamed-test') {
    this.testName = testName;
    this.reset();
  }

  /**
   * Reset all metrics
   */
  reset(): void {
    this.operationTimings = [];
    this.bytesProcessed = 0;
    this.operationCount = 0;
    this.errors.clear();
    this.errorDetails = [];
    this.startTime = performance.now();
    this.memorySnapshots = [];
  }

  /**
   * Start timing an operation
   */
  startOperation(): () => void {
    const start = performance.now();
    return () => {
      const duration = performance.now() - start;
      this.operationTimings.push(duration);
      this.operationCount++;
    };
  }

  /**
   * Record bytes processed
   */
  recordBytes(bytes: number): void {
    this.bytesProcessed += bytes;
  }

  /**
   * Record an error
   */
  recordError(
    errorType: string,
    message?: string,
    stack?: string,
    context?: any
  ): void {
    this.errors.set(errorType, (this.errors.get(errorType) || 0) + 1);

    const errorDetail = {
      timestamp: Date.now(),
      type: errorType,
      message: message || errorType,
      stack,
      context,
    };
    this.errorDetails.push(errorDetail);

    console.error(`❌ Error recorded:`, {
      type: errorType,
      message,
      context,
      time: new Date().toISOString(),
    });
  }

  /**
   * Collect memory metrics from the browser
   */
  async collectMemoryMetrics(): Promise<MemoryMetrics> {
    const metrics: MemoryMetrics = {
      heapUsed: 0,
      heapTotal: 0,
      wasmMemory: 0,
      indexedDBSize: 0,
    };

    // Get JavaScript heap memory (check if we're in browser context)
    if (typeof performance !== 'undefined' && 'memory' in performance) {
      const memInfo = (performance as any).memory;
      metrics.heapUsed = memInfo.usedJSHeapSize;
      metrics.heapTotal = memInfo.totalJSHeapSize;
    }

    // Estimate WASM memory (this is a rough estimate)
    // In a real implementation, you'd query the actual WASM module
    metrics.wasmMemory = await this.estimateWasmMemory();

    // Get IndexedDB size
    metrics.indexedDBSize = await this.getIndexedDBSize();

    this.memorySnapshots.push(metrics);

    return metrics;
  }

  /**
   * Estimate WASM memory usage
   */
  private async estimateWasmMemory(): Promise<number> {
    // This is a placeholder - in production, you'd query the actual WASM module
    // For now, return a rough estimate based on heap usage
    // Check if we're in browser context
    if (typeof performance !== 'undefined' && 'memory' in performance) {
      const memInfo = (performance as any).memory;
      // Assume WASM uses about 20% of total memory
      return Math.floor(memInfo.totalJSHeapSize * 0.2);
    }
    return 0;
  }

  /**
   * Get IndexedDB storage size
   */
  private async getIndexedDBSize(): Promise<number> {
    // Check if we're in a browser environment
    if (
      typeof navigator !== 'undefined' &&
      'storage' in navigator &&
      'estimate' in navigator.storage
    ) {
      try {
        const estimate = await navigator.storage.estimate();
        return estimate.usage || 0;
      } catch (error) {
        console.warn('Failed to estimate IndexedDB size:', error);
      }
    }
    return 0;
  }

  /**
   * Calculate latency percentiles
   */
  calculateLatencyPercentiles(): LatencyMetrics {
    if (this.operationTimings.length === 0) {
      return {
        min: 0,
        max: 0,
        mean: 0,
        median: 0,
        p50: 0,
        p95: 0,
        p99: 0,
      };
    }

    const sorted = [...this.operationTimings].sort((a, b) => a - b);
    const sum = sorted.reduce((a, b) => a + b, 0);

    return {
      min: sorted[0],
      max: sorted[sorted.length - 1],
      mean: sum / sorted.length,
      median: sorted[Math.floor(sorted.length / 2)],
      p50: sorted[Math.floor(sorted.length * 0.5)],
      p95: sorted[Math.floor(sorted.length * 0.95)],
      p99: sorted[Math.floor(sorted.length * 0.99)],
    };
  }

  /**
   * Calculate throughput metrics
   */
  calculateThroughput() {
    const elapsedSeconds = (performance.now() - this.startTime) / 1000;

    return {
      operationsPerSecond:
        elapsedSeconds > 0 ? this.operationCount / elapsedSeconds : 0,
      bytesPerSecond:
        elapsedSeconds > 0 ? this.bytesProcessed / elapsedSeconds : 0,
      totalOperations: this.operationCount,
      totalBytes: this.bytesProcessed,
    };
  }

  /**
   * Get current metrics snapshot
   */
  async getMetrics(): Promise<BenchmarkMetrics> {
    const memoryMetrics = await this.collectMemoryMetrics();
    const latencyMetrics = this.calculateLatencyPercentiles();
    const throughputMetrics = this.calculateThroughput();

    return {
      testName: this.testName,
      timestamp: Date.now(),
      throughput: throughputMetrics,
      latency: latencyMetrics,
      memory: memoryMetrics,
      errors: {
        count: Array.from(this.errors.values()).reduce((a, b) => a + b, 0),
        types: this.errors,
        lastError:
          this.errors.size > 0
            ? Array.from(this.errors.keys()).pop()
            : undefined,
      },
    };
  }

  /**
   * Generate a summary report
   */
  async generateReport(): Promise<string> {
    const metrics = await this.getMetrics();

    const report = `
=== Performance Report: ${metrics.testName} ===
Timestamp: ${new Date(metrics.timestamp).toISOString()}

THROUGHPUT:
- Operations/sec: ${metrics.throughput.operationsPerSecond.toFixed(2)}
- Bytes/sec: ${(metrics.throughput.bytesPerSecond / (1024 * 1024)).toFixed(2)} MB/s
- Total Operations: ${metrics.throughput.totalOperations}
- Total Bytes: ${(metrics.throughput.totalBytes / (1024 * 1024)).toFixed(2)} MB

LATENCY (ms):
- Min: ${metrics.latency.min.toFixed(2)}
- Mean: ${metrics.latency.mean.toFixed(2)}
- Median: ${metrics.latency.median.toFixed(2)}
- P95: ${metrics.latency.p95.toFixed(2)}
- P99: ${metrics.latency.p99.toFixed(2)}
- Max: ${metrics.latency.max.toFixed(2)}

MEMORY:
- Heap Used: ${(metrics.memory.heapUsed / (1024 * 1024)).toFixed(2)} MB
- Heap Total: ${(metrics.memory.heapTotal / (1024 * 1024)).toFixed(2)} MB
- WASM Memory: ${(metrics.memory.wasmMemory / (1024 * 1024)).toFixed(2)} MB
- IndexedDB Size: ${(metrics.memory.indexedDBSize / (1024 * 1024)).toFixed(2)} MB

ERRORS:
- Total: ${metrics.errors.count}
${Array.from(metrics.errors.types.entries())
  .map(entry => `  - ${entry[0]}: ${entry[1]}`)
  .join('\n')}
`;

    return report;
  }

  /**
   * Export metrics as JSON
   */
  async exportJSON(): Promise<string> {
    const metrics = await this.getMetrics();
    return JSON.stringify(
      metrics,
      (_key, value) => {
        if (value instanceof Map) {
          return Object.fromEntries(value);
        }
        return value;
      },
      2
    );
  }

  /**
   * Export metrics as CSV
   */
  async exportCSV(): Promise<string> {
    const metrics = await this.getMetrics();

    const headers = [
      'Test Name',
      'Timestamp',
      'Ops/sec',
      'MB/sec',
      'Total Ops',
      'Total MB',
      'Min Latency',
      'Mean Latency',
      'P95 Latency',
      'P99 Latency',
      'Max Latency',
      'Heap Used MB',
      'Heap Total MB',
      'WASM MB',
      'IndexedDB MB',
      'Error Count',
    ];

    const values = [
      metrics.testName,
      metrics.timestamp,
      metrics.throughput.operationsPerSecond.toFixed(2),
      (metrics.throughput.bytesPerSecond / (1024 * 1024)).toFixed(2),
      metrics.throughput.totalOperations,
      (metrics.throughput.totalBytes / (1024 * 1024)).toFixed(2),
      metrics.latency.min.toFixed(2),
      metrics.latency.mean.toFixed(2),
      metrics.latency.p95.toFixed(2),
      metrics.latency.p99.toFixed(2),
      metrics.latency.max.toFixed(2),
      (metrics.memory.heapUsed / (1024 * 1024)).toFixed(2),
      (metrics.memory.heapTotal / (1024 * 1024)).toFixed(2),
      (metrics.memory.wasmMemory / (1024 * 1024)).toFixed(2),
      (metrics.memory.indexedDBSize / (1024 * 1024)).toFixed(2),
      metrics.errors.count,
    ];

    return headers.join(',') + '\n' + values.join(',');
  }

  /**
   * Get memory trend over time
   */
  getMemoryTrend(): MemoryMetrics[] {
    return [...this.memorySnapshots];
  }

  /**
   * Check for memory leaks by analyzing memory trend
   */
  detectMemoryLeak(thresholdMB: number = 50): boolean {
    if (this.memorySnapshots.length < 10) {
      return false; // Not enough data
    }

    // Get first and last quarter of snapshots
    const quarterSize = Math.floor(this.memorySnapshots.length / 4);
    const firstQuarter = this.memorySnapshots.slice(0, quarterSize);
    const lastQuarter = this.memorySnapshots.slice(-quarterSize);

    // Calculate average heap usage
    const avgFirst =
      firstQuarter.reduce((sum, m) => sum + m.heapUsed, 0) /
      firstQuarter.length;
    const avgLast =
      lastQuarter.reduce((sum, m) => sum + m.heapUsed, 0) / lastQuarter.length;

    // Check if memory increased by more than threshold
    const increaseMB = (avgLast - avgFirst) / (1024 * 1024);

    return increaseMB > thresholdMB;
  }

  getErrorDetails() {
    return [...this.errorDetails];
  }

  getErrorSummary() {
    return {
      totalErrors: this.errorDetails.length,
      errorsByType: Object.fromEntries(this.errors),
      recentErrors: this.errorDetails.slice(-10),
    };
  }
}

// Export singleton for convenience
export const metricsCollector = new MetricsCollector();
