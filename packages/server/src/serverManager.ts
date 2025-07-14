import {spawn} from 'node:child_process';
import {PortAllocator} from './portAllocator.js';
import chalk from 'chalk';

import type {ChildProcess} from 'node:child_process';

interface ServerConfig {
  bundleName: string;
  serverPath: string;
  port: number;
}

interface ServerProcess {
  process: ChildProcess;
  port: number;
  startTime: Date;
}

export class ServerManager {
  private runningServers: Map<string, ServerProcess> = new Map();
  private portAllocator: PortAllocator;

  constructor() {
    this.portAllocator = new PortAllocator();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    console.log(chalk[color](message));
  }

  /**
   * Start a custom server process for a bundle
   */
  async startServer(config: ServerConfig): Promise<void> {
    const {bundleName, serverPath, port} = config;

    // Check if server is already running
    if (this.runningServers.has(bundleName)) {
      this.log(
        'yellow',
        `Server for bundle "${bundleName}" is already running`,
      );
      return;
    }

    this.log(
      'blue',
      `Starting server for bundle "${bundleName}" on port ${port}`,
    );

    // Start the Node.js server process
    const serverProcess = spawn('node', ['dist/index.js'], {
      cwd: serverPath,
      env: {
        ...process.env,
        PORT: port.toString(),
        BUNDLE_NAME: bundleName,
      },
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    // Handle process output
    serverProcess.stdout?.on('data', data => {
      this.log('blue', `[${bundleName}] ${data.toString().trim()}`);
    });

    serverProcess.stderr?.on('data', data => {
      this.log('red', `[${bundleName}] ${data.toString().trim()}`);
    });

    // Handle process exit
    serverProcess.on('exit', async (code, signal) => {
      this.log(
        'yellow',
        `[${bundleName}] Server process exited with code ${code}, signal ${signal}`,
      );
      const serverProcess = this.runningServers.get(bundleName);
      if (serverProcess) {
        await this.portAllocator.deallocate(serverProcess.port);
      }
      this.runningServers.delete(bundleName);
    });

    // Handle process error
    serverProcess.on('error', async error => {
      this.log('red', `[${bundleName}] Server process error: ${error.message}`);
      const serverProcess = this.runningServers.get(bundleName);
      if (serverProcess) {
        await this.portAllocator.deallocate(serverProcess.port);
      }
      this.runningServers.delete(bundleName);
    });

    // Track the running server
    this.runningServers.set(bundleName, {
      process: serverProcess,
      port,
      startTime: new Date(),
    });

    // Wait for server to start
    try {
      await this.waitForServer(port, bundleName);
      this.log(
        'green',
        `✅ Server for bundle "${bundleName}" started successfully on port ${port}`,
      );
    } catch (error) {
      this.log(
        'red',
        `❌ Failed to start server for bundle "${bundleName}": ${error}`,
      );
      const serverProcess = this.runningServers.get(bundleName);
      if (serverProcess) {
        await this.portAllocator.deallocate(serverProcess.port);
      }
      throw error;
    }
  }

  /**
   * Stop a custom server process
   */
  async stopServer(bundleName: string): Promise<void> {
    const serverProcess = this.runningServers.get(bundleName);
    if (!serverProcess) {
      this.log('yellow', `No server running for bundle "${bundleName}"`);
      return;
    }

    this.log('blue', `Stopping server for bundle "${bundleName}"`);

    // Gracefully terminate the process
    serverProcess.process.kill('SIGTERM');

    // Wait for graceful shutdown, then force kill if needed
    const killTimeout = setTimeout(() => {
      if (!serverProcess.process.killed) {
        this.log('yellow', `Force killing server for bundle "${bundleName}"`);
        serverProcess.process.kill('SIGKILL');
      }
    }, 5000);

    // Clean up when process exits
    serverProcess.process.on('exit', async () => {
      clearTimeout(killTimeout);
      this.runningServers.delete(bundleName);
      await this.portAllocator.deallocate(serverProcess.port);
      this.log('green', `✅ Server for bundle "${bundleName}" stopped`);
    });
  }

  /**
   * Get status of all running servers
   */
  getRunningServers(): Map<string, ServerProcess> {
    return new Map(this.runningServers);
  }

  /**
   * Check if a server is running for a bundle
   */
  isServerRunning(bundleName: string): boolean {
    return this.runningServers.has(bundleName);
  }

  /**
   * Stop all running servers
   */
  async stopAllServers(): Promise<void> {
    const stopPromises = Array.from(this.runningServers.keys()).map(
      bundleName => this.stopServer(bundleName),
    );
    await Promise.all(stopPromises);
  }

  /**
   * Wait for server to be ready by checking health endpoint
   */
  private async waitForServer(port: number, bundleName: string): Promise<void> {
    const maxAttempts = 30;
    const delay = 1000;

    for (let attempt = 0; attempt < maxAttempts; attempt++) {
      try {
        const response = await fetch(`http://localhost:${port}/ping`);
        if (response.ok) {
          return;
        }
      } catch (error) {
        // Server not ready yet, wait and retry
        this.log(
          'yellow',
          `[${bundleName}] Waiting for server to be ready... (attempt ${attempt + 1}/${maxAttempts})`,
        );
      }

      await new Promise(resolve => setTimeout(resolve, delay));
    }

    throw new Error(
      `Server failed to start on port ${port} after ${maxAttempts} attempts`,
    );
  }
}
