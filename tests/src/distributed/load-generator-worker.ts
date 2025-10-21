#!/usr/bin/env node

import express from 'express';
import { chromium, Browser } from '@playwright/test';
import { ConnectionManager } from '../utils/connection-manager';
import { MetricsCollector } from '../utils/metrics-collector';
import type { WorkerConfig, WorkerMetrics, WorkerCommand } from './types';
import type { ServerInstance } from '../test-ui/types';
import os from 'os';

class LoadGeneratorWorker {
  private config: WorkerConfig;
  private app: express.Express;
  private browser?: Browser;
  private connectionManager?: ConnectionManager;
  private metricsCollector: MetricsCollector;
  private isRunning: boolean = false;
  private operationInterval?: NodeJS.Timeout;
  private heartbeatInterval?: NodeJS.Timeout;
  private syncCheckInterval?: NodeJS.Timeout;
  private serverInstance: ServerInstance;
  private currentTargetConnections: number;
  private currentOperationsPerMinute: number;
  private phaseStartTime: number = 0;
  private sourceOfTruthConnectionId: string | null = null;

  constructor(config: WorkerConfig) {
    this.config = config;
    this.app = express();
    this.app.use(express.json());
    this.metricsCollector = new MetricsCollector(`worker-${config.workerId}`);
    this.currentTargetConnections = config.targetConnections;
    this.currentOperationsPerMinute = config.operationsPerMinute;

    this.serverInstance = {
      process: null as any,
      port: parseInt(new URL(config.relayUrl).port) || 8080,
      wsUrl: config.relayUrl.replace(/^http/, 'ws'),
      manifestUrl: `${config.relayUrl}/.manifest.tonk`,
      testId: config.workerId,
    };

    this.setupRoutes();
  }

  private setupRoutes(): void {
    this.app.post('/command', this.handleCommand.bind(this));
    this.app.get('/metrics', this.handleMetricsQuery.bind(this));
    this.app.get('/health', this.handleHealthCheck.bind(this));
    this.app.get('/status', this.handleStatusQuery.bind(this));
  }

  private async handleCommand(
    req: express.Request,
    res: express.Response
  ): Promise<void> {
    const command: WorkerCommand = req.body;

    try {
      switch (command.type) {
        case 'start':
          const payload = command.payload || {};
          await this.start(
            payload.targetConnections,
            payload.operationsPerMinute
          );
          res.json({ success: true, workerId: this.config.workerId });
          break;

        case 'stop':
          await this.stop();
          res.json({ success: true, workerId: this.config.workerId });
          break;

        case 'pause':
          this.pauseOperations();
          res.json({ success: true, workerId: this.config.workerId });
          break;

        case 'resume':
          this.resumeOperations();
          res.json({ success: true, workerId: this.config.workerId });
          break;

        case 'shutdown':
          res.json({ success: true, workerId: this.config.workerId });
          await this.shutdown();
          break;

        default:
          res.status(400).json({ success: false, error: 'Unknown command' });
      }
    } catch (error) {
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleMetricsQuery(
    _req: express.Request,
    res: express.Response
  ): Promise<void> {
    try {
      const metrics = await this.getMetrics();
      res.json(metrics);
    } catch (error) {
      res.status(500).json({ error: String(error) });
    }
  }

  private async handleHealthCheck(
    _req: express.Request,
    res: express.Response
  ): Promise<void> {
    const status = this.isRunning ? 'healthy' : 'stopped';
    res.json({ status, workerId: this.config.workerId, timestamp: Date.now() });
  }

  private async handleStatusQuery(
    _req: express.Request,
    res: express.Response
  ): Promise<void> {
    try {
      const stats = this.connectionManager?.getStats() || {
        totalConnections: 0,
        healthyConnections: 0,
        avgLatency: 0,
        p95Latency: 0,
        p99Latency: 0,
        totalOperations: 0,
        totalErrors: 0,
        avgOperationsPerConnection: 0,
      };

      res.json({
        workerId: this.config.workerId,
        isRunning: this.isRunning,
        connections: stats.totalConnections,
        targetConnections: this.config.targetConnections,
        stats,
      });
    } catch (error) {
      res.status(500).json({ error: String(error) });
    }
  }

  async initialize(): Promise<void> {
    console.log(`Initializing worker ${this.config.workerId}...`);

    this.browser = await chromium.launch({
      headless: true,
      args: [
        '--disable-dev-shm-usage',
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--disable-gpu',
        '--disable-software-rasterizer',
        '--disable-background-timer-throttling',
        '--disable-backgrounding-occluded-windows',
        '--disable-renderer-backgrounding',
      ],
    });

    this.connectionManager = new ConnectionManager(
      this.browser,
      this.serverInstance
    );

    console.log('✓ Browser initialized');

    await this.registerWithCoordinator();

    this.startHeartbeat();
  }

  private async registerWithCoordinator(): Promise<void> {
    try {
      const registration = {
        workerId: this.config.workerId,
        publicIp: this.config.publicIp,
        privateIp: this.getPrivateIP(),
        capabilities: {
          maxConnections: this.config.targetConnections,
          memory: os.totalmem(),
          cpus: os.cpus().length,
        },
        timestamp: Date.now(),
      };

      const response = await fetch(
        `${this.config.coordinatorUrl}/worker/register`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(registration),
        }
      );

      if (!response.ok) {
        throw new Error(`Registration failed: ${response.status}`);
      }

      console.log('✓ Registered with coordinator');
    } catch (error) {
      console.error('Failed to register with coordinator:', error);
      throw error;
    }
  }

  private startHeartbeat(): void {
    this.heartbeatInterval = setInterval(async () => {
      try {
        const stats = this.connectionManager?.getStats();

        const heartbeat = {
          workerId: this.config.workerId,
          timestamp: Date.now(),
          status:
            stats && stats.healthyConnections > stats.totalConnections * 0.8
              ? 'healthy'
              : stats && stats.healthyConnections > stats.totalConnections * 0.5
                ? 'degraded'
                : 'unhealthy',
          activeConnections: stats?.totalConnections || 0,
          metrics: await this.getMetrics(),
        };

        await fetch(`${this.config.coordinatorUrl}/worker/heartbeat`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(heartbeat),
        });
      } catch (error) {
        console.error('Heartbeat failed:', error);
      }
    }, 10000);
  }

