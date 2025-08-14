import { spawn } from 'child_process';
import path from 'path';
import { fileURLToPath } from 'url';

export interface CLIResult {
  code: number | null;
  stdout: string;
  stderr: string;
  success: boolean;
}

export class CLITestHelper {
  private cliPath: string;

  constructor() {
    // Resolve CLI path relative to this test file
    const currentDir = path.dirname(fileURLToPath(import.meta.url));
    this.cliPath = path.resolve(currentDir, '../../bin/tonk.js');
  }

  async run(
    args: string[],
    options: { env?: Record<string, string>; cwd?: string } = {}
  ): Promise<CLIResult> {
    return new Promise((resolve, reject) => {
      const childProcess = spawn('node', [this.cliPath, ...args], {
        env: { ...process.env, NODE_ENV: 'test', ...options.env },
        cwd: options.cwd || process.cwd(),
        stdio: ['pipe', 'pipe', 'pipe'],
      });

      let stdout = '';
      let stderr = '';

      childProcess.stdout?.on('data', data => {
        stdout += data.toString();
      });

      childProcess.stderr?.on('data', data => {
        stderr += data.toString();
      });

      childProcess.on('close', code => {
        resolve({
          code,
          stdout,
          stderr,
          success: code === 0,
        });
      });

      childProcess.on('error', reject);

      // Set a timeout to prevent hanging tests
      const timeout = setTimeout(() => {
        childProcess.kill('SIGTERM');
        reject(new Error('CLI command timed out'));
      }, 30000);

      childProcess.on('close', () => {
        clearTimeout(timeout);
      });
    });
  }

  async runWithInput(
    args: string[],
    input: string,
    options: { env?: Record<string, string>; cwd?: string } = {}
  ): Promise<CLIResult> {
    return new Promise((resolve, reject) => {
      const childProcess = spawn('node', [this.cliPath, ...args], {
        env: { ...process.env, NODE_ENV: 'test', ...options.env },
        cwd: options.cwd || process.cwd(),
        stdio: ['pipe', 'pipe', 'pipe'],
      });

      let stdout = '';
      let stderr = '';

      childProcess.stdout?.on('data', data => {
        stdout += data.toString();
      });

      childProcess.stderr?.on('data', data => {
        stderr += data.toString();
      });

      // Send input to stdin
      if (childProcess.stdin) {
        childProcess.stdin.write(input);
        childProcess.stdin.end();
      }

      childProcess.on('close', code => {
        resolve({
          code,
          stdout,
          stderr,
          success: code === 0,
        });
      });

      childProcess.on('error', reject);

      // Set a timeout to prevent hanging tests
      const timeout = setTimeout(() => {
        childProcess.kill('SIGTERM');
        reject(new Error('CLI command timed out'));
      }, 30000);

      childProcess.on('close', () => {
        clearTimeout(timeout);
      });
    });
  }
}
