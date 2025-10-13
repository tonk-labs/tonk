import { MetricsCollector } from './metrics-collector';
import { ServerProfiler } from './server-profiler';
import { ConnectionManager } from './connection-manager';
import * as fs from 'fs';
import * as path from 'path';

export interface UptimeSnapshot {
  timestamp: number;
  elapsedSeconds: number;
  connectionCount: number;
  healthyConnections: number;
  totalOperations: number;
  operationsPerSecond: number;
  latencyP50: number;
  latencyP95: number;
  latencyP99: number;
  avgLatency: number;
  errorCount: number;
  errorRate: number;
  memoryUsedMB: number;
  memoryTotalMB: number;
  serverMemoryMB?: number;
  throughputMBps?: number;
}

export interface UptimeReport {
  testName: string;
  startTime: number;
  endTime: number;
  durationSeconds: number;
  snapshots: UptimeSnapshot[];
  summary: {
    totalOperations: number;
    totalErrors: number;
    errorRate: number;
    avgLatency: number;
    p95Latency: number;
    p99Latency: number;
    maxConnections: number;
    avgConnectionCount: number;
    memoryGrowthMB: number;
    degradationDetected: boolean;
    healthScore: number;
  };
}

export class UptimeLogger {
  private testName: string;
  private snapshots: UptimeSnapshot[] = [];
  private startTime: number = 0;
  private logInterval: NodeJS.Timeout | null = null;
  private metricsCollector: MetricsCollector;
  private serverProfiler: ServerProfiler;
  private connectionManager: ConnectionManager | null = null;
  private outputDir: string;

  constructor(
    testName: string,
    metricsCollector: MetricsCollector,
    serverProfiler: ServerProfiler,
    outputDir: string = './test-results/uptime-metrics'
  ) {
    this.testName = testName;
    this.metricsCollector = metricsCollector;
    this.serverProfiler = serverProfiler;
    this.outputDir = outputDir;

    fs.mkdirSync(this.outputDir, { recursive: true });
  }

  setConnectionManager(connectionManager: ConnectionManager): void {
    this.connectionManager = connectionManager;
  }

  startLogging(intervalMs: number = 30000): void {
    this.startTime = Date.now();
    this.snapshots = [];

    console.log(
      `[UptimeLogger] Started logging for "${this.testName}" (interval: ${intervalMs}ms)`
    );

    this.serverProfiler.startProfiling(this.testName, intervalMs);

    this.logInterval = setInterval(() => {
      this.captureSnapshot().catch(error => {
        console.error('[UptimeLogger] Error capturing snapshot:', error);
      });
    }, intervalMs);

    this.captureSnapshot().catch(error => {
      console.error('[UptimeLogger] Error capturing initial snapshot:', error);
    });
  }

  async captureSnapshot(): Promise<void> {
    const now = Date.now();
    const elapsedSeconds = (now - this.startTime) / 1000;

    const metrics = await this.metricsCollector.getMetrics();
    const connectionStats = this.connectionManager?.getStats();

    let serverMemoryMB: number | undefined;
    const serverProfile = this.serverProfiler.getProfile(this.testName);
    if (serverProfile && serverProfile.snapshots.length > 0) {
      const latestSnapshot =
        serverProfile.snapshots[serverProfile.snapshots.length - 1];
      serverMemoryMB = latestSnapshot.rss / (1024 * 1024);
    }

    if (this.connectionManager) {
      this.serverProfiler.setConnectionCount(
        this.connectionManager.getConnectionCount()
      );
      this.serverProfiler.incrementOperations(
        connectionStats?.totalOperations || 0
      );
    }

    const snapshot: UptimeSnapshot = {
      timestamp: now,
      elapsedSeconds,
      connectionCount: connectionStats?.totalConnections || 0,
      healthyConnections: connectionStats?.healthyConnections || 0,
      totalOperations: metrics.throughput.totalOperations,
      operationsPerSecond: metrics.throughput.operationsPerSecond,
      latencyP50: metrics.latency.p50,
      latencyP95: metrics.latency.p95,
      latencyP99: metrics.latency.p99,
      avgLatency: metrics.latency.mean,
      errorCount: metrics.errors.count,
      errorRate:
        metrics.throughput.totalOperations > 0
          ? (metrics.errors.count / metrics.throughput.totalOperations) * 100
          : 0,
      memoryUsedMB: metrics.memory.heapUsed / (1024 * 1024),
      memoryTotalMB: metrics.memory.heapTotal / (1024 * 1024),
      serverMemoryMB,
      throughputMBps: metrics.throughput.bytesPerSecond / (1024 * 1024),
    };

    this.snapshots.push(snapshot);

    console.log(
      `[UptimeLogger] Snapshot ${this.snapshots.length} @ ${elapsedSeconds.toFixed(0)}s: ` +
        `${snapshot.connectionCount} conns, ${snapshot.totalOperations} ops, ` +
        `P95: ${snapshot.latencyP95.toFixed(1)}ms, ` +
        `Mem: ${snapshot.memoryUsedMB.toFixed(1)}MB, ` +
        `Errors: ${snapshot.errorCount}`
    );
  }

