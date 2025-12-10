import express, { type Express, type Request, type Response } from 'express';
import type { Server } from 'http';
import type {
  CoordinatorState,
  TestPhase,
  TestScenario,
  WorkerCommand,
  WorkerCommandResponse,
  WorkerHeartbeat,
  WorkerInfo,
  WorkerMetrics,
  WorkerRegistration,
} from './types';

export class WorkerCoordinator {
  private app: Express;
  private server?: Server;
  private port: number;
  private state: CoordinatorState;

  constructor(port: number, testConfig: TestScenario) {
    this.port = port;
    this.app = express();
    this.app.use(express.json({ limit: '50mb' }));

    this.state = {
      currentPhase: 'provision',
      phaseStartedAt: Date.now(),
      workers: new Map(),
      metricsHistory: [],
      testStartedAt: Date.now(),
      testConfig,
    };

    this.setupRoutes();
  }

  private setupRoutes(): void {
    this.app.post('/worker/register', this.handleWorkerRegistration.bind(this));
    this.app.post('/worker/heartbeat', this.handleWorkerHeartbeat.bind(this));
    this.app.post('/worker/metrics', this.handleWorkerMetrics.bind(this));
    this.app.get('/worker/phase', this.handlePhaseQuery.bind(this));
    this.app.get('/worker/status', this.handleStatusQuery.bind(this));
    this.app.get('/state', this.handleStateQuery.bind(this));
    this.app.get('/health', this.handleHealthCheck.bind(this));
  }

