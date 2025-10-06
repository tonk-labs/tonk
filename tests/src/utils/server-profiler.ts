export interface ServerMemorySnapshot {
  timestamp: number;
  connectionCount: number;
  operationCount: number;
  rss: number; // Resident Set Size (total memory)
  heapTotal: number;
  heapUsed: number;
  external: number;
  arrayBuffers: number;
}

export interface ServerMemoryProfile {
  snapshots: ServerMemorySnapshot[];
  testName: string;
  startTime: number;
  endTime?: number;
}

/**
 * Profiles server-side memory usage during benchmark tests
 */
export class ServerProfiler {
  private profiles: Map<string, ServerMemoryProfile> = new Map();
  private activeProfile: ServerMemoryProfile | null = null;
  private monitorInterval: NodeJS.Timeout | null = null;
  private connectionCount: number = 0;
  private operationCount: number = 0;

  /**
   * Start profiling a server instance
   */
  startProfiling(testName: string, intervalMs: number = 1000): void {
    if (this.activeProfile) {
      console.warn('A profile is already active. Stopping previous profile.');
      this.stopProfiling();
    }

    this.activeProfile = {
      snapshots: [],
      testName,
      startTime: Date.now(),
    };

    this.connectionCount = 0;
    this.operationCount = 0;

    // Start periodic memory monitoring
    this.monitorInterval = setInterval(() => {
      this.captureSnapshot();
    }, intervalMs);

    console.log(
      `Started server profiling for "${testName}" (interval: ${intervalMs}ms)`
    );
  }

  /**
   * Stop profiling and save results
   */
  stopProfiling(): ServerMemoryProfile | null {
    if (!this.activeProfile) {
      console.warn('No active profile to stop');
      return null;
    }

    if (this.monitorInterval) {
      clearInterval(this.monitorInterval);
      this.monitorInterval = null;
    }

    // Capture final snapshot
    this.captureSnapshot();

    this.activeProfile.endTime = Date.now();
    this.profiles.set(this.activeProfile.testName, this.activeProfile);

    console.log(
      `Stopped server profiling for "${this.activeProfile.testName}" (${this.activeProfile.snapshots.length} snapshots)`
    );

    const profile = this.activeProfile;
    this.activeProfile = null;

    return profile;
  }

  /**
   * Update connection count
   */
  setConnectionCount(count: number): void {
    this.connectionCount = count;
  }

  /**
   * Increment operation count
   */
  incrementOperations(count: number = 1): void {
    this.operationCount += count;
  }

  /**
   * Capture a memory snapshot from the Node.js process
   */
  private captureSnapshot(): void {
    if (!this.activeProfile) return;

    const memUsage = process.memoryUsage();

    const snapshot: ServerMemorySnapshot = {
      timestamp: Date.now(),
      connectionCount: this.connectionCount,
      operationCount: this.operationCount,
      rss: memUsage.rss,
      heapTotal: memUsage.heapTotal,
      heapUsed: memUsage.heapUsed,
      external: memUsage.external,
      arrayBuffers: memUsage.arrayBuffers,
    };

    this.activeProfile.snapshots.push(snapshot);
  }

  /**
   * Get profile for a specific test
   */
  getProfile(testName: string): ServerMemoryProfile | undefined {
    return this.profiles.get(testName);
  }

  /**
   * Get all profiles
   */
  getAllProfiles(): Map<string, ServerMemoryProfile> {
    return new Map(this.profiles);
  }

  /**
   * Analyze memory usage per connection
   */
  analyzeMemoryPerConnection(profile: ServerMemoryProfile): {
    avgMemoryPerConnection: number;
    memoryGrowthRate: number;
    baselineMemory: number;
  } {
    if (profile.snapshots.length < 2) {
      return {
        avgMemoryPerConnection: 0,
        memoryGrowthRate: 0,
        baselineMemory: 0,
      };
    }

    // Get baseline (first snapshot with 0 connections or minimum connections)
    const baseline = profile.snapshots[0];
    const baselineMemory = baseline.rss;

    // Get snapshots with connections
    const snapshotsWithConnections = profile.snapshots.filter(
      s => s.connectionCount > 0
    );

    if (snapshotsWithConnections.length === 0) {
      return {
        avgMemoryPerConnection: 0,
        memoryGrowthRate: 0,
        baselineMemory,
      };
    }

    // Calculate memory per connection
    const memoryDeltas = snapshotsWithConnections.map(snapshot => ({
      connections: snapshot.connectionCount,
      memoryDelta: snapshot.rss - baselineMemory,
    }));

    // Simple linear regression for memory vs connections
    const avgMemoryPerConnection =
      memoryDeltas.reduce((sum, d) => sum + d.memoryDelta / d.connections, 0) /
      memoryDeltas.length;

    // Calculate growth rate (bytes per second)
    const first = profile.snapshots[0];
    const last = profile.snapshots[profile.snapshots.length - 1];
    const durationSeconds = (last.timestamp - first.timestamp) / 1000;
    const memoryGrowthRate =
      durationSeconds > 0 ? (last.rss - first.rss) / durationSeconds : 0;

    return {
      avgMemoryPerConnection,
      memoryGrowthRate,
      baselineMemory,
    };
  }