  stopLogging(): UptimeReport {
    if (this.logInterval) {
      clearInterval(this.logInterval);
      this.logInterval = null;
    }

    this.serverProfiler.stopProfiling();

    const endTime = Date.now();
    const durationSeconds = (endTime - this.startTime) / 1000;

    const report = this.generateReport(endTime, durationSeconds);

    console.log(
      `[UptimeLogger] Stopped logging for "${this.testName}" (${this.snapshots.length} snapshots)`
    );

    return report;
  }

  private generateReport(
    endTime: number,
    durationSeconds: number
  ): UptimeReport {
    if (this.snapshots.length === 0) {
      return {
        testName: this.testName,
        startTime: this.startTime,
        endTime,
        durationSeconds,
        snapshots: [],
        summary: {
          totalOperations: 0,
          totalErrors: 0,
          errorRate: 0,
          avgLatency: 0,
          p95Latency: 0,
          p99Latency: 0,
          maxConnections: 0,
          avgConnectionCount: 0,
          memoryGrowthMB: 0,
          degradationDetected: false,
          healthScore: 0,
        },
      };
    }

    const firstSnapshot = this.snapshots[0];
    const lastSnapshot = this.snapshots[this.snapshots.length - 1];

    const totalOperations = lastSnapshot.totalOperations;
    const totalErrors = lastSnapshot.errorCount;

    const avgLatency =
      this.snapshots.reduce((sum, s) => sum + s.avgLatency, 0) /
      this.snapshots.length;
    const p95Latency =
      this.snapshots.reduce((sum, s) => sum + s.latencyP95, 0) /
      this.snapshots.length;
    const p99Latency =
      this.snapshots.reduce((sum, s) => sum + s.latencyP99, 0) /
      this.snapshots.length;

    const maxConnections = Math.max(
      ...this.snapshots.map(s => s.connectionCount)
    );
    const avgConnectionCount =
      this.snapshots.reduce((sum, s) => sum + s.connectionCount, 0) /
      this.snapshots.length;

    const memoryGrowthMB =
      lastSnapshot.memoryUsedMB - firstSnapshot.memoryUsedMB;

    const degradationDetected = this.detectPerformanceDegradation();

    const healthScore = this.calculateHealthScore(
      totalErrors,
      totalOperations,
      degradationDetected,
      memoryGrowthMB
    );

    const errorRate =
      totalOperations > 0 ? (totalErrors / totalOperations) * 100 : 0;

    return {
      testName: this.testName,
      startTime: this.startTime,
      endTime,
      durationSeconds,
      snapshots: this.snapshots,
      summary: {
        totalOperations,
        totalErrors,
        errorRate,
        avgLatency,
        p95Latency,
        p99Latency,
        maxConnections,
        avgConnectionCount,
        memoryGrowthMB,
        degradationDetected,
        healthScore,
      },
    };
  }

  detectPerformanceDegradation(): boolean {
    if (this.snapshots.length < 10) {
      return false;
    }

    const quarterSize = Math.floor(this.snapshots.length / 4);
    const firstQuarter = this.snapshots.slice(0, quarterSize);
    const lastQuarter = this.snapshots.slice(-quarterSize);

    const avgFirstP95 =
      firstQuarter.reduce((sum, s) => sum + s.latencyP95, 0) /
      firstQuarter.length;
    const avgLastP95 =
      lastQuarter.reduce((sum, s) => sum + s.latencyP95, 0) /
      lastQuarter.length;

    const latencyIncrease = ((avgLastP95 - avgFirstP95) / avgFirstP95) * 100;

    return latencyIncrease > 20;
  }

  private calculateHealthScore(
    totalErrors: number,
    totalOperations: number,
    degradationDetected: boolean,
    memoryGrowthMB: number
  ): number {
    let score = 100;

    const errorRate =
      totalOperations > 0 ? (totalErrors / totalOperations) * 100 : 0;
    score -= errorRate * 10;

    if (degradationDetected) {
      score -= 20;
    }

    if (memoryGrowthMB > 100) {
      score -= 15;
    } else if (memoryGrowthMB > 50) {
      score -= 10;
    }

    return Math.max(0, Math.min(100, score));
  }