  private async handleWorkerRegistration(
    req: Request,
    res: Response
  ): Promise<void> {
    try {
      const registration: WorkerRegistration = req.body;

      console.log(
        `Worker registered: ${registration.workerId} from ${registration.publicIp}`
      );

      const workerInfo: WorkerInfo = {
        workerId: registration.workerId,
        instanceId: registration.workerId.split('-')[1] || 'unknown',
        publicIp: registration.publicIp,
        privateIp: registration.privateIp,
        status: 'ready',
        registeredAt: registration.timestamp,
        lastHeartbeat: registration.timestamp,
        connections: 0,
        targetConnections: 0,
      };

      this.state.workers.set(registration.workerId, workerInfo);

      res.json({
        success: true,
        workerId: registration.workerId,
        currentPhase: this.state.currentPhase,
        testConfig: this.state.testConfig,
      });
    } catch (error) {
      console.error('Error in worker registration:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleWorkerHeartbeat(
    req: Request,
    res: Response
  ): Promise<void> {
    try {
      const heartbeat: WorkerHeartbeat = req.body;

      const worker = this.state.workers.get(heartbeat.workerId);
      if (!worker) {
        res.status(404).json({ success: false, error: 'Worker not found' });
        return;
      }

      worker.lastHeartbeat = heartbeat.timestamp;
      worker.connections = heartbeat.activeConnections;
      worker.status = heartbeat.status === 'healthy' ? 'running' : 'failed';

      res.json({ success: true, currentPhase: this.state.currentPhase });
    } catch (error) {
      console.error('Error in worker heartbeat:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleWorkerMetrics(
    req: Request,
    res: Response
  ): Promise<void> {
    try {
      const metrics: WorkerMetrics = req.body;

      const worker = this.state.workers.get(metrics.workerId);
      if (worker) {
        worker.lastHeartbeat = metrics.timestamp;
        worker.connections = metrics.connections.totalConnections;
      }

      res.json({ success: true });
    } catch (error) {
      console.error('Error in worker metrics:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handlePhaseQuery(_req: Request, res: Response): Promise<void> {
    try {
      res.json({
        phase: this.state.currentPhase,
        phaseStartedAt: this.state.phaseStartedAt,
      });
    } catch (error) {
      console.error('Error in phase query:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleStatusQuery(_req: Request, res: Response): Promise<void> {
    try {
      const workers = Array.from(this.state.workers.values());

      const summary = {
        totalWorkers: workers.length,
        readyWorkers: workers.filter(w => w.status === 'ready').length,
        runningWorkers: workers.filter(w => w.status === 'running').length,
        failedWorkers: workers.filter(w => w.status === 'failed').length,
        totalConnections: workers.reduce((sum, w) => sum + w.connections, 0),
      };

      res.json({ summary, workers });
    } catch (error) {
      console.error('Error in status query:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleStateQuery(_req: Request, res: Response): Promise<void> {
    try {
      res.json({
        currentPhase: this.state.currentPhase,
        phaseStartedAt: this.state.phaseStartedAt,
        testStartedAt: this.state.testStartedAt,
        workers: Array.from(this.state.workers.values()),
        metricsHistoryCount: this.state.metricsHistory.length,
      });
    } catch (error) {
      console.error('Error in state query:', error);
      res.status(500).json({ success: false, error: String(error) });
    }
  }

  private async handleHealthCheck(_req: Request, res: Response): Promise<void> {
    res.json({ status: 'healthy', timestamp: Date.now() });
  }

  async start(): Promise<void> {
    return new Promise((resolve, reject) => {
      try {
        this.server = this.app.listen(this.port, () => {
          console.log(`Worker coordinator listening on port ${this.port}`);
          resolve();
        });

        this.server.on('error', (error: Error) => {
          console.error('Server error:', error);
          reject(error);
        });
      } catch (error) {
        reject(error);
      }
    });
  }

  async stop(): Promise<void> {
    return new Promise((resolve, reject) => {
      if (!this.server) {
        resolve();
        return;
      }

      this.server.close((error?: Error) => {
        if (error) {
          console.error('Error stopping coordinator:', error);
          reject(error);
        } else {
          console.log('Worker coordinator stopped');
          resolve();
        }
      });
    });
  }

  setPhase(phase: TestPhase): void {
    console.log(`Phase transition: ${this.state.currentPhase} -> ${phase}`);
    this.state.currentPhase = phase;
    this.state.phaseStartedAt = Date.now();
  }

  updateWorkerTarget(workerId: string, targetConnections: number): void {
    const worker = this.state.workers.get(workerId);
    if (worker) {
      worker.targetConnections = targetConnections;
    }
  }

  getWorkers(): WorkerInfo[] {
    return Array.from(this.state.workers.values());
  }

  getWorker(workerId: string): WorkerInfo | undefined {
    return this.state.workers.get(workerId);
  }

  getActiveWorkerCount(): number {
    return Array.from(this.state.workers.values()).filter(
      w => w.status === 'running' || w.status === 'ready'
    ).length;
  }

  getTotalConnections(): number {
    return Array.from(this.state.workers.values()).reduce(
      (sum, w) => sum + w.connections,
      0
    );
  }

  async sendCommandToWorker(
    workerId: string,
    command: WorkerCommand
  ): Promise<WorkerCommandResponse> {
    const worker = this.state.workers.get(workerId);
    if (!worker) {
      throw new Error(`Worker not found: ${workerId}`);
    }

    try {
      const response = await fetch(`http://${worker.publicIp}:3000/command`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(command),
      });

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      return await response.json();
    } catch (error) {
      return {
        workerId,
        commandType: command.type,
        success: false,
        error: String(error),
        timestamp: Date.now(),
      };
    }
  }

  async broadcastCommand(
    command: WorkerCommand
  ): Promise<WorkerCommandResponse[]> {
    const workers = Array.from(this.state.workers.values());
    const promises = workers.map(w =>
      this.sendCommandToWorker(w.workerId, command)
    );

    return Promise.all(promises);
  }

  checkWorkerHealth(timeoutMs: number = 30000): {
    healthy: string[];
    stale: string[];
    missing: string[];
  } {
    const now = Date.now();
    const healthy: string[] = [];
    const stale: string[] = [];
    const missing: string[] = [];

    for (const [workerId, worker] of this.state.workers) {
      if (!worker.lastHeartbeat) {
        missing.push(workerId);
      } else if (now - worker.lastHeartbeat > timeoutMs) {
        stale.push(workerId);
      } else {
        healthy.push(workerId);
      }
    }

    return { healthy, stale, missing };
  }

  getCoordinatorUrl(): string {
    return `http://localhost:${this.port}`;
  }

  getCurrentPhase(): TestPhase {
    return this.state.currentPhase;
  }

  getState(): CoordinatorState {
    return this.state;
  }
}