  /**
   * Analyze memory usage per operation
   */
  analyzeMemoryPerOperation(profile: ServerMemoryProfile): {
    avgMemoryPerOperation: number;
    peakMemory: number;
    memoryEfficiency: number; // ops per MB
  } {
    if (profile.snapshots.length < 2) {
      return {
        avgMemoryPerOperation: 0,
        peakMemory: 0,
        memoryEfficiency: 0,
      };
    }

    const baseline = profile.snapshots[0];
    const snapshotsWithOps = profile.snapshots.filter(
      s => s.operationCount > 0
    );

    if (snapshotsWithOps.length === 0) {
      return {
        avgMemoryPerOperation: 0,
        peakMemory: Math.max(...profile.snapshots.map(s => s.rss)),
        memoryEfficiency: 0,
      };
    }

    // Calculate memory per operation
    const avgMemoryPerOperation =
      snapshotsWithOps.reduce((sum, s) => {
        const memDelta = s.rss - baseline.rss;
        return sum + memDelta / s.operationCount;
      }, 0) / snapshotsWithOps.length;

    const peakMemory = Math.max(...profile.snapshots.map(s => s.rss));

    const lastSnapshot = profile.snapshots[profile.snapshots.length - 1];
    const totalMemoryUsed = lastSnapshot.rss - baseline.rss;
    const memoryEfficiency =
      totalMemoryUsed > 0
        ? lastSnapshot.operationCount / (totalMemoryUsed / (1024 * 1024))
        : 0;

    return {
      avgMemoryPerOperation,
      peakMemory,
      memoryEfficiency,
    };
  }

  /**
   * Generate a report for a profile
   */
  generateReport(profile: ServerMemoryProfile): string {
    const duration = profile.endTime
      ? (profile.endTime - profile.startTime) / 1000
      : 0;

    const memPerConnection = this.analyzeMemoryPerConnection(profile);
    const memPerOperation = this.analyzeMemoryPerOperation(profile);

    const lastSnapshot = profile.snapshots[profile.snapshots.length - 1];
    const firstSnapshot = profile.snapshots[0];

    const report = `
=== Server Memory Profile: ${profile.testName} ===
Duration: ${duration.toFixed(2)}s
Snapshots: ${profile.snapshots.length}

MEMORY OVERVIEW:
- Baseline Memory: ${(memPerConnection.baselineMemory / (1024 * 1024)).toFixed(2)} MB
- Peak Memory: ${(memPerOperation.peakMemory / (1024 * 1024)).toFixed(2)} MB
- Final RSS: ${(lastSnapshot.rss / (1024 * 1024)).toFixed(2)} MB
- Final Heap Used: ${(lastSnapshot.heapUsed / (1024 * 1024)).toFixed(2)} MB
- Final Heap Total: ${(lastSnapshot.heapTotal / (1024 * 1024)).toFixed(2)} MB

CONNECTION METRICS:
- Max Connections: ${Math.max(...profile.snapshots.map(s => s.connectionCount))}
- Avg Memory Per Connection: ${(memPerConnection.avgMemoryPerConnection / (1024 * 1024)).toFixed(2)} MB
- Memory Growth Rate: ${(memPerConnection.memoryGrowthRate / (1024 * 1024)).toFixed(2)} MB/s

OPERATION METRICS:
- Total Operations: ${lastSnapshot.operationCount}
- Avg Memory Per Operation: ${(memPerOperation.avgMemoryPerOperation / 1024).toFixed(2)} KB
- Memory Efficiency: ${memPerOperation.memoryEfficiency.toFixed(2)} ops/MB

MEMORY LEAK DETECTION:
- Total Memory Growth: ${((lastSnapshot.rss - firstSnapshot.rss) / (1024 * 1024)).toFixed(2)} MB
- Growth Per Second: ${(memPerConnection.memoryGrowthRate / (1024 * 1024)).toFixed(2)} MB/s
${
  memPerConnection.memoryGrowthRate > 1024 * 1024 * 5
    ? '⚠️  WARNING: High memory growth detected (>5 MB/s)'
    : '✓ Memory growth within acceptable range'
}
`;

    return report;
  }

  /**
   * Export profile as JSON
   */
  exportJSON(profile: ServerMemoryProfile): string {
    return JSON.stringify(
      {
        ...profile,
        analysis: {
          memoryPerConnection: this.analyzeMemoryPerConnection(profile),
          memoryPerOperation: this.analyzeMemoryPerOperation(profile),
        },
      },
      null,
      2
    );
  }

  /**
   * Export profile as CSV
   */
  exportCSV(profile: ServerMemoryProfile): string {
    const headers = [
      'Timestamp',
      'Connections',
      'Operations',
      'RSS (MB)',
      'Heap Total (MB)',
      'Heap Used (MB)',
      'External (MB)',
      'Array Buffers (MB)',
    ];

    const rows = profile.snapshots.map(s => [
      s.timestamp,
      s.connectionCount,
      s.operationCount,
      (s.rss / (1024 * 1024)).toFixed(2),
      (s.heapTotal / (1024 * 1024)).toFixed(2),
      (s.heapUsed / (1024 * 1024)).toFixed(2),
      (s.external / (1024 * 1024)).toFixed(2),
      (s.arrayBuffers / (1024 * 1024)).toFixed(2),
    ]);

    return headers.join(',') + '\n' + rows.map(row => row.join(',')).join('\n');
  }

  /**
   * Clear all profiles
   */
  clearProfiles(): void {
    this.profiles.clear();
    console.log('All server profiles cleared');
  }
}

// Export singleton instance
export const serverProfiler = new ServerProfiler();
