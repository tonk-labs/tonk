import type { DocumentData, RefNode, JsonValue } from '../../../src/core.js';

export type DocumentContent = {
  content: JsonValue;
  bytes?: string;
};

// Image generator types
export interface ImageSpec {
  width: number;
  height: number;
  sizeInMB: number;
  format: 'jpeg' | 'png' | 'webp';
  quality?: number;
}

export interface TestImage {
  name: string;
  data: Uint8Array;
  size: number;
  metadata: {
    width: number;
    height: number;
    format: string;
  };
}

// Message types for VFS Worker communication
export type VFSWorkerMessage =
  | { type: 'init'; manifest: ArrayBuffer; wsUrl: string }
  | { type: 'readFile'; id: string; path: string }
  | {
      type: 'writeFile';
      id: string;
      path: string;
      content: DocumentContent;
      create: boolean;
    }
  | { type: 'deleteFile'; id: string; path: string }
  | { type: 'listDirectory'; id: string; path: string }
  | { type: 'exists'; id: string; path: string }
  | { type: 'watchFile'; id: string; path: string }
  | { type: 'unwatchFile'; id: string; path: string }
  | { type: 'watchDirectory'; id: string; path: string }
  | { type: 'unwatchDirectory'; id: string; path: string };
export type VFSWorkerResponse =
  | { type: 'init'; success: boolean; error?: string }
  | {
      type: 'readFile';
      id: string;
      success: boolean;
      data?: DocumentData;
      error?: string;
    }
  | { type: 'writeFile'; id: string; success: boolean; error?: string }
  | { type: 'deleteFile'; id: string; success: boolean; error?: string }
  | {
      type: 'listDirectory';
      id: string;
      success: boolean;
      data?: RefNode[];
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
  | { type: 'fileChanged'; watchId: string; documentData: DocumentData }
  | { type: 'watchDirectory'; id: string; success: boolean; error?: string }
  | { type: 'unwatchDirectory'; id: string; success: boolean; error?: string }
  | {
      type: 'directoryChanged';
      watchId: string;
      path: string;
      changeData: any;
    };

// Performance metrics types
export interface MemoryMetrics {
  heapUsed: number;
  heapTotal: number;
  wasmMemory: number;
  indexedDBSize: number;
}

export interface LatencyMetrics {
  min: number;
  max: number;
  mean: number;
  median: number;
  p50: number;
  p95: number;
  p99: number;
}

export interface ThroughputMetrics {
  operationsPerSecond: number;
  bytesPerSecond: number;
  totalOperations: number;
  totalBytes: number;
}

export interface ErrorMetrics {
  count: number;
  types: Map<string, number>;
  lastError?: string;
}

export interface BenchmarkMetrics {
  testName: string;
  timestamp: number;
  throughput: ThroughputMetrics;
  latency: LatencyMetrics;
  memory: MemoryMetrics;
  errors: ErrorMetrics;
}