  exportCSV(): string {
    const filename = `${this.testName}-${new Date().toISOString().replace(/[:.]/g, '-')}.csv`;
    const filepath = path.join(this.outputDir, filename);

    const headers = [
      'Timestamp',
      'Elapsed (s)',
      'Connections',
      'Healthy Connections',
      'Total Operations',
      'Ops/sec',
      'Latency P50 (ms)',
      'Latency P95 (ms)',
      'Latency P99 (ms)',
      'Avg Latency (ms)',
      'Error Count',
      'Error Rate (%)',
      'Memory Used (MB)',
      'Memory Total (MB)',
      'Server Memory (MB)',
      'Throughput (MB/s)',
    ];

    const rows = this.snapshots.map(s => [
      s.timestamp,
      s.elapsedSeconds.toFixed(1),
      s.connectionCount,
      s.healthyConnections,
      s.totalOperations,
      s.operationsPerSecond.toFixed(2),
      s.latencyP50.toFixed(2),
      s.latencyP95.toFixed(2),
      s.latencyP99.toFixed(2),
      s.avgLatency.toFixed(2),
      s.errorCount,
      s.errorRate.toFixed(2),
      s.memoryUsedMB.toFixed(2),
      s.memoryTotalMB.toFixed(2),
      s.serverMemoryMB?.toFixed(2) || '',
      s.throughputMBps?.toFixed(2) || '',
    ]);

    const csv =
      headers.join(',') + '\n' + rows.map(row => row.join(',')).join('\n');

    fs.writeFileSync(filepath, csv, 'utf-8');
    console.log(`[UptimeLogger] Exported CSV to: ${filepath}`);

    return filepath;
  }

  exportJSON(): string {
    const endTime = Date.now();
    const durationSeconds = (endTime - this.startTime) / 1000;
    const report = this.generateReport(endTime, durationSeconds);

    const filename = `${this.testName}-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    const filepath = path.join(this.outputDir, filename);

    fs.writeFileSync(filepath, JSON.stringify(report, null, 2), 'utf-8');
    console.log(`[UptimeLogger] Exported JSON to: ${filepath}`);

    return filepath;
  }

  generateTextReport(): string {
    const endTime = Date.now();
    const durationSeconds = (endTime - this.startTime) / 1000;
    const report = this.generateReport(endTime, durationSeconds);

    const summary = report.summary;

    const textReport = `
═══════════════════════════════════════════════════════════
  UPTIME TEST REPORT: ${report.testName}
═══════════════════════════════════════════════════════════

DURATION: ${durationSeconds.toFixed(1)}s (${(durationSeconds / 60).toFixed(1)} minutes)
SNAPSHOTS: ${this.snapshots.length}

OPERATIONS:
  Total Operations:     ${summary.totalOperations}
  Total Errors:         ${summary.totalErrors}
  Error Rate:           ${((summary.totalErrors / summary.totalOperations) * 100).toFixed(2)}%
  
LATENCY:
  Average:              ${summary.avgLatency.toFixed(2)}ms
  P95:                  ${summary.p95Latency.toFixed(2)}ms
  P99:                  ${summary.p99Latency.toFixed(2)}ms

CONNECTIONS:
  Max Connections:      ${summary.maxConnections}
  Avg Connections:      ${summary.avgConnectionCount.toFixed(1)}

MEMORY:
  Memory Growth:        ${summary.memoryGrowthMB.toFixed(2)}MB
  ${summary.memoryGrowthMB > 50 ? '⚠️  High memory growth detected' : '✓ Memory growth acceptable'}

PERFORMANCE:
  Degradation Detected: ${summary.degradationDetected ? '⚠️  YES' : '✓ NO'}
  Health Score:         ${summary.healthScore.toFixed(0)}/100 ${this.getHealthEmoji(summary.healthScore)}

SNAPSHOTS (First, Mid, Last):
  ${this.formatSnapshot(this.snapshots[0])}
  ${this.formatSnapshot(this.snapshots[Math.floor(this.snapshots.length / 2)])}
  ${this.formatSnapshot(this.snapshots[this.snapshots.length - 1])}

═══════════════════════════════════════════════════════════
`;

    return textReport;
  }

  private formatSnapshot(snapshot: UptimeSnapshot): string {
    return `@ ${snapshot.elapsedSeconds.toFixed(0)}s: ${snapshot.connectionCount} conns, ${snapshot.totalOperations} ops, P95: ${snapshot.latencyP95.toFixed(1)}ms, Mem: ${snapshot.memoryUsedMB.toFixed(1)}MB`;
  }

  private getHealthEmoji(score: number): string {
    if (score >= 90) return '🟢';
    if (score >= 70) return '🟡';
    return '🔴';
  }

  saveReport(): string[] {
    const csvPath = this.exportCSV();
    const jsonPath = this.exportJSON();

    const textReport = this.generateTextReport();
    const txtFilename = `${this.testName}-${new Date().toISOString().replace(/[:.]/g, '-')}.txt`;
    const txtPath = path.join(this.outputDir, txtFilename);
    fs.writeFileSync(txtPath, textReport, 'utf-8');

    console.log(textReport);

    return [csvPath, jsonPath, txtPath];
  }
}
