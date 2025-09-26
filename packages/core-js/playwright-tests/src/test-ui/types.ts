// VFS Worker message types - matching the latergram example structure
export type VFSWorkerMessage =
  | { type: 'init'; manifest: ArrayBuffer; wsUrl: string }
  | { type: 'readFile'; id: string; path: string }
  | {
      type: 'writeFile';
      id: string;
      path: string;
      content: string;
      create: boolean;
    }
  | { type: 'deleteFile'; id: string; path: string }
  | { type: 'listDirectory'; id: string; path: string }
  | { type: 'exists'; id: string; path: string }
  | { type: 'watchFile'; id: string; path: string }
  | { type: 'unwatchFile'; id: string };

export type VFSWorkerResponse =
  | { type: 'ready' }
  | { type: 'init'; success: boolean; error?: string }
  | {
      type: 'readFile';
      id: string;
      success: boolean;
      data?: string;
      error?: string;
    }
  | { type: 'writeFile'; id: string; success: boolean; error?: string }
  | { type: 'deleteFile'; id: string; success: boolean; error?: string }
  | {
      type: 'listDirectory';
      id: string;
      success: boolean;
      data?: unknown[];
      error?: string;
    }
  | {
      type: 'exists';
      id: string;
      success: boolean;
      data?: boolean;
      error?: string;
    }
  | { type: 'watchFile'; id: string; success: boolean; error?: string }
  | { type: 'unwatchFile'; id: string; success: boolean; error?: string }
  | { type: 'fileChanged'; watchId: string; content: string };

// Performance metrics types
export interface BenchmarkMetrics {
  testName: string;
  timestamp: number;

  throughput: {
    operationsPerSecond: number;
    bytesPerSecond: number;
    totalOperations: number;
    totalBytes: number;
  };

  latency: {
    min: number;
    max: number;
    mean: number;
    median: number;
    p50: number;
    p95: number;
    p99: number;
  };

  memory: {
    heapUsed: number;
    heapTotal: number;
    wasmMemory: number;
    indexedDBSize: number;
  };

  errors: {
    count: number;
    types: Map<string, number>;
    lastError?: string;
  };
}

// Test image specifications
export interface TestImage {
  name: string;
  data: Uint8Array;
  size: number;
  metadata?: {
    width: number;
    height: number;
    format: string;
  };
}

export interface ImageSpec {
  width: number;
  height: number;
  sizeInMB: number;
  format: 'jpeg' | 'png' | 'webp';
  quality?: number;
}

// Server instance management
export interface ServerInstance {
  process: any;
  port: number;
  wsUrl: string;
  manifestUrl: string;
}

// UI State
export interface UIState {
  connected: boolean;
  operations: number;
  throughput: number;
  errors: number;
  memoryUsage: number;
  files: string[];
  uploadProgress: number;
}
