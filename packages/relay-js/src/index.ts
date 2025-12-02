import { spawn, type ChildProcess } from 'node:child_process';
import { getBinaryPath } from './binary.js';

export { getBinaryPath };

/**
 * Options for starting the relay server.
 */
export interface RelayOptions {
  /** Port to listen on (default: 8081) */
  port?: number;
  /** Path to the .tonk bundle file (required) */
  bundlePath: string;
  /** Path to store automerge data (default: "automerge-repo-data") */
  storagePath?: string;
  /** Host to bind to (default: "127.0.0.1") */
  host?: string;
  /** S3 bucket name for bundle storage */
  s3BucketName?: string;
  /** AWS region for S3 */
  awsRegion?: string;
  /** Whether to inherit stdio from parent process (default: true) */
  stdio?: 'inherit' | 'pipe' | 'ignore';
}

/**
 * Start the Tonk relay server.
 *
 * @param options - Configuration options for the relay
 * @returns The spawned child process
 *
 * @example
 * ```typescript
 * import { startRelay } from '@tonk/relay';
 *
 * const relay = startRelay({
 *   port: 8081,
 *   bundlePath: './my-app.tonk',
 * });
 *
 * relay.on('exit', (code) => {
 *   console.log('Relay exited with code', code);
 * });
 *
 * // To stop the relay:
 * relay.kill();
 * ```
 */
export function startRelay(options: RelayOptions): ChildProcess {
  const binary = getBinaryPath();

  const args = [
    String(options.port ?? 8081),
    options.bundlePath,
  ];

  if (options.storagePath) {
    args.push(options.storagePath);
  }

  const env: NodeJS.ProcessEnv = { ...process.env };
  if (options.host) env.HOST = options.host;
  if (options.s3BucketName) env.S3_BUCKET_NAME = options.s3BucketName;
  if (options.awsRegion) env.AWS_REGION = options.awsRegion;

  return spawn(binary, args, {
    env,
    stdio: options.stdio ?? 'inherit',
  });
}

/**
 * Start the relay and return a promise that resolves when it exits.
 *
 * @param options - Configuration options for the relay
 * @returns Promise that resolves with the exit code
 */
export function runRelay(options: RelayOptions): Promise<number> {
  return new Promise((resolve, reject) => {
    const child = startRelay(options);

    child.on('error', reject);
    child.on('exit', (code) => {
      resolve(code ?? 0);
    });
  });
}
