import type {
  RelayMetrics,
  WorkerMetrics,
  AggregateMetrics,
  WorkerInfo,
  TestReport,
  TestPhase,
} from './types';
import { promises as fs } from 'fs';
import { join } from 'path';

export class MetricsAggregator {
  private relayUrl: string;
  private metricsHistory: AggregateMetrics[] = [];
  private pollInterval?: NodeJS.Timeout;
  private testStartTime: number;
  private testName: string;
  private outputDir: string;

  constructor(relayUrl: string, testName: string, outputDir?: string) {
    this.relayUrl = relayUrl;
    this.testName = testName;
    this.testStartTime = Date.now();
    this.outputDir =
      outputDir ||
      join(
        process.cwd(),
        'test-results',
        'distributed',
        `${testName}-${Date.now()}`
      );
  }

  async fetchRelayMetrics(): Promise<RelayMetrics | null> {
    try {
      const response = await fetch(`${this.relayUrl}/metrics`);
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      return await response.json();
    } catch (error) {
      console.error('Error fetching relay metrics:', error);
      return null;
    }
  }

  async fetchWorkerMetrics(worker: WorkerInfo): Promise<WorkerMetrics | null> {
    try {
      const response = await fetch(`http://${worker.publicIp}:3000/metrics`);
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      return await response.json();
    } catch (error) {
      console.error(
        `Error fetching metrics from worker ${worker.workerId}:`,
        error
      );
      return null;
    }
  }

  async collectMetrics(
    workers: WorkerInfo[]
  ): Promise<AggregateMetrics | null> {
    const relay = await this.fetchRelayMetrics();
    if (!relay) {
      return null;
    }

    const workerMetricsPromises = workers.map(w => this.fetchWorkerMetrics(w));
    const workerMetricsResults = await Promise.all(workerMetricsPromises);
    const workerMetrics = workerMetricsResults.filter(
      (m): m is WorkerMetrics => m !== null
    );

    const timestamp = Date.now();

    const totalConnections = workerMetrics.reduce(
      (sum, w) => sum + w.connections.totalConnections,
      0
    );

    const healthyConnections = workerMetrics.reduce(
      (sum, w) => sum + w.connections.healthyConnections,
      0
    );

    const totalOperations = workerMetrics.reduce(
      (sum, w) => sum + w.operations.completed,
      0
    );

    const totalErrors = workerMetrics.reduce(
      (sum, w) => sum + w.errors.count,
      0
    );

    const operationsPerSecond = workerMetrics.reduce(
      (sum, w) => sum + w.operations.operationsPerSecond,
      0
    );

    const errorRate =
      totalOperations > 0 ? (totalErrors / totalOperations) * 100 : 0;

    const allLatencies = workerMetrics.flatMap(w => [
      w.latency.min,
      w.latency.max,
      w.latency.mean,
      w.latency.p50,
      w.latency.p95,
      w.latency.p99,
    ]);

    const latency = {
      min: Math.min(...allLatencies.filter(l => l > 0)),
      max: Math.max(...allLatencies),
      mean:
        allLatencies.reduce((sum, l) => sum + l, 0) / allLatencies.length || 0,
      p50: this.calculatePercentile(
        workerMetrics.map(w => w.latency.p50),
        0.5
      ),
      p95: this.calculatePercentile(
        workerMetrics.map(w => w.latency.p95),
        0.95
      ),
      p99: this.calculatePercentile(
        workerMetrics.map(w => w.latency.p99),
        0.99
      ),
    };

    const totalWorkerRss = workerMetrics.reduce(
      (sum, w) => sum + w.memory.rss,
      0
    );

    const aggregate: AggregateMetrics = {
      timestamp,
      relay,
      workers: workerMetrics,
      aggregate: {
        totalConnections,
        healthyConnections,
        totalOperations,
        operationsPerSecond,
        errorRate,
        latency,
        memory: {
          relayRss: relay.memory.rss,
          totalWorkerRss,
          totalRss: relay.memory.rss + totalWorkerRss,
        },
      },
    };

    this.metricsHistory.push(aggregate);
    return aggregate;
  }

  private calculatePercentile(values: number[], percentile: number): number {
    if (values.length === 0) return 0;
    const sorted = values.sort((a, b) => a - b);
    const index = Math.floor(sorted.length * percentile);
    return sorted[index] || 0;
  }

  startPolling(workers: WorkerInfo[], intervalMs: number = 5000): void {
    console.log(`Starting metrics polling every ${intervalMs}ms`);

    this.pollInterval = setInterval(async () => {
      await this.collectMetrics(workers);
    }, intervalMs);
  }

  stopPolling(): void {
    if (this.pollInterval) {
      clearInterval(this.pollInterval);
      this.pollInterval = undefined;
      console.log('Metrics polling stopped');
    }
  }

