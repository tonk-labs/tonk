import type { ConnectionStats } from '../utils/connection-manager';

export interface ErrorDetail {
  timestamp: number;
  type: string;
  message: string;
  stack?: string;
  context?: {
    connectionId?: string;
    operation?: string;
    workerId?: string;
    phase?: TestPhase;
    [key: string]: any;
  };
}

export interface WorkerConfig {
  workerId: string;
  coordinatorUrl: string;
  relayUrl: string;
  targetConnections: number;
  operationsPerMinute: number;
  rampUpDuration?: number;
  publicIp: string;
}

export interface WorkerInfo {
  workerId: string;
  instanceId: string;
  publicIp: string;
  privateIp: string;
  status:
    | 'provisioning'
    | 'deploying'
    | 'ready'
    | 'running'
    | 'stopping'
    | 'stopped'
    | 'failed';
  registeredAt?: number;
  lastHeartbeat?: number;
  connections: number;
  targetConnections: number;
}

export interface WorkerMetrics {
  workerId: string;
  timestamp: number;
  connections: ConnectionStats;
  operations: {
    completed: number;
    failed: number;
    operationsPerSecond: number;
  };
  memory: {
    heapUsed: number;
    heapTotal: number;
    rss: number;
  };
  latency: {
    min: number;
    max: number;
    mean: number;
    p50: number;
    p95: number;
    p99: number;
  };
  errors: {
    count: number;
    types: { [key: string]: number };
    details: ErrorDetail[];
    lastError?: ErrorDetail;
  };
}

export interface RelayMetrics {
  timestamp: number;
  memory: {
    rss: number;
    total: number;
  };
  connections: number;
  uptime: number;
  process: {
    pid: number;
  };
}

export interface AggregateMetrics {
  timestamp: number;
  relay: RelayMetrics;
  workers: WorkerMetrics[];
  aggregate: {
    totalConnections: number;
    healthyConnections: number;
    totalOperations: number;
    operationsPerSecond: number;
    errorRate: number;
    latency: {
      min: number;
      max: number;
      mean: number;
      p50: number;
      p95: number;
      p99: number;
    };
    memory: {
      relayRss: number;
      totalWorkerRss: number;
      totalRss: number;
    };
  };
}

export type TestPhase =
  | 'provision'
  | 'deploy'
  | 'warmup'
  | 'rampup'
  | 'sustained'
  | 'stress'
  | 'cooldown'
  | 'report'
  | 'teardown'
  | 'complete'
  | 'failed';

export interface PhaseConfig {
  phase: TestPhase;
  durationMs: number;
  targetConnections: number;
  operationsPerMinute: number;
  description: string;
}

export interface TestScenario {
  name: string;
  description: string;
  workerInstanceType: string;
  workerCount: number;
  connectionsPerWorker: number;
  phases: PhaseConfig[];
  relayHost: string;
  relayPort: number;
  useSpotInstances: boolean;
  maxCostPerHour?: number;
}

export interface EC2Config {
  region: string;
  instanceType: string;
  amiId?: string;
  keyName: string;
  securityGroupName: string;
  useSpotInstances: boolean;
  maxSpotPrice?: string;
  instanceTags: { [key: string]: string };
}

export interface ProvisionedInstance {
  instanceId: string;
  publicIp: string;
  privateIp: string;
  publicDns: string;
  status: string;
  launchedAt: number;
}

export interface WorkerRegistration {
  workerId: string;
  publicIp: string;
  privateIp: string;
  capabilities: {
    maxConnections: number;
    memory: number;
    cpus: number;
  };
  timestamp: number;
}

export interface WorkerHeartbeat {
  workerId: string;
  timestamp: number;
  status: 'healthy' | 'degraded' | 'unhealthy';
  activeConnections: number;
  metrics: WorkerMetrics;
}

export interface CoordinatorState {
  currentPhase: TestPhase;
  phaseStartedAt: number;
  workers: Map<string, WorkerInfo>;
  metricsHistory: AggregateMetrics[];
  testStartedAt: number;
  testConfig: TestScenario;
}

export interface TestReport {
  testName: string;
  scenario: string;
  startTime: number;
  endTime: number;
  duration: number;
  status: 'passed' | 'failed' | 'aborted';
  phases: {
    phase: TestPhase;
    startTime: number;
    endTime: number;
    duration: number;
    status: 'completed' | 'failed' | 'skipped';
    metrics?: AggregateMetrics;
  }[];
  summary: {
    totalConnections: number;
    peakConnections: number;
    totalOperations: number;
    totalErrors: number;
    errorRate: number;
    avgLatency: number;
    p95Latency: number;
    p99Latency: number;
    maxLatency: number;
    relayMemoryGrowth: number;
    workerCost: number;
    totalCost: number;
  };
  workerStats: {
    workerId: string;
    connections: number;
    operations: number;
    errors: number;
    avgLatency: number;
  }[];
  failures?: {
    timestamp: number;
    phase: TestPhase;
    error: string;
    context: any;
  }[];
  errorReport?: {
    totalErrors: number;
    errorsByType: { [key: string]: number };
    errorsByWorker: { [workerId: string]: number };
    errorsByPhase: { [phase: string]: number };
    criticalErrors: ErrorDetail[];
    allErrors: Array<{
      timestamp: number;
      workerId?: string;
      phase?: string;
      error: ErrorDetail;
    }>;
  };
  metricsTimeSeries: AggregateMetrics[];
}

export interface WorkerCommand {
  type: 'start' | 'stop' | 'pause' | 'resume' | 'phase_change' | 'shutdown';
  payload?: any;
}

export interface WorkerCommandResponse {
  workerId: string;
  commandType: string;
  success: boolean;
  error?: string;
  timestamp: number;
}

export interface SSHConnection {
  host: string;
  username: string;
  privateKeyPath: string;
  port?: number;
}

export interface DeploymentConfig {
  repoUrl: string;
  branch: string;
  setupScriptPath: string;
  workerScriptPath: string;
  nodeVersion: string;
}
