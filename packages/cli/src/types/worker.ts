/**
 * Worker specification for Tonk
 *
 * This file defines the standard format for worker definitions in Tonk.
 */

/**
 * Worker status information
 */
export interface WorkerStatus {
  /** Whether the worker is currently active */
  active: boolean;
  /** Timestamp of when the worker was last seen (null if never) */
  lastSeen: number | null;
}

/**
 * Worker health check configuration
 */
export interface WorkerHealthCheck {
  /** Endpoint to ping for health check */
  endpoint: string;
  /** Method to use for health check (GET, POST, etc.) */
  method: string;
  /** Interval in milliseconds between health checks */
  interval: number;
  /** Timeout in milliseconds for health check */
  timeout: number;
}

/**
 * Worker runtime configuration
 */
export interface WorkerRuntimeConfig {
  /** Port the worker runs on */
  port?: number;
  /** Health check configuration */
  healthCheck?: WorkerHealthCheck;
  /** Any other runtime options */
  [key: string]: any;
}

/**
 * Worker process configuration
 */
export interface WorkerProcessConfig {
  /** Main file to execute */
  file?: string;
  /** Arguments to pass to the process */
  args?: string;
  /** Working directory */
  cwd?: string;
  /** Number of instances */
  instances?: number;
  /** Whether to auto-restart on failure */
  autorestart?: boolean;
  /** Whether to watch for file changes */
  watch?: boolean;
  /** Memory limit before restart */
  max_memory_restart?: string;
  /** Environment variables for the process */
  env?: Record<string, string>;
  /** Any other process options */
  [key: string]: any;
}

/**
 * Worker CLI configuration
 */
export interface WorkerCLIConfig {
  /** Script path */
  script?: string;
  /** Command to run */
  command?: string;
  /** Arguments array */
  args?: string[];
  /** Any other CLI options */
  [key: string]: any;
}

/**
 * Worker definition
 */
export interface Worker {
  /** Unique identifier for the worker */
  id: string;
  /** Human-readable name of the worker */
  name: string;
  /** Description of the worker's purpose */
  description: string;
  /** Endpoint URL/path where the worker can be reached */
  endpoint: string;
  /** Protocol used by the worker (http, ws, tcp, etc.) */
  protocol: string;
  /** Current status of the worker */
  status: WorkerStatus;
  /** Environment variables required by the worker */
  env: Record<string, string>;
  /** Additional configuration options for the worker */
  config: {
    /** Runtime configuration */
    runtime?: WorkerRuntimeConfig;
    /** Process configuration */
    process?: WorkerProcessConfig;
    /** CLI configuration */
    cli?: WorkerCLIConfig;
    /** Schema definitions */
    schemas?: {
      documents?: {
        default?: any;
        [key: string]: any;
      };
      [key: string]: any;
    };
    /** Worker version */
    version?: string;
    /** Worker type (e.g., "obsidian", "rag", "custom") */
    type?: string;
    /** Any other configuration options */
    [key: string]: any;
  };
  /** Timestamp of when the worker was created */
  createdAt: number;
  /** Timestamp of when the worker was last updated */
  updatedAt: number;
}

/**
 * Worker registration options
 */
export interface WorkerRegistrationOptions {
  /** Name of the worker */
  name: string;
  /** Description of the worker */
  description?: string;
  /** Endpoint URL/path of the worker */
  endpoint: string;
  /** Protocol used by the worker */
  protocol?: string;
  /** Environment variables in KEY=VALUE format */
  env?: string[];
  /** Path to configuration file */
  config?: string;
  /** Worker type */
  type?: string;
}

/**
 * Worker manager interface
 */
export interface WorkerManager {
  /** Register a new worker */
  register(options: WorkerRegistrationOptions): Promise<Worker>;
  /** Get a worker by ID */
  get(id: string): Promise<Worker | null>;
  /** List all registered workers */
  list(): Promise<Worker[]>;
  /** Update a worker */
  update(id: string, updates: Partial<Worker>): Promise<Worker>;
  /** Remove a worker */
  remove(id: string): Promise<boolean>;
  /** Start a worker */
  start(id: string): Promise<boolean>;
  /** Stop a worker */
  stop(id: string): Promise<boolean>;
  /** Check worker health */
  checkHealth(id: string): Promise<boolean>;
}