  getLatestMetrics(): AggregateMetrics | null {
    return this.metricsHistory[this.metricsHistory.length - 1] || null;
  }

  getMetricsHistory(): AggregateMetrics[] {
    return [...this.metricsHistory];
  }

  getMetricsSummary(): {
    totalSamples: number;
    duration: number;
    peakConnections: number;
    avgConnections: number;
    peakLatency: number;
    avgLatency: number;
    totalOperations: number;
    avgErrorRate: number;
  } {
    if (this.metricsHistory.length === 0) {
      return {
        totalSamples: 0,
        duration: 0,
        peakConnections: 0,
        avgConnections: 0,
        peakLatency: 0,
        avgLatency: 0,
        totalOperations: 0,
        avgErrorRate: 0,
      };
    }

    const connections = this.metricsHistory.map(
      m => m.aggregate.totalConnections
    );
    const latencies = this.metricsHistory.map(m => m.aggregate.latency.p95);
    const errorRates = this.metricsHistory.map(m => m.aggregate.errorRate);
    const lastMetrics = this.metricsHistory[this.metricsHistory.length - 1];

    return {
      totalSamples: this.metricsHistory.length,
      duration: Date.now() - this.testStartTime,
      peakConnections: Math.max(...connections),
      avgConnections:
        connections.reduce((sum, c) => sum + c, 0) / connections.length,
      peakLatency: Math.max(...latencies),
      avgLatency: latencies.reduce((sum, l) => sum + l, 0) / latencies.length,
      totalOperations: lastMetrics.aggregate.totalOperations,
      avgErrorRate:
        errorRates.reduce((sum, e) => sum + e, 0) / errorRates.length,
    };
  }

  async detectAnomalies(): Promise<{
    latencySpikes: { timestamp: number; value: number }[];
    memoryLeaks: { startMB: number; endMB: number; growthMB: number };
    connectionDrops: { timestamp: number; drop: number }[];
  }> {
    const latencySpikes: { timestamp: number; value: number }[] = [];
    const connectionDrops: { timestamp: number; drop: number }[] = [];

    if (this.metricsHistory.length < 10) {
      return {
        latencySpikes,
        memoryLeaks: { startMB: 0, endMB: 0, growthMB: 0 },
        connectionDrops,
      };
    }

    const latencies = this.metricsHistory.map(m => m.aggregate.latency.p95);
    const avgLatency =
      latencies.reduce((sum, l) => sum + l, 0) / latencies.length;
    const latencyThreshold = avgLatency * 2;

    for (let i = 0; i < this.metricsHistory.length; i++) {
      const metrics = this.metricsHistory[i];
      if (metrics.aggregate.latency.p95 > latencyThreshold) {
        latencySpikes.push({
          timestamp: metrics.timestamp,
          value: metrics.aggregate.latency.p95,
        });
      }

      if (i > 0) {
        const prev = this.metricsHistory[i - 1];
        const connectionDrop =
          prev.aggregate.totalConnections - metrics.aggregate.totalConnections;
        if (connectionDrop > 10) {
          connectionDrops.push({
            timestamp: metrics.timestamp,
            drop: connectionDrop,
          });
        }
      }
    }

    const quarterSize = Math.floor(this.metricsHistory.length / 4);
    const firstQuarter = this.metricsHistory.slice(0, quarterSize);
    const lastQuarter = this.metricsHistory.slice(-quarterSize);

    const avgFirstMemory =
      firstQuarter.reduce((sum, m) => sum + m.relay.memory.rss, 0) /
      firstQuarter.length;
    const avgLastMemory =
      lastQuarter.reduce((sum, m) => sum + m.relay.memory.rss, 0) /
      lastQuarter.length;

    const memoryLeaks = {
      startMB: avgFirstMemory / (1024 * 1024),
      endMB: avgLastMemory / (1024 * 1024),
      growthMB: (avgLastMemory - avgFirstMemory) / (1024 * 1024),
    };

    return { latencySpikes, memoryLeaks, connectionDrops };
  }