  async start(
    targetConnections?: number,
    operationsPerMinute?: number
  ): Promise<void> {
    const newTargetConnections =
      targetConnections ?? this.currentTargetConnections;
    const newOperationsPerMinute =
      operationsPerMinute ?? this.currentOperationsPerMinute;

    const currentConnectionCount =
      this.connectionManager?.getStats().totalConnections || 0;

    if (
      this.isRunning &&
      currentConnectionCount === newTargetConnections &&
      newOperationsPerMinute === this.currentOperationsPerMinute
    ) {
      console.log(
        `Worker already at target: ${currentConnectionCount}/${newTargetConnections} connections at ${this.currentOperationsPerMinute} ops/min`
      );
      return;
    }

    this.currentTargetConnections = newTargetConnections;
    this.currentOperationsPerMinute = newOperationsPerMinute;

    console.log(
      `Scaling to ${this.currentTargetConnections} connections (current: ${currentConnectionCount})...`
    );

    if (!this.isRunning) {
      this.isRunning = true;
      this.phaseStartTime = Date.now();
    }

    if (this.currentTargetConnections > currentConnectionCount) {
      const connectionsToAdd =
        this.currentTargetConnections - currentConnectionCount;
      const rampUpDuration = this.config.rampUpDuration || 60000;
      const connectionsPerBatch = Math.ceil(connectionsToAdd / 10);
      const batchDelay = rampUpDuration / 10;

      console.log(`Adding ${connectionsToAdd} new connections...`);

      for (let i = 0; i < connectionsToAdd; i += connectionsPerBatch) {
        const count = Math.min(connectionsPerBatch, connectionsToAdd - i);

        console.log(
          `Creating batch of ${count} connections (${currentConnectionCount + i + count}/${this.currentTargetConnections})...`
        );

        await this.connectionManager?.createConnections(
          count,
          `${this.config.workerId}-conn`,
          currentConnectionCount + i
        );

        if (i + connectionsPerBatch < connectionsToAdd) {
          await new Promise(resolve => setTimeout(resolve, batchDelay));
        }
      }

      console.log(`✓ Scaled to ${this.currentTargetConnections} connections`);
    }

    if (
      this.operationInterval &&
      this.currentOperationsPerMinute !== operationsPerMinute
    ) {
      console.log(
        `Updating operations rate to ${this.currentOperationsPerMinute}/min`
      );
      this.pauseOperations();
      this.startOperations(this.currentOperationsPerMinute);
    } else if (!this.operationInterval) {
      this.startOperations(this.currentOperationsPerMinute);
    }
  }

