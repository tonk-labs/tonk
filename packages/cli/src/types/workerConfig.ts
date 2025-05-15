/**
 * Worker YAML Configuration Schema
 *
 * This file defines the standard format for worker configuration YAML files in Tonk.
 */
import {z} from 'zod';

/**
 * Zod schema for worker health check configuration
 */
export const WorkerHealthCheckSchema = z.object({
  endpoint: z.string().optional(),
  method: z.string().default('GET'),
  interval: z.number().default(30000),
  timeout: z.number().default(5000),
});

/**
 * Zod schema for worker configuration
 */
export const WorkerConfigSchema = z.object({
  // Basic worker information
  name: z.string(),
  description: z.string().optional(),
  version: z.string().optional(),

  // Connection details
  endpoint: z.string(),
  protocol: z.string().default('http'),

  // Dependencies
  dependencies: z.array(z.string()).optional(),

  // Health check configuration
  healthCheck: WorkerHealthCheckSchema.optional(),

  // Process management
  process: z.object({
    script: z.string(),
    cwd: z.string().optional(),
    instances: z.number().default(1),
    autorestart: z.boolean().default(true),
    watch: z.boolean().default(false),
    max_memory_restart: z.string().optional(),
    env: z.record(z.string()).optional(),
  }),

  // Additional custom configuration
  config: z.record(z.any()).optional(),
});

/**
 * Type for worker configuration
 */
export type WorkerConfig = z.infer<typeof WorkerConfigSchema>;

/**
 * Example YAML configuration:
 *
 * ```yaml
 * # Worker Information
 * name: obsidian-worker
 * description: Tonk worker for Obsidian integration
 * version: 1.0.0
 *
 * # Connection Details
 * endpoint: http://localhost:5555/tonk
 * protocol: http
 *
 * # Dependencies
 * # List of worker names that this worker depends on
 * dependencies:
 *   - vector-store-worker
 *   - embedding-worker
 *
 * # Health Check Configuration
 * healthCheck:
 *   endpoint: http://localhost:5555/health
 *   method: GET
 *   interval: 30000
 *   timeout: 5000
 *
 * # Process Management
 * process:
 *   script: ./dist/index.js
 *   cwd: ./
 *   instances: 1
 *   autorestart: true
 *   watch: false
 *   max_memory_restart: 500M
 *   env:
 *     NODE_ENV: production
 *     CHROMA_URL: http://localhost:8888
 *
 * # Additional Configuration
 * config:
 *   syncDirectory: notes
 *   embedModel: all-MiniLM-L6-v2
 * ```
 */