  async generateReport(
    scenario: string,
    phases: {
      phase: TestPhase;
      startTime: number;
      endTime: number;
      duration: number;
      status: 'completed' | 'failed' | 'skipped';
    }[]
  ): Promise<TestReport> {
    const endTime = Date.now();
    const duration = endTime - this.testStartTime;
    const summary = this.getMetricsSummary();
    const anomalies = await this.detectAnomalies();

    const lastMetrics = this.getLatestMetrics();

    const workerStats =
      lastMetrics?.workers.map(w => ({
        workerId: w.workerId,
        connections: w.connections.totalConnections,
        operations: w.operations.completed,
        errors: w.errors.count,
        avgLatency: w.latency.mean,
      })) || [];

    const report: TestReport = {
      testName: this.testName,
      scenario,
      startTime: this.testStartTime,
      endTime,
      duration,
      status: 'passed',
      phases: phases.map(p => ({
        ...p,
        metrics: this.metricsHistory.find(
          m => m.timestamp >= p.startTime && m.timestamp <= p.endTime
        ),
      })),
      summary: {
        totalConnections: summary.avgConnections,
        peakConnections: summary.peakConnections,
        totalOperations: summary.totalOperations,
        totalErrors: Math.floor(
          summary.totalOperations * (summary.avgErrorRate / 100)
        ),
        errorRate: summary.avgErrorRate,
        avgLatency: summary.avgLatency,
        p95Latency: summary.peakLatency,
        p99Latency: summary.peakLatency,
        maxLatency: summary.peakLatency,
        relayMemoryGrowth: anomalies.memoryLeaks.growthMB,
        workerCost: 0,
        totalCost: 0,
      },
      workerStats,
      metricsTimeSeries: this.metricsHistory,
    };

    return report;
  }

  async saveReport(report: TestReport): Promise<string[]> {
    await fs.mkdir(this.outputDir, { recursive: true });

    const files: string[] = [];

    const summaryPath = join(this.outputDir, 'summary.json');
    await fs.writeFile(summaryPath, JSON.stringify(report, null, 2));
    files.push(summaryPath);

    const metricsPath = join(this.outputDir, 'metrics.json');
    await fs.writeFile(
      metricsPath,
      JSON.stringify(this.metricsHistory, null, 2)
    );
    files.push(metricsPath);

    const csvPath = join(this.outputDir, 'metrics.csv');
    await fs.writeFile(csvPath, this.generateCSV());
    files.push(csvPath);

    const logPath = join(this.outputDir, 'test.log');
    await fs.writeFile(logPath, this.generateLogSummary(report));
    files.push(logPath);

    console.log(`Report saved to ${this.outputDir}`);
    console.log(`Files: ${files.join(', ')}`);

    return files;
  }

  private generateCSV(): string {
    const headers = [
      'Timestamp',
      'Connections',
      'Healthy Connections',
      'Operations',
      'Ops/sec',
      'Error Rate',
      'P50 Latency',
      'P95 Latency',
      'P99 Latency',
      'Relay RSS (MB)',
      'Worker RSS (MB)',
    ].join(',');

    const rows = this.metricsHistory.map(m => {
      return [
        m.timestamp,
        m.aggregate.totalConnections,
        m.aggregate.healthyConnections,
        m.aggregate.totalOperations,
        m.aggregate.operationsPerSecond.toFixed(2),
        m.aggregate.errorRate.toFixed(2),
        m.aggregate.latency.p50.toFixed(2),
        m.aggregate.latency.p95.toFixed(2),
        m.aggregate.latency.p99.toFixed(2),
        (m.aggregate.memory.relayRss / (1024 * 1024)).toFixed(2),
        (m.aggregate.memory.totalWorkerRss / (1024 * 1024)).toFixed(2),
      ].join(',');
    });

    return [headers, ...rows].join('\n');
  }

  private generateLogSummary(report: TestReport): string {
    return `
=================================================================
DISTRIBUTED LOAD TEST REPORT
=================================================================
Test: ${report.testName}
Scenario: ${report.scenario}
Status: ${report.status}
Duration: ${(report.duration / 1000 / 60).toFixed(2)} minutes

SUMMARY
-----------------------------------------------------------------
Peak Connections:    ${report.summary.peakConnections}
Total Operations:    ${report.summary.totalOperations}
Total Errors:        ${report.summary.totalErrors}
Error Rate:          ${report.summary.errorRate.toFixed(2)}%
Average Latency:     ${report.summary.avgLatency.toFixed(2)}ms
P95 Latency:         ${report.summary.p95Latency.toFixed(2)}ms
P99 Latency:         ${report.summary.p99Latency.toFixed(2)}ms
Relay Memory Growth: ${report.summary.relayMemoryGrowth.toFixed(2)}MB

PHASES
-----------------------------------------------------------------
${report.phases
  .map(
    p =>
      `${p.phase.padEnd(12)} | ${((p.endTime - p.startTime) / 1000 / 60).toFixed(1)}min | ${p.status}`
  )
  .join('\n')}

WORKER STATS
-----------------------------------------------------------------
${report.workerStats
  .map(
    w =>
      `${w.workerId.padEnd(15)} | ${w.connections} conn | ${w.operations} ops | ${w.errors} err | ${w.avgLatency.toFixed(0)}ms avg`
  )
  .join('\n')}

=================================================================
`.trim();
  }
}