  private startOperations(operationsPerMinute?: number): void {
    const opsPerMin = operationsPerMinute ?? this.currentOperationsPerMinute;

    if (opsPerMin === 0) {
      console.log('No operations configured, connections will remain idle');
      return;
    }

    const operationIntervalMs = (60 * 1000) / opsPerMin;

    console.log(
      `Starting operations at ${opsPerMin}/min (every ${operationIntervalMs}ms)`
    );

    this.operationInterval = setInterval(async () => {
      if (!this.isRunning) return;

      try {
        const connections = this.connectionManager?.getConnectionIds() || [];
        if (connections.length === 0) return;

        const randomConnId =
          connections[Math.floor(Math.random() * connections.length)];

        const endOp = this.metricsCollector.startOperation();

        await this.connectionManager?.executeOperation(
          randomConnId,
          async page => {
            await page.getByTestId('increment-btn').click();
            await page.waitForTimeout(50);
          }
        );

        endOp();
      } catch (error) {
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        const errorStack = error instanceof Error ? error.stack : undefined;

        this.metricsCollector.recordError(
          'operation-failed',
          errorMessage,
          errorStack,
          {
            workerId: this.config.workerId,
            timestamp: Date.now(),
          }
        );

        console.error(`❌ [${this.config.workerId}] Operation failed:`, {
          error: errorMessage,
          stack: errorStack,
          time: new Date().toISOString(),
        });
      }
    }, operationIntervalMs);
  }

  private pauseOperations(): void {
    if (this.operationInterval) {
      clearInterval(this.operationInterval);
      this.operationInterval = undefined;
      console.log('Operations paused');
    }
  }

  private resumeOperations(): void {
    if (!this.operationInterval) {
      this.startOperations(this.currentOperationsPerMinute);
      console.log('Operations resumed');
    }
  }

  async stop(): Promise<void> {
    console.log('Stopping worker...');
    this.isRunning = false;

    this.pauseOperations();

    await this.connectionManager?.closeAll();

    console.log('✓ Worker stopped');
  }

  async shutdown(): Promise<void> {
    console.log('Shutting down worker...');

    await this.stop();

    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
    }

    if (this.browser) {
      await this.browser.close();
    }

    console.log('✓ Worker shutdown complete');
    process.exit(0);
  }

  private async getMetrics(): Promise<WorkerMetrics> {
    const stats = this.connectionManager?.getStats() || {
      totalConnections: 0,
      healthyConnections: 0,
      avgLatency: 0,
      p95Latency: 0,
      p99Latency: 0,
      totalOperations: 0,
      totalErrors: 0,
      avgOperationsPerConnection: 0,
    };

    const memUsage = process.memoryUsage();

    let syncMetrics = undefined;
    if (this.connectionManager && this.isRunning) {
      const timeSinceStart = Date.now() - this.phaseStartTime;
      if (timeSinceStart >= 30000) {
        try {
          if (!this.sourceOfTruthConnectionId) {
            const connIds = this.connectionManager.getConnectionIds();
            if (connIds.length > 0) {
              this.sourceOfTruthConnectionId = connIds[0];
            }
          }

          const syncResult = await this.connectionManager.verifySync(
            this.sourceOfTruthConnectionId || undefined
          );

          const distributionObj: { [value: number]: number } = {};
          for (const [value, count] of syncResult.valueDistribution.entries()) {
            distributionObj[value] = count;
          }

          syncMetrics = {
            sourceOfTruthValue: syncResult.sourceOfTruthValue,
            inSync: syncResult.inSync,
            inSyncCount: syncResult.inSyncCount,
            outOfSyncCount: syncResult.outOfSyncCount,
            totalConnections: syncResult.totalConnections,
            valueDistribution: distributionObj,
            minValue: syncResult.minValue,
            maxValue: syncResult.maxValue,
            medianValue: syncResult.medianValue,
            modeValue: syncResult.modeValue,
            lastCheckTimestamp: Date.now(),
          };

          if (!syncResult.inSync && syncResult.sampleOutliers.length > 0) {
            console.warn(
              `⚠️  [${this.config.workerId}] Sync drift detected: ${syncResult.outOfSyncCount}/${syncResult.totalConnections} out of sync`
            );
            console.warn(
              `   Source of truth (${syncResult.sourceOfTruthId}): ${syncResult.sourceOfTruthValue}`
            );
            console.warn(
              `   Range: ${syncResult.minValue} - ${syncResult.maxValue}, Median: ${syncResult.medianValue}, Mode: ${syncResult.modeValue}`
            );
            console.warn(`   Sample outliers:`);
            for (const outlier of syncResult.sampleOutliers.slice(0, 3)) {
              console.warn(
                `     ${outlier.connectionId}: ${outlier.value} (drift: ${outlier.drift > 0 ? '+' : ''}${outlier.drift})`
              );
            }
          }
        } catch (error) {
          console.warn(
            'Failed to verify sync during metrics collection:',
            error
          );
        }
      }
    }

    return {
      workerId: this.config.workerId,
      timestamp: Date.now(),
      connections: stats,
      operations: {
        completed: stats.totalOperations,
        failed: stats.totalErrors,
        operationsPerSecond:
          stats.totalOperations /
          ((Date.now() - this.metricsCollector['startTime']) / 1000),
      },
      memory: {
        heapUsed: memUsage.heapUsed,
        heapTotal: memUsage.heapTotal,
        rss: memUsage.rss,
      },
      latency: {
        min: 0,
        max: 0,
        mean: stats.avgLatency,
        p50: stats.avgLatency,
        p95: stats.p95Latency,
        p99: stats.p99Latency,
      },
      errors: {
        count: stats.totalErrors,
        types: this.metricsCollector.getErrorSummary().errorsByType,
        details: this.metricsCollector.getErrorDetails(),
        lastError: this.metricsCollector.getErrorDetails().slice(-1)[0],
      },
      sync: syncMetrics,
    };
  }

  private async getPublicIP(): Promise<string> {
    if (this.config.publicIp) {
      return this.config.publicIp;
    }
    try {
      const response = await fetch(
        'http://169.254.169.254/latest/meta-data/public-ipv4',
        {
          signal: AbortSignal.timeout(2000),
        }
      );
      const ip = await response.text();
      return ip || this.config.publicIp || '0.0.0.0';
    } catch {
      return this.config.publicIp || '0.0.0.0';
    }
  }

  private getPrivateIP(): string {
    const interfaces = os.networkInterfaces();
    for (const name of Object.keys(interfaces)) {
      const iface = interfaces[name];
      if (!iface) continue;

      for (const alias of iface) {
        if (alias.family === 'IPv4' && !alias.internal) {
          return alias.address;
        }
      }
    }
    return '127.0.0.1';
  }

  async listen(port: number = 3000): Promise<void> {
    return new Promise((resolve, reject) => {
      this.app
        .listen(port, () => {
          console.log(`Worker API listening on port ${port}`);
          resolve();
        })
        .on('error', reject);
    });
  }
}

async function main() {
  const args = process.argv.slice(2);

  if (args.length < 6) {
    console.error(
      'Usage: load-generator-worker <workerId> <coordinatorUrl> <relayUrl> <targetConnections> <operationsPerMinute> <publicIp> [rampUpDuration]'
    );
    process.exit(1);
  }

  const config: WorkerConfig = {
    workerId: args[0],
    coordinatorUrl: args[1],
    relayUrl: args[2],
    targetConnections: parseInt(args[3]),
    operationsPerMinute: parseInt(args[4] || '60'),
    publicIp: args[5],
    rampUpDuration: parseInt(args[6] || '60000'),
  };

  console.log('Load Generator Worker');
  console.log('=====================');
  console.log(`Worker ID: ${config.workerId}`);
  console.log(`Coordinator: ${config.coordinatorUrl}`);
  console.log(`Relay: ${config.relayUrl}`);
  console.log(`Target Connections: ${config.targetConnections}`);
  console.log(`Operations/min: ${config.operationsPerMinute}`);
  console.log('');

  const worker = new LoadGeneratorWorker(config);

  process.on('SIGINT', async () => {
    console.log('\nReceived SIGINT, shutting down...');
    await worker.shutdown();
  });

  process.on('SIGTERM', async () => {
    console.log('\nReceived SIGTERM, shutting down...');
    await worker.shutdown();
  });

  try {
    await worker.initialize();
    await worker.listen(3000);

    console.log('✓ Worker ready and waiting for commands');
  } catch (error) {
    console.error('Failed to start worker:', error);
    process.exit(1);
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});

export { LoadGeneratorWorker };
